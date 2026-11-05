//! The incremental-scan concrete scanner.
//!
//! Ports `IncrementalScanner`: instead of walking everything, each directory
//! is diffed against the index — direct children become `Modify` events, rows
//! with nothing on disk become deletes — and only directories the database
//! says changed are descended into. Exhaustive mode finds those under the
//! entrypoint newer than the last successful scan; pruned mode climbs to the
//! nearest successful scan and follows only changed or new directories.
//!
//! Like its sibling, the shared half lives in [`Scanner`]; this struct owns
//! one, a read database behind [`IndexReader`], an [`IoPacer`], and the entry
//! filter. Interrupting only flags the core — the C++ loops poll the flag
//! between directories rather than stopping a walk mid-stride, and so do
//! these.
//!
//! One structural delta: the C++ streams new entries to its caller through
//! an `EntryCallback` while diffing. A callback borrowing the reader cannot
//! cross a `&mut self` scan method, so the diff returns the entries with
//! their new flags and the caller queues from those instead.
//!
//! One behavioural delta: a directory holding a `CACHEDIR.TAG` is diffed as
//! empty, and an entrypoint inside one is forgotten rather than read. The C++
//! checks the tag only in its full walk, so a cache created after the last
//! full scan was indexed file by file here.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_core::file_walk::{IndexWalk, WalkEntry, is_cache_directory};
use compass_core::io_pacer::IoPacer;

use crate::db_writer::{DbWriter, IndexDatabase};
use crate::query_reader::IndexReader;
use crate::scan::{IncrementalScan, Scan, ScanData, ScanMode};
use crate::scanner::{
    Scanner, StatusCallback, cutoff_seconds, file_event, home_dir, modified_seconds, root_event,
};

/// An incremental scan: diff what changed, descend where it did.
pub struct IncrementalScanner<D: IndexDatabase, R: IndexReader> {
    core: Scanner<D>,
    reader: R,
    filter: IndexWalk,
    pacer: IoPacer,
    /// The entries seen by the directory being diffed, so rows with nothing
    /// on disk read as deletes. A member reused across directories, like the
    /// C++ `m_currentEntries`, rather than a fresh set per call.
    current_entries: HashSet<PathBuf>,
}

impl<D: IndexDatabase, R: IndexReader> IncrementalScanner<D, R> {
    /// Tracks `scan` against `writer`, reading the index through `reader`
    /// and reporting to `on_status`.
    pub fn new(writer: Arc<DbWriter<D>>, scan: Scan, reader: R, on_status: StatusCallback) -> Self {
        Self {
            core: Scanner::new(writer, scan, on_status),
            reader,
            filter: IndexWalk::new(home_dir().as_deref()),
            pacer: IoPacer::system(),
            current_entries: HashSet::new(),
        }
    }

    /// Runs the scan to `Succeeded` — or `Interrupted` when flagged, `Failed`
    /// when the scan is not an incremental one.
    pub fn run(&mut self) {
        self.core.start();
        let scan = self.core.scan().clone();
        match scan.data {
            ScanData::Incremental(ref incremental) => {
                self.scan(&scan.path, incremental);
                self.core.finish();
            }
            ScanData::Full(_) => self.core.fail(),
        }
    }

    /// Flags the scan. The directory loops poll the flag between
    /// directories; there is no walk to stop, the way the C++ `interrupt`
    /// only sets its flag.
    pub fn interrupt(&self) {
        self.core.interrupt();
    }

    /// A shareable [`IncrementalScanner::interrupt`] over the same flag.
    pub fn stop_handle(&self) -> Arc<dyn Fn() + Send + Sync> {
        self.core.interrupt_handle()
    }

    /// Diffs the direct contents of `root` against the index, returning each
    /// visited entry with whether it is new to the index. A cache directory
    /// lists as empty, so whatever was indexed inside it is deleted.
    fn process_directory(&mut self, root: &Path) -> Vec<(WalkEntry, bool)> {
        self.pacer.checkpoint();
        let indexed = self.reader.list_indexed_directory_files(root);
        let mut deleted = Vec::new();
        let mut events = vec![root_event(root)];
        let mut visited = Vec::new();

        self.current_entries.clear();
        self.current_entries.insert(root.to_path_buf());

        let listing = (!is_cache_directory(root))
            .then(|| std::fs::read_dir(root))
            .and_then(Result::ok);
        if let Some(listing) = listing {
            for entry in listing {
                let Ok(entry) = entry else {
                    continue;
                };
                let path = entry.path();
                let (is_symlink, is_directory) = entry_kind(&entry);
                if !self.filter.should_visit(&path, is_symlink, is_directory) {
                    continue;
                }
                self.current_entries.insert(path.clone());
                let record = WalkEntry {
                    path,
                    is_directory,
                    is_symlink,
                };
                let is_new = !indexed.contains(&record.path);
                events.push(file_event(record.path.clone(), record.is_directory));
                visited.push((record, is_new));
            }
        }

        for path in &indexed {
            if !self.current_entries.contains(path) {
                deleted.push(path.clone());
            }
        }

        let processed = events.len();
        self.core.delete_indexed_files(deleted);
        self.core.index_events(events);
        self.core.report_progress(processed);
        visited
    }

    /// Whether `path` changed since `cutoff`: newer than the last successful
    /// scan, or not indexed at all. A time the disk will not give reads as
    /// changed, the way the C++ returns true when `last_write_time` fails.
    fn should_process_entry(&self, path: &Path, cutoff: i64) -> bool {
        let Some(modified) = modified_seconds(path) else {
            return true;
        };
        modified >= cutoff || !self.reader.tracks_file(path)
    }

    /// The entrypoint plus every directory under it newer than the last
    /// successful scan there. Without one there is nothing to compare
    /// against, so the entrypoint walks alone.
    fn scannable_directories(
        &mut self,
        root: &Path,
        max_depth: Option<usize>,
        excluded: &[PathBuf],
    ) -> Vec<PathBuf> {
        let mut scannable = vec![root.to_path_buf()];
        let Some(last) = self.reader.last_successful_scan(root) else {
            return scannable;
        };
        let cutoff = cutoff_seconds(&last);

        let mut walker = IndexWalk::new(home_dir().as_deref());
        walker.set_excluded_paths(excluded.to_vec());
        let walker = walker.max_depth(max_depth);
        let reader = &self.reader;
        let core = &mut self.core;
        walker.walk(root, |entry| {
            core.report_progress(1);
            if !entry.is_directory {
                return;
            }
            let fresh = modified_seconds(&entry.path).is_none_or(|modified| modified >= cutoff);
            if fresh || !reader.tracks_file(&entry.path) {
                scannable.push(entry.path.clone());
            }
        });
        scannable
    }

    /// Reads everything under the entrypoint, descending into new
    /// directories found along the way.
    fn exhaustive_scan(&mut self, root: &Path, scan: &IncrementalScan) {
        let mut new_dirs = VecDeque::new();
        let mut processed = HashSet::new();

        for dir in self.scannable_directories(root, scan.max_depth, &scan.excluded_paths) {
            if self.core.is_interrupted() {
                break;
            }
            processed.insert(dir.clone());
            for (entry, is_new) in self.process_directory(&dir) {
                if is_new && entry.is_directory {
                    new_dirs.push_back(entry.path);
                }
            }
        }

        while let Some(dir) = new_dirs.pop_front() {
            if self.core.is_interrupted() {
                break;
            }
            if !processed.insert(dir.clone()) {
                continue;
            }
            for (entry, is_new) in self.process_directory(&dir) {
                if is_new && entry.is_directory {
                    new_dirs.push_back(entry.path);
                }
            }
        }
    }

    /// Climbs to the nearest successful scan for its timestamp, then follows
    /// only changed or new directories down from the entrypoint.
    fn pruned_scan(&mut self, root: &Path) {
        let mut dir = root.to_path_buf();
        let cutoff = loop {
            if let Some(last) = self.reader.last_successful_scan(&dir) {
                break cutoff_seconds(&last);
            }
            match dir.parent().map(Path::to_path_buf) {
                Some(parent) if parent != dir => dir = parent,
                _ => break 0,
            }
        };

        let mut queue = VecDeque::from([root.to_path_buf()]);
        while let Some(dir) = queue.pop_front() {
            if self.core.is_interrupted() {
                break;
            }
            for (entry, is_new) in self.process_directory(&dir) {
                if !entry.is_directory {
                    continue;
                }
                if is_new || self.should_process_entry(&entry.path, cutoff) {
                    queue.push_back(entry.path);
                }
            }
        }
    }

    /// Reads what changed under `root`, exhaustively or pruned. An
    /// entrypoint inside a cache directory has nothing to read: what the
    /// index holds of it is deleted instead.
    fn scan(&mut self, root: &Path, scan: &IncrementalScan) {
        if root.ancestors().skip(1).any(is_cache_directory) {
            self.core.delete_indexed_files(vec![root.to_path_buf()]);
            return;
        }
        self.filter.set_excluded_paths(scan.excluded_paths.clone());
        self.filter
            .set_excluded_filenames(scan.excluded_filenames.clone());
        match scan.mode {
            ScanMode::Exhaustive => self.exhaustive_scan(root, scan),
            ScanMode::Pruned => self.pruned_scan(root),
        }
    }
}

/// A directory listing's symlink and directory flags. The type query never
/// follows links, so links resolve through a second query the way the C++
/// `is_directory` reads through the link — while the filter still refuses
/// the link itself.
fn entry_kind(entry: &std::fs::DirEntry) -> (bool, bool) {
    let file_type = entry.file_type().ok();
    let is_symlink = file_type.as_ref().is_some_and(|kind| kind.is_symlink());
    let is_directory = if is_symlink {
        entry.metadata().is_ok_and(|metadata| metadata.is_dir())
    } else {
        file_type.as_ref().is_some_and(|kind| kind.is_dir())
    };
    (is_symlink, is_directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, SystemTime};

    use crate::db_writer::{FileEvent, IndexDatabase, ScanRecord, ScanStatus, ScanType};
    use crate::query_engine::{SearchCandidate, SearchOptions};
    use crate::query_policy::VocabularySuggestion;
    use crate::scan::{FullScan, IncrementalScan, ScanMode};

    /// A database recording batches, events and deletes, in order.
    struct RecordingDb {
        batches: Arc<Mutex<Vec<usize>>>,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl IndexDatabase for RecordingDb {
        fn is_open(&self) -> bool {
            true
        }

        fn update_scan_status(&mut self, _scan_id: i32, _status: ScanStatus) -> bool {
            true
        }

        fn finalize_scan(&mut self, _scan_id: i32, _status: ScanStatus, _count: i64) -> bool {
            true
        }

        fn set_scan_error(&mut self, _scan_id: i32, _error: &str) -> bool {
            true
        }

        fn prune_scan_history(&mut self, _max_age_seconds: i64) -> bool {
            true
        }

        fn create_scan(&mut self, path: &Path, scan_type: ScanType) -> Result<ScanRecord, String> {
            Ok(ScanRecord {
                id: 7,
                status: ScanStatus::Pending,
                created_at: 0,
                finished_at: 0,
                indexed_file_count: 0,
                path: path.to_path_buf(),
                scan_type,
            })
        }

        fn index_files(&mut self, _paths: &[PathBuf]) {}

        fn delete_indexed_files(&mut self, paths: &[PathBuf]) {
            self.deleted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(paths.iter().cloned());
        }

        fn retain_indexed_files(
            &mut self,
            _roots: &[PathBuf],
            _excluded: &[PathBuf],
        ) -> Result<(), crate::db_writer::ReconcileError> {
            Ok(())
        }

        fn delete_all_indexed_files(&mut self) {}

        fn compact(&mut self) {}

        fn needs_compaction(&self) -> bool {
            false
        }

        fn rebuild_vocabulary(&mut self) {}

        fn index_events(&mut self, events: &[FileEvent]) {
            self.batches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(events.len());
        }
    }

    /// The index as the scanner reads it: children per directory, tracked
    /// paths, successful scans, and the scan lookups it was asked.
    struct FakeReader {
        children: HashMap<PathBuf, HashSet<PathBuf>>,
        tracked: HashSet<PathBuf>,
        scans: HashMap<PathBuf, ScanRecord>,
        scan_queries: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl FakeReader {
        fn empty() -> Self {
            Self {
                children: HashMap::new(),
                tracked: HashSet::new(),
                scans: HashMap::new(),
                scan_queries: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl IndexReader for FakeReader {
        fn is_open(&self) -> bool {
            true
        }

        fn search_candidates(
            &self,
            _query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Vec::new()
        }

        fn search_skeleton_candidates(
            &self,
            _query: &str,
            _limit: usize,
            _options: &SearchOptions,
        ) -> Vec<SearchCandidate> {
            Vec::new()
        }

        fn vocabulary_suggestions(
            &self,
            _word: &str,
            _top: i32,
            _prefix: bool,
        ) -> Vec<VocabularySuggestion> {
            Vec::new()
        }

        fn list_indexed_directory_files(&self, path: &Path) -> HashSet<PathBuf> {
            self.children.get(path).cloned().unwrap_or_default()
        }

        fn tracks_file(&self, path: &Path) -> bool {
            self.tracked.contains(path)
        }

        fn last_successful_scan(&self, path: &Path) -> Option<ScanRecord> {
            self.scan_queries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(path.to_path_buf());
            self.scans.get(path).cloned()
        }

        fn last_scan(&self, path: &Path, scan_type: ScanType) -> Option<ScanRecord> {
            self.scans
                .get(path)
                .filter(|scan| scan.scan_type == scan_type)
                .cloned()
        }

        fn has_vocabulary(&self) -> bool {
            false
        }

        fn recent_directories(&self, _limit: usize) -> Vec<PathBuf> {
            Vec::new()
        }
    }

    struct Harness {
        writer: Arc<DbWriter<RecordingDb>>,
        batches: Arc<Mutex<Vec<usize>>>,
        deleted: Arc<Mutex<Vec<PathBuf>>>,
        statuses: Arc<Mutex<Vec<(ScanStatus, usize)>>>,
    }

    impl Harness {
        fn new() -> Self {
            let batches = Arc::new(Mutex::new(Vec::new()));
            let deleted = Arc::new(Mutex::new(Vec::new()));
            let maker_batches = Arc::clone(&batches);
            let maker_deleted = Arc::clone(&deleted);
            let writer = Arc::new(DbWriter::new(move || RecordingDb {
                batches: maker_batches,
                deleted: maker_deleted,
            }));
            Self {
                writer,
                batches,
                deleted,
                statuses: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn scanner(
            &self,
            data: ScanData,
            root: &Path,
            reader: FakeReader,
        ) -> IncrementalScanner<RecordingDb, FakeReader> {
            let statuses = Arc::clone(&self.statuses);
            IncrementalScanner::new(
                Arc::clone(&self.writer),
                Scan {
                    path: root.to_path_buf(),
                    data,
                    notify: false,
                },
                reader,
                Box::new(move |status, count| {
                    statuses
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((status, count));
                }),
            )
        }

        fn incremental(
            &self,
            root: &Path,
            reader: FakeReader,
            mode: ScanMode,
        ) -> IncrementalScanner<RecordingDb, FakeReader> {
            self.scanner(
                ScanData::Incremental(IncrementalScan {
                    mode,
                    max_depth: None,
                    excluded_filenames: Vec::new(),
                    excluded_paths: Vec::new(),
                }),
                root,
                reader,
            )
        }

        fn reported(&self) -> Vec<(ScanStatus, usize)> {
            self.statuses
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    /// Waits until `locked` holds `count` entries, so assertions see finished
    /// writer work rather than racing it.
    fn wait_for<T: Clone>(locked: &Mutex<Vec<T>>, count: usize) -> Vec<T> {
        let start = std::time::Instant::now();
        loop {
            let entries = locked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if entries.len() >= count {
                return entries;
            }
            if start.elapsed() > Duration::from_secs(5) {
                panic!("timed out waiting for {count} entries");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fixture() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary tree")
    }

    fn scan_record(path: &Path, created_at: u64) -> ScanRecord {
        ScanRecord {
            id: 3,
            status: ScanStatus::Succeeded,
            created_at,
            finished_at: created_at,
            indexed_file_count: 0,
            path: path.to_path_buf(),
            scan_type: ScanType::Incremental,
        }
    }

    #[test]
    fn new_directories_are_descended_into() {
        let dir = fixture();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("seed dir");
        std::fs::write(sub.join("nested.txt"), "deep").expect("seed file");

        // Nothing indexed: the subdirectory is new, so the scan descends.
        let harness = Harness::new();
        let mut scanner =
            harness.incremental(dir.path(), FakeReader::empty(), ScanMode::Exhaustive);
        scanner.run();

        // Root diffs to root plus sub; the sub diffs to sub plus nested.
        assert_eq!(wait_for(&harness.batches, 2), [2, 2]);
        assert!(
            harness
                .deleted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
        let reported = harness.reported();
        assert_eq!(reported.first(), Some(&(ScanStatus::Started, 0)));
        assert_eq!(reported.last(), Some(&(ScanStatus::Succeeded, 4)));
    }

    #[test]
    fn missing_rows_are_deleted_and_known_dirs_are_not_redescended() {
        let dir = fixture();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("seed dir");
        std::fs::write(sub.join("nested.txt"), "deep").expect("seed file");
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        // Everything already indexed: no new directory to descend into, but
        // the stale row has nothing on disk.
        let mut reader = FakeReader::empty();
        let gone = dir.path().join("gone.txt");
        reader.children.insert(
            dir.path().to_path_buf(),
            HashSet::from([sub.clone(), gone.clone()]),
        );
        reader.tracked.insert(sub.clone());

        let harness = Harness::new();
        let mut scanner = harness.incremental(dir.path(), reader, ScanMode::Exhaustive);
        scanner.run();

        // Root diffs to root, sub and a.txt; nothing new descends.
        assert_eq!(wait_for(&harness.batches, 1), [3]);
        assert_eq!(wait_for(&harness.deleted, 1), [gone]);
    }

    #[test]
    fn exhaustive_prunes_directories_older_than_the_last_scan() {
        let dir = fixture();
        let stale = dir.path().join("stale");
        std::fs::create_dir(&stale).expect("seed dir");
        std::fs::write(stale.join("old.txt"), "old").expect("seed file");
        let fresh = dir.path().join("fresh");
        std::fs::create_dir(&fresh).expect("seed dir");
        std::fs::write(fresh.join("new.txt"), "new").expect("seed file");
        // Age the stale tree past the last scan; the fresh tree stays new.
        // The directory time moves when children arrive, so it goes last.
        std::fs::File::options()
            .read(true)
            .open(&stale)
            .expect("open stale")
            .set_modified(SystemTime::UNIX_EPOCH)
            .expect("age stale");

        let mut reader = FakeReader::empty();
        reader.scans.insert(
            dir.path().to_path_buf(),
            scan_record(dir.path(), 1_000_000_000),
        );
        reader.tracked.insert(stale.clone());
        reader.tracked.insert(fresh.clone());
        // Both subtrees indexed: neither is new, so only the fresh one is
        // descended into. The stale one is old, tracked, and pruned.
        reader.children.insert(
            dir.path().to_path_buf(),
            HashSet::from([stale.clone(), fresh.clone()]),
        );
        reader.children.insert(fresh.clone(), HashSet::new());

        let harness = Harness::new();
        let mut scanner = harness.incremental(dir.path(), reader, ScanMode::Exhaustive);
        scanner.run();

        // The stale tree is never diffed: root, stale and fresh from the
        // root diff, then fresh alone. Two batches, four files of progress
        // from the scannable walk plus five indexed events.
        assert_eq!(wait_for(&harness.batches, 2), [3, 2]);
        let reported = harness.reported();
        assert_eq!(reported.first(), Some(&(ScanStatus::Started, 0)));
        assert_eq!(reported.last(), Some(&(ScanStatus::Succeeded, 9)));
    }

    #[test]
    fn pruned_follows_only_changed_or_new_directories() {
        let dir = fixture();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).expect("seed dir");
        std::fs::write(sub.join("old.txt"), "old").expect("seed file");
        // Age the whole tree past the cutoff the ancestor scan will give.
        for path in [dir.path().to_path_buf(), sub.clone()] {
            std::fs::File::options()
                .read(true)
                .open(&path)
                .expect("open for ageing")
                .set_modified(SystemTime::UNIX_EPOCH)
                .expect("age tree");
        }

        let mut reader = FakeReader::empty();
        reader.scans.insert(
            dir.path().to_path_buf(),
            scan_record(dir.path(), 1_000_000_000),
        );
        reader.tracked.insert(sub.clone());
        reader
            .children
            .insert(dir.path().to_path_buf(), HashSet::from([sub.clone()]));
        reader.children.insert(sub.clone(), HashSet::new());

        let harness = Harness::new();
        let mut scanner = harness.incremental(dir.path(), reader, ScanMode::Pruned);
        scanner.run();

        // The subdirectory is tracked and older than the cutoff, so the scan
        // diffs the root and stops: root plus sub, nothing deleted.
        assert_eq!(wait_for(&harness.batches, 1), [2]);
        assert!(
            harness
                .deleted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn pruned_climbs_to_the_nearest_successful_scan() {
        let dir = fixture();

        let reader = FakeReader::empty();
        let queries = Arc::clone(&reader.scan_queries);
        let harness = Harness::new();
        let mut scanner = harness.incremental(dir.path(), reader, ScanMode::Pruned);
        scanner.run();

        // No scan anywhere: the lookup climbs from the entrypoint to the
        // filesystem root before giving up on a cutoff.
        assert_eq!(wait_for(&harness.batches, 1), [1]);
        let queried = queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(queried.first(), Some(&dir.path().to_path_buf()));
        assert_eq!(queried.last(), Some(&PathBuf::from("/")));
    }

    #[test]
    fn an_interrupted_scan_stops_between_directories() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        let harness = Harness::new();
        let mut scanner =
            harness.incremental(dir.path(), FakeReader::empty(), ScanMode::Exhaustive);
        scanner.interrupt();
        scanner.run();

        assert_eq!(
            harness.reported().last(),
            Some(&(ScanStatus::Interrupted, 0))
        );
        assert!(
            harness
                .batches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn a_full_scan_shape_fails_without_indexing() {
        let dir = fixture();

        let harness = Harness::new();
        let mut scanner = harness.scanner(
            ScanData::Full(FullScan {
                excluded_paths: Vec::new(),
            }),
            dir.path(),
            FakeReader::empty(),
        );
        scanner.run();

        assert_eq!(
            harness.reported(),
            [(ScanStatus::Started, 0), (ScanStatus::Failed, 0)]
        );
    }

    #[test]
    fn entries_read_as_changed_without_a_time_or_a_row() {
        let dir = fixture();
        std::fs::write(dir.path().join("a.txt"), "notes").expect("seed file");

        let harness = Harness::new();
        let scanner = harness.incremental(dir.path(), FakeReader::empty(), ScanMode::Exhaustive);

        // Missing from the disk's clock and the index alike: changed.
        assert!(scanner.should_process_entry(Path::new("/no/such/file"), 0));
        // Older than the cutoff but never indexed: changed anyway.
        assert!(scanner.should_process_entry(&dir.path().join("a.txt"), i64::MAX));
    }
}
