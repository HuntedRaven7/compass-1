//! Bounded, read-only size diagnostics for the file-index cache.
//!
//! This deliberately bypasses the normal open helpers: they change pragmas,
//! create missing databases, or rebuild an unsupported schema. A read-only
//! SQLite transaction includes committed WAL pages; `immutable=1` would not.
//! Existing WAL shared-memory reader bookkeeping is allowed, but no schema,
//! rows, journal mode, checkpoint, or compaction is changed here.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use compass_sqlcipher_sys::rusqlite::{self, Connection, ErrorCode, OpenFlags};

use crate::sqlite_writer::FILE_INDEX_SCHEMA_VERSION;

const QUERY_BUDGET: Duration = Duration::from_secs(2);
const LOCK_BUDGET: Duration = Duration::from_millis(100);

/// A measurement, including the distinction between absence and failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measurement {
    /// An observed byte or row/page count, including a genuine zero.
    Value(u64),
    /// The file does not exist. This is not a measured zero.
    Missing,
    /// The measurement could not be obtained, with a reason safe to print.
    Unavailable(String),
}

/// File sizes and SQLite measurements; no indexed paths or file contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStats {
    /// Main database file's logical length in bytes, not allocated disk blocks.
    pub database_bytes: Measurement,
    /// Write-ahead log's logical length, including reusable frames.
    pub wal_bytes: Measurement,
    /// Shared-memory file's logical length.
    pub shm_bytes: Measurement,
    /// SQLite page size in bytes.
    pub page_size: Measurement,
    /// Pages in the SQLite read snapshot, including committed WAL pages.
    pub page_count: Measurement,
    /// Unused pages available for SQLite to reuse within that snapshot.
    pub freelist_count: Measurement,
    /// Rows in `indexed_file`, including directories.
    pub indexed_rows: Measurement,
    /// Bytes of pages belonging to the two FTS indexes' shadow tables.
    /// Excludes the external content table and WAL overhead.
    pub fts_bytes: Measurement,
}

impl IndexStats {
    /// Whether all required measurements were available. FTS is optional
    /// because SQLite builds without `dbstat` cannot report its footprint.
    #[must_use]
    pub fn complete(&self) -> bool {
        [
            &self.database_bytes,
            &self.page_size,
            &self.page_count,
            &self.freelist_count,
            &self.indexed_rows,
        ]
        .iter()
        .all(|value| matches!(value, Measurement::Value(_)))
            && !matches!(self.wal_bytes, Measurement::Unavailable(_))
            && !matches!(self.shm_bytes, Measurement::Unavailable(_))
    }

    /// Bytes of the pages that hold data in the read snapshot: every page
    /// but the free ones, so rows, their indexes and the FTS indexes
    /// ([`Self::fts_bytes`] is part of it). `None` unless the page size,
    /// page count and freelist were all measured.
    #[must_use]
    pub fn live_bytes(&self) -> Option<u64> {
        let (size, count, free) = self.pages()?;
        count.checked_sub(free)?.checked_mul(size)
    }

    /// Bytes of the free pages: space the file holds that SQLite reuses
    /// before growing it, and that only a `VACUUM` gives back. `None` on the
    /// terms of [`Self::live_bytes`].
    #[must_use]
    pub fn reclaimable_bytes(&self) -> Option<u64> {
        let (size, _, free) = self.pages()?;
        free.checked_mul(size)
    }

    fn pages(&self) -> Option<(u64, u64, u64)> {
        match (&self.page_size, &self.page_count, &self.freelist_count) {
            (Measurement::Value(size), Measurement::Value(count), Measurement::Value(free)) => {
                Some((*size, *count, *free))
            }
            _ => None,
        }
    }
}

/// Measures only an existing file-index database. Missing or unreadable
/// metrics stay explicitly unavailable. A missing main database or parent
/// directory is never created.
///
/// SQL runs in one read transaction with a two-second query budget and a
/// 100 ms lock wait. Filesystem lengths are separate observations and may
/// change during a live scan; they are not an atomic snapshot with SQLite.
/// WAL databases without both sidecars are left unopened to avoid creating
/// them. If a concurrent writer removes sidecars after this preflight,
/// SQLite may recreate them for reader coordination.
#[must_use]
pub fn inspect(path: &Path) -> IndexStats {
    let wal = sidecar(path, "-wal");
    let shm = sidecar(path, "-shm");
    let unavailable = Measurement::Unavailable("database measurements unavailable".into());
    let mut stats = IndexStats {
        database_bytes: file_bytes(path),
        wal_bytes: file_bytes(&wal),
        shm_bytes: file_bytes(&shm),
        page_size: unavailable.clone(),
        page_count: unavailable.clone(),
        freelist_count: unavailable.clone(),
        indexed_rows: unavailable.clone(),
        fts_bytes: unavailable,
    };
    if let Err(reason) = inspect_sql(path, &mut stats, QUERY_BUDGET) {
        let unavailable = Measurement::Unavailable(reason);
        stats.page_size = unavailable.clone();
        stats.page_count = unavailable.clone();
        stats.freelist_count = unavailable.clone();
        stats.indexed_rows = unavailable.clone();
        stats.fts_bytes = unavailable;
    }
    stats
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn file_bytes(path: &Path) -> Measurement {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Measurement::Value(metadata.len()),
        Ok(_) => Measurement::Unavailable("not a regular file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Measurement::Missing,
        Err(error) => Measurement::Unavailable(error.to_string()),
    }
}

fn inspect_sql(path: &Path, stats: &mut IndexStats, budget: Duration) -> Result<(), String> {
    match &stats.database_bytes {
        Measurement::Missing => return Err("file index has not been created".into()),
        Measurement::Unavailable(reason) => return Err(reason.clone()),
        Measurement::Value(_) => {}
    }
    let mut header = [0_u8; 20];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| format!("cannot read database header: {error}"))?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err("not an unencrypted SQLite file-index database".into());
    }
    if (header[18] == 2 || header[19] == 2)
        && (!matches!(stats.wal_bytes, Measurement::Value(_))
            || !matches!(stats.shm_bytes, Measurement::Value(_)))
    {
        return Err(
            "WAL sidecars are missing or unreadable; SQL measurements were skipped to avoid \
             creating them. Retry while the file indexer is running."
                .into(),
        );
    }

    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(database_error)?;
    db.busy_timeout(LOCK_BUDGET).map_err(database_error)?;
    with_query_budget(&db, budget, |deadline| read_snapshot(&db, stats, deadline))
}

fn with_query_budget<T>(db: &Connection, budget: Duration, read: impl FnOnce(Instant) -> T) -> T {
    // A timer interrupts long count/dbstat queries without enabling another
    // rusqlite feature or leaving a background thread after the probe ends.
    let interrupt = db.get_interrupt_handle();
    let deadline = Instant::now() + budget;
    let (finished, wait) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            if matches!(
                wait.recv_timeout(budget),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                loop {
                    interrupt.interrupt();
                    if !matches!(
                        wait.recv_timeout(Duration::from_millis(10)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            }
        });
        let result = read(deadline);
        let _ = finished.send(());
        result
    })
}

fn read_snapshot(db: &Connection, stats: &mut IndexStats, deadline: Instant) -> Result<(), String> {
    let transaction = db.unchecked_transaction().map_err(database_error)?;
    stats.page_size = number(&transaction, "PRAGMA page_size", deadline);
    stats.page_count = number(&transaction, "PRAGMA page_count", deadline);
    stats.freelist_count = number(&transaction, "PRAGMA freelist_count", deadline);
    let version = number(&transaction, "PRAGMA user_version", deadline);
    if version != Measurement::Value(FILE_INDEX_SCHEMA_VERSION as u64) {
        let reason = match version {
            Measurement::Value(version) => format!(
                "unsupported file-index schema version {version}; \
                 expected {FILE_INDEX_SCHEMA_VERSION}"
            ),
            Measurement::Unavailable(reason) => reason,
            Measurement::Missing => "file-index schema version unavailable".into(),
        };
        stats.indexed_rows = Measurement::Unavailable(reason.clone());
        stats.fts_bytes = Measurement::Unavailable(reason);
        return Ok(());
    }
    let table = number(
        &transaction,
        "SELECT count(*) FROM sqlite_schema WHERE name = 'indexed_file' \
         AND type = 'table' AND sql LIKE 'CREATE TABLE%'",
        deadline,
    );
    if table != Measurement::Value(1) {
        let reason = match table {
            Measurement::Unavailable(reason) => reason,
            _ => "unsupported file-index schema: indexed_file table missing".into(),
        };
        stats.indexed_rows = Measurement::Unavailable(reason.clone());
        stats.fts_bytes = Measurement::Unavailable(reason);
        return Ok(());
    }
    stats.indexed_rows = number(&transaction, "SELECT count(*) FROM indexed_file", deadline);
    stats.fts_bytes = fts_bytes(&transaction, deadline);
    Ok(())
}

fn fts_bytes(db: &Connection, deadline: Instant) -> Measurement {
    let tables = number(
        db,
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' \
         AND name IN ('path_idx', 'skeleton_idx') AND sql LIKE '%USING fts5%'",
        deadline,
    );
    if tables != Measurement::Value(2) {
        return match tables {
            Measurement::Unavailable(reason) => Measurement::Unavailable(reason),
            _ => Measurement::Unavailable("expected FTS5 indexes are missing".into()),
        };
    }
    number(
        db,
        "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name IN \
         ('path_idx_data', 'path_idx_idx', 'path_idx_docsize', 'path_idx_config', \
          'skeleton_idx_data', 'skeleton_idx_idx', 'skeleton_idx_docsize', 'skeleton_idx_config')",
        deadline,
    )
}

fn number(db: &Connection, sql: &str, deadline: Instant) -> Measurement {
    if Instant::now() >= deadline {
        return Measurement::Unavailable(
            "diagnostic query time limit reached; retry when idle".into(),
        );
    }
    match db.query_row(sql, [], |row| row.get::<_, i64>(0)) {
        Ok(value) => match u64::try_from(value) {
            Ok(value) => Measurement::Value(value),
            Err(_) => Measurement::Unavailable("database returned a negative measurement".into()),
        },
        Err(error) => Measurement::Unavailable(database_error(error)),
    }
}

fn database_error(error: rusqlite::Error) -> String {
    match &error {
        rusqlite::Error::SqliteFailure(code, _) => match code.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                "database is busy; retry when the indexer is idle".into()
            }
            ErrorCode::OperationInterrupted => {
                "diagnostic query time limit reached; retry when idle".into()
            }
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                "database is corrupt or is not a supported file-index database".into()
            }
            ErrorCode::PermissionDenied | ErrorCode::CannotOpen | ErrorCode::ReadOnly => {
                "database or its sidecars cannot be read; check permissions and retry".into()
            }
            _ if error.to_string().contains("no such table: dbstat") => {
                "dbstat is unavailable in this SQLite build".into()
            }
            _ => error.to_string(),
        },
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite_writer::ensure_file_index_schema;

    fn fixture(path: &Path, wal: bool) -> Connection {
        let db = compass_sqlcipher_sys::open(path, &[]).expect("temporary fixture");
        if !wal {
            db.execute_batch("PRAGMA journal_mode = DELETE").unwrap();
        }
        ensure_file_index_schema(&db).unwrap();
        db.execute_batch(
            "INSERT INTO indexed_file(path, skeleton_path) VALUES \
             ('/private/fixture/document.txt', 'document txt'), ('/private/fixture', 'fixture')",
        )
        .unwrap();
        db
    }

    fn directory_entries(path: &Path) -> Vec<std::ffi::OsString> {
        let mut entries: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        entries
    }

    #[test]
    fn missing_database_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("index.db");
        let stats = inspect(&path);
        assert_eq!(stats.database_bytes, Measurement::Missing);
        assert_eq!(stats.wal_bytes, Measurement::Missing);
        assert!(!stats.complete());
        assert!(directory_entries(dir.path()).is_empty());
    }

    #[test]
    fn measures_without_changing_database_or_creating_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        drop(fixture(&path, false));
        let before = fs::read(&path).unwrap();
        let entries = directory_entries(dir.path());
        let stats = inspect(&path);
        assert!(stats.complete(), "{stats:?}");
        assert_eq!(
            stats.database_bytes,
            Measurement::Value(before.len() as u64)
        );
        assert_eq!(stats.indexed_rows, Measurement::Value(2));
        assert_eq!(stats.wal_bytes, Measurement::Missing);
        assert_eq!(stats.shm_bytes, Measurement::Missing);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(directory_entries(dir.path()), entries);
        assert!(!format!("{stats:?}").contains("/private/fixture"));
    }

    #[test]
    fn includes_committed_wal_without_checkpointing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let _writer = fixture(&path, true);
        let main_before = fs::read(&path).unwrap();
        let wal_before = fs::read(sidecar(&path, "-wal")).unwrap();
        let entries = directory_entries(dir.path());
        let stats = inspect(&path);
        assert!(stats.complete(), "{stats:?}");
        assert_eq!(stats.indexed_rows, Measurement::Value(2));
        assert_eq!(stats.wal_bytes, Measurement::Value(wal_before.len() as u64));
        assert_eq!(fs::read(&path).unwrap(), main_before);
        assert_eq!(fs::read(sidecar(&path, "-wal")).unwrap(), wal_before);
        assert_eq!(directory_entries(dir.path()), entries);
    }

    #[test]
    fn absent_wal_sidecars_are_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        drop(fixture(&path, true));
        let entries = directory_entries(dir.path());
        let before = fs::read(&path).unwrap();
        let stats = inspect(&path);
        assert_eq!(stats.wal_bytes, Measurement::Missing);
        assert!(!stats.complete());
        assert_eq!(directory_entries(dir.path()), entries);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn unsupported_schema_is_reported_without_rebuilding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let db = fixture(&path, false);
        db.pragma_update(None, "user_version", 999).unwrap();
        drop(db);
        let before = fs::read(&path).unwrap();
        let stats = inspect(&path);
        assert!(matches!(stats.page_count, Measurement::Value(_)));
        assert!(matches!(stats.indexed_rows, Measurement::Unavailable(_)));
        assert!(!stats.complete());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn invalid_database_is_reported_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let content = b"this is not a valid sqlite database";
        fs::write(&path, content).unwrap();
        let stats = inspect(&path);
        assert_eq!(
            stats.database_bytes,
            Measurement::Value(content.len() as u64)
        );
        assert!(!stats.complete());
        assert_eq!(fs::read(&path).unwrap(), content);
    }

    #[test]
    fn busy_database_keeps_file_sizes_and_reports_unavailable_sql() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let writer = fixture(&path, false);
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let stats = inspect(&path);
        assert!(matches!(stats.database_bytes, Measurement::Value(_)));
        assert!(matches!(stats.page_count, Measurement::Unavailable(_)));
        assert!(!stats.complete());
    }

    #[test]
    fn negative_sql_measurements_are_unavailable_but_zero_is_valid() {
        let db = Connection::open_in_memory().unwrap();
        let deadline = Instant::now() + QUERY_BUDGET;
        assert!(matches!(
            number(&db, "SELECT -1", deadline),
            Measurement::Unavailable(reason) if reason.contains("negative")
        ));
        assert_eq!(number(&db, "SELECT 0", deadline), Measurement::Value(0));
    }

    #[test]
    fn expired_budget_does_not_run_count_or_dbstat() {
        let db = Connection::open_in_memory().unwrap();
        let value = number(&db, "SELECT 1", Instant::now());
        assert!(matches!(value, Measurement::Unavailable(reason) if reason.contains("time limit")));
    }

    #[test]
    fn deleted_rows_show_as_reclaimable_not_live() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let db = fixture(&path, false);
        db.execute_batch(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 2000) \
             INSERT INTO indexed_file(path, skeleton_path) \
             SELECT '/fixture/' || x || '/a-rather-long-file-name.txt', 'fixture ' || x FROM n",
        )
        .unwrap();
        let before = inspect(&path);
        assert_eq!(before.reclaimable_bytes(), Some(0), "{before:?}");
        db.execute_batch("DELETE FROM indexed_file WHERE path LIKE '/fixture/%'")
            .unwrap();
        drop(db);
        let stats = inspect(&path);
        let (Measurement::Value(size), Measurement::Value(count), Measurement::Value(free)) =
            (&stats.page_size, &stats.page_count, &stats.freelist_count)
        else {
            panic!("pages were not measured: {stats:?}");
        };
        assert!(*free > 0, "deleting rows frees pages");
        assert_eq!(stats.reclaimable_bytes(), Some(free * size));
        assert_eq!(
            stats.live_bytes().unwrap() + stats.reclaimable_bytes().unwrap(),
            count * size
        );
        assert!(stats.live_bytes() < before.live_bytes());
        assert_eq!(stats.indexed_rows, Measurement::Value(2));
    }

    #[test]
    fn no_breakdown_without_page_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let stats = inspect(&dir.path().join("absent.db"));
        assert_eq!(stats.live_bytes(), None);
        assert_eq!(stats.reclaimable_bytes(), None);
    }

    #[test]
    fn running_query_is_interrupted_when_budget_expires() {
        let db = Connection::open_in_memory().unwrap();
        let value = with_query_budget(&db, Duration::from_millis(1), |deadline| {
            number(
                &db,
                "WITH RECURSIVE numbers(x) AS (SELECT 1 UNION ALL \
                 SELECT x + 1 FROM numbers WHERE x < 100000000) SELECT sum(x) FROM numbers",
                deadline,
            )
        });
        assert!(matches!(value, Measurement::Unavailable(reason) if reason.contains("time limit")));
    }

    #[test]
    fn corruption_after_the_valid_header_is_reported_without_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        drop(fixture(&path, false));
        let mut content = fs::read(&path).unwrap();
        content[100] = 0; // An invalid b-tree page type, with the SQLite header intact.
        fs::write(&path, &content).unwrap();
        let stats = inspect(&path);
        assert!(!stats.complete());
        assert_eq!(fs::read(&path).unwrap(), content);
    }

    #[test]
    fn permissions_and_non_files_are_not_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let stats = inspect(dir.path());
        assert!(matches!(stats.database_bytes, Measurement::Unavailable(_)));
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_PERM),
            None,
        );
        assert!(database_error(error).contains("permissions"));
    }

    #[test]
    fn dbstat_is_explicitly_optional() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        let db = fixture(&path, false);
        let expected = db.query_row(
            "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name GLOB 'path_idx_*' \
             OR name GLOB 'skeleton_idx_*'",
            [],
            |row| row.get::<_, i64>(0),
        );
        let stats = inspect(&path);
        match expected {
            Ok(bytes) => {
                let bytes = u64::try_from(bytes).expect("dbstat reports nonnegative bytes");
                assert!(bytes > 0);
                assert_eq!(stats.fts_bytes, Measurement::Value(bytes));
            }
            Err(error) => assert_eq!(
                stats.fts_bytes,
                Measurement::Unavailable(database_error(error))
            ),
        }
    }
}
