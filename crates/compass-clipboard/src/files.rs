//! The history's files on disk: measuring them, reading the history without
//! changing it, and deleting or setting it aside for `compass clipboard reset`.
//!
//! The history is a SQLCipher database, its `-wal` and `-shm` sidecars, and a
//! directory of encrypted payloads, all in one data directory. Diagnostics read
//! them and must never create, migrate or checkpoint anything, so they go
//! through [`compass_sqlcipher_sys::open_read_only`] rather than the store's own
//! open, which creates the file and switches it to WAL.
//!
//! A database the key in the keyring does not open is not deleted by a reset:
//! that key may have been lost with a keyring that comes back, so its history is
//! moved into a backup directory beside it instead.

use std::io;
use std::path::{Path, PathBuf};

use compass_crypto::KEY_SIZE;
use compass_sqlcipher_sys::rusqlite::{self, ErrorCode};

/// The history database's file name, in the data directory.
pub const DATABASE_FILE_NAME: &str = "compass-clipboard.db";

/// Directory, beside the database, holding encrypted payloads.
pub const PAYLOAD_DIR_NAME: &str = "compass-clipboard";

/// What a backup directory's name starts with; a Unix time follows.
pub const BACKUP_DIR_PREFIX: &str = "compass-clipboard-backup-";

/// One piece of the history on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// The SQLCipher database.
    Database,
    /// Its write-ahead log, present while a writer has it open or after a
    /// crash.
    WriteAheadLog,
    /// Its WAL index, present alongside the log.
    SharedMemory,
    /// The directory of encrypted payloads.
    Payloads,
}

impl PartKind {
    /// Every part, in the order a reset removes them: the sidecars before the
    /// database, so a log is never left to be replayed into a new database
    /// created later under the same name.
    pub const RESET_ORDER: [Self; 4] = [
        Self::WriteAheadLog,
        Self::SharedMemory,
        Self::Database,
        Self::Payloads,
    ];

    /// The part's file name in the data directory.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Database => DATABASE_FILE_NAME,
            Self::WriteAheadLog => "compass-clipboard.db-wal",
            Self::SharedMemory => "compass-clipboard.db-shm",
            Self::Payloads => PAYLOAD_DIR_NAME,
        }
    }
}

/// A part that exists, and how large it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Which part.
    pub kind: PartKind,
    /// Where it is.
    pub path: PathBuf,
    /// Whether it is a real directory (not a link to one).
    pub directory: bool,
    /// Its logical size in bytes; for the payload directory, the sum of the
    /// files directly in it.
    pub bytes: u64,
    /// For the payload directory, how many entries it holds; 1 otherwise.
    pub files: u64,
}

/// The parts of the history found in a data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Survey {
    /// The data directory surveyed.
    pub dir: PathBuf,
    /// The parts that exist, in [`PartKind::RESET_ORDER`].
    pub parts: Vec<Part>,
}

impl Survey {
    /// The part of `kind`, if it exists.
    #[must_use]
    pub fn part(&self, kind: PartKind) -> Option<&Part> {
        self.parts.iter().find(|part| part.kind == kind)
    }

    /// Whether no part of the history exists.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Every part's bytes added up.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.parts.iter().map(|part| part.bytes).sum()
    }

    /// Whether a sidecar exists, which means a writer has the database open
    /// or exited without checkpointing; either way it must be read through
    /// SQLite's locking rather than as an immutable file.
    #[must_use]
    pub fn has_sidecars(&self) -> bool {
        self.part(PartKind::WriteAheadLog).is_some() || self.part(PartKind::SharedMemory).is_some()
    }
}

/// Lists the parts of the history in `dir` with their sizes. A missing
/// directory is an empty survey.
///
/// # Errors
///
/// When a part exists but cannot be inspected.
pub fn survey(dir: &Path) -> io::Result<Survey> {
    let mut parts = Vec::new();
    for kind in PartKind::RESET_ORDER {
        let path = dir.join(kind.file_name());
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        let directory = metadata.is_dir();
        let (bytes, files) = if directory {
            payload_usage(&path)?
        } else {
            (metadata.len(), 1)
        };
        parts.push(Part {
            kind,
            path,
            directory,
            bytes,
            files,
        });
    }
    Ok(Survey {
        dir: dir.to_path_buf(),
        parts,
    })
}

/// The bytes and entries directly in the payload directory, which is flat:
/// one file per offer.
fn payload_usage(dir: &Path) -> io::Result<(u64, u64)> {
    let mut bytes = 0;
    let mut files = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        bytes += entry.metadata()?.len();
        files += 1;
    }
    Ok((bytes, files))
}

/// Why the history could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReadError {
    /// There is no database.
    #[error("there is no clipboard history database")]
    Missing,
    /// SQLCipher refused the key. SQLCipher cannot tell a wrong key from a
    /// damaged file: both are "not a database".
    #[error("the key does not open the clipboard history, or the file is damaged")]
    Unreadable,
    /// A writer held a lock for longer than the busy timeout.
    #[error("the clipboard history is locked by another process")]
    Busy,
    /// The key opens the file but it holds no history table.
    #[error("the clipboard history database has no history table")]
    NotInitialised,
    /// Anything else SQLite reported.
    #[error("{0}")]
    Other(String),
}

impl From<rusqlite::Error> for ReadError {
    fn from(err: rusqlite::Error) -> Self {
        match err.sqlite_error_code() {
            Some(ErrorCode::NotADatabase) => Self::Unreadable,
            Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Self::Busy,
            _ => Self::Other(err.to_string()),
        }
    }
}

/// How many entries the history holds, read with `master` without changing
/// any file. Without sidecars the database is opened as immutable, so none
/// are created; with them it is read through SQLite's locks.
///
/// # Errors
///
/// See [`ReadError`].
pub fn count_entries(survey: &Survey, master: &[u8; KEY_SIZE]) -> Result<u64, ReadError> {
    let database = survey.part(PartKind::Database).ok_or(ReadError::Missing)?;
    let keys = compass_crypto::keys::derive_all(master);
    let db = compass_sqlcipher_sys::open_read_only(
        &database.path,
        &keys.database,
        !survey.has_sidecars(),
    )?;
    let initialised: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'selection')",
        [],
        |row| row.get(0),
    )?;
    if !initialised {
        return Err(ReadError::NotInitialised);
    }
    let count: i64 = db.query_row("SELECT count(*) FROM selection", [], |row| row.get(0))?;
    Ok(u64::try_from(count).unwrap_or(0))
}

/// What a reset does with the history's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposal {
    /// Delete them: the history is readable and the user asked for it gone.
    Delete,
    /// Move them into a backup directory: they cannot be read with the key
    /// there is now, and may be with one that comes back.
    Backup,
}

/// What a reset will do, worked out before anything is touched so it can be
/// shown first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPlan {
    /// Delete or set aside.
    pub disposal: Disposal,
    /// The parts, in [`PartKind::RESET_ORDER`].
    pub parts: Vec<Part>,
    /// Where [`Disposal::Backup`] moves them; `None` for a delete.
    pub backup_dir: Option<PathBuf>,
}

/// Plans a reset of everything `survey` found. `stamp` names the backup
/// directory, a Unix time in seconds.
#[must_use]
pub fn plan_reset(survey: &Survey, disposal: Disposal, stamp: u64) -> ResetPlan {
    ResetPlan {
        disposal,
        parts: survey.parts.clone(),
        backup_dir: (disposal == Disposal::Backup)
            .then(|| survey.dir.join(format!("{BACKUP_DIR_PREFIX}{stamp}"))),
    }
}

/// A reset that stopped part way.
#[derive(Debug, thiserror::Error)]
#[error("could not {action} {}: {source}", path.display())]
pub struct ResetError {
    /// What was being done: "delete", "move" or "create".
    pub action: &'static str,
    /// The path it was being done to.
    pub path: PathBuf,
    /// Why it failed.
    #[source]
    pub source: io::Error,
}

/// Carries out `plan`, stopping at the first failure so that what is left
/// stays together. A part that has gone since the survey is skipped.
///
/// # Errors
///
/// The first operation that failed; the parts after it are untouched.
pub fn apply_reset(plan: &ResetPlan) -> Result<(), ResetError> {
    if let Some(backup) = &plan.backup_dir {
        // `create_dir`, not `create_dir_all`: an existing backup is never
        // merged into.
        std::fs::create_dir(backup).map_err(|source| ResetError {
            action: "create",
            path: backup.clone(),
            source,
        })?;
    }
    for part in &plan.parts {
        let (action, result) = match &plan.backup_dir {
            Some(backup) => (
                "move",
                std::fs::rename(&part.path, backup.join(part.kind.file_name())),
            ),
            None if part.directory => ("delete", std::fs::remove_dir_all(&part.path)),
            None => ("delete", std::fs::remove_file(&part.path)),
        };
        match result {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(ResetError {
                    action,
                    path: part.path.clone(),
                    source,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: [u8; KEY_SIZE] = [3; KEY_SIZE];

    /// A history with `entries` texts, written the way the engine writes it.
    fn history(dir: &Path, entries: usize) {
        std::fs::create_dir_all(dir.join(PAYLOAD_DIR_NAME)).expect("payload dir");
        let keys = compass_crypto::keys::derive_all(&MASTER);
        let db = compass_sqlcipher_sys::open(&dir.join(DATABASE_FILE_NAME), &keys.database)
            .expect("open");
        crate::schema::run(&db).expect("schema");
        let mut n = 0;
        for i in 0..entries {
            crate::ingest::ingest(
                &db,
                &dir.join(PAYLOAD_DIR_NAME),
                &crate::ingest::Incoming {
                    data: format!("copied text {i}").as_bytes(),
                    mime_type: "text/plain",
                    source_app: None,
                },
                Some(&keys.clipboard),
                &mut || {
                    n += 1;
                    format!("id{n}")
                },
            )
            .expect("ingest");
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .expect("list")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_directory_is_an_empty_survey() {
        let dir = tempfile::tempdir().expect("tempdir");
        let survey = survey(&dir.path().join("absent")).expect("survey");
        assert!(survey.is_empty());
        assert_eq!(count_entries(&survey, &MASTER), Err(ReadError::Missing));
    }

    #[test]
    fn the_survey_measures_every_part_and_the_count_reads_without_writing() {
        let dir = tempfile::tempdir().expect("tempdir");
        history(dir.path(), 3);
        let survey = survey(dir.path()).expect("survey");
        let database = survey.part(PartKind::Database).expect("database");
        assert_eq!(
            database.bytes,
            std::fs::metadata(dir.path().join(DATABASE_FILE_NAME))
                .expect("metadata")
                .len()
        );
        let payloads = survey.part(PartKind::Payloads).expect("payloads");
        assert!(payloads.directory);
        assert_eq!(payloads.files, 3);
        assert!(payloads.bytes > 0);
        assert!(!survey.has_sidecars(), "the writer closed cleanly");

        let before = names(dir.path());
        let bytes = std::fs::read(&database.path).expect("read");
        assert_eq!(count_entries(&survey, &MASTER), Ok(3));
        assert_eq!(names(dir.path()), before, "no sidecar was created");
        assert_eq!(std::fs::read(&database.path).expect("read"), bytes);
    }

    #[test]
    fn another_key_is_reported_as_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        history(dir.path(), 1);
        let survey = survey(dir.path()).expect("survey");
        assert_eq!(
            count_entries(&survey, &[9; KEY_SIZE]),
            Err(ReadError::Unreadable)
        );
    }

    #[test]
    fn a_reset_that_deletes_removes_every_part_and_nothing_else() {
        let dir = tempfile::tempdir().expect("tempdir");
        history(dir.path(), 2);
        std::fs::write(dir.path().join("compass-file-index.db"), b"unrelated").expect("write");
        let plan = plan_reset(&survey(dir.path()).expect("survey"), Disposal::Delete, 7);
        assert_eq!(plan.backup_dir, None);
        apply_reset(&plan).expect("reset");
        assert_eq!(names(dir.path()), ["compass-file-index.db"]);
    }

    #[test]
    fn a_reset_that_backs_up_keeps_the_history_whole_beside_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        history(dir.path(), 2);
        let plan = plan_reset(
            &survey(dir.path()).expect("survey"),
            Disposal::Backup,
            1_700_000_000,
        );
        let backup = dir.path().join("compass-clipboard-backup-1700000000");
        assert_eq!(plan.backup_dir.as_deref(), Some(backup.as_path()));
        apply_reset(&plan).expect("reset");
        assert_eq!(names(dir.path()), ["compass-clipboard-backup-1700000000"]);
        assert_eq!(names(&backup), [PAYLOAD_DIR_NAME, DATABASE_FILE_NAME]);
        // Still the same history, readable with the key it was made under.
        assert_eq!(
            count_entries(&survey(&backup).expect("survey"), &MASTER),
            Ok(2)
        );
    }

    #[test]
    fn an_existing_backup_is_never_merged_into() {
        let dir = tempfile::tempdir().expect("tempdir");
        history(dir.path(), 1);
        let plan = plan_reset(&survey(dir.path()).expect("survey"), Disposal::Backup, 5);
        std::fs::create_dir(dir.path().join("compass-clipboard-backup-5")).expect("mkdir");
        let err = apply_reset(&plan).expect_err("refused");
        assert_eq!(err.action, "create");
        assert!(
            dir.path().join(DATABASE_FILE_NAME).exists(),
            "nothing moved"
        );
    }
}
