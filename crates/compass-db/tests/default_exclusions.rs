//! The shipped exclusions against a real temporary index: package caches,
//! build output and tagged caches never reach the database, whichever scan
//! meets them, while the files around them do.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use compass_core::entry_filter::CACHEDIR_TAG_SIGNATURE;
use compass_db::db_writer::{DbWriter, IndexDatabase, ScanType};
use compass_db::file_indexer::FileIndexer;
use compass_db::sqlite_reader::SqliteReader;
use compass_db::sqlite_writer::{SqliteWriter, prepare_file_index_database};

const KEPT: &[&str] = &[
    "notes/report.txt",
    "archery/target/scores.txt",
    "crate/Cargo.toml",
    "crate/src/main.rs",
    "java/pom.xml",
];

/// Junk directories and a file inside each.
const JUNK: &[(&str, &str)] = &[
    ("web/node_modules", "left-pad/index.js"),
    (".cache", "thumbnails/large/a.png"),
    ("crate/target", "debug/app"),
    ("java/target", "classes/A.class"),
    ("py/__pycache__", "m.cpython-312.pyc"),
    ("py/.venv", "lib/site.py"),
    ("repo/.git", "objects/ab/cdef"),
];

struct Fixture {
    dir: tempfile::TempDir,
    database: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("index.db");
        prepare_file_index_database(&database).unwrap();
        let fixture = Self { dir, database };
        for path in KEPT {
            fixture.write(path, "contents");
        }
        for (dir, file) in JUNK {
            fixture.write(&format!("{dir}/{file}"), "contents");
        }
        fixture
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn rows(&self) -> Vec<PathBuf> {
        let db = compass_sqlcipher_sys::open(&self.database, &[]).unwrap();
        let mut query = db.prepare("SELECT path FROM indexed_file").unwrap();
        query
            .query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn indexed(&self, relative: &str) -> bool {
        let path = self.root().join(relative);
        self.rows().contains(&path)
    }

    fn indexer(
        &self,
    ) -> (
        FileIndexer<SqliteWriter, SqliteReader>,
        Arc<DbWriter<SqliteWriter>>,
    ) {
        let database = self.database.clone();
        let writer = Arc::new(DbWriter::new(move || {
            SqliteWriter::open(&database).unwrap()
        }));
        let database = self.database.clone();
        let indexer = FileIndexer::new(
            Arc::clone(&writer),
            move || SqliteReader::open(&database).unwrap(),
            "index.db",
        );
        indexer.set_config(vec![self.root()], Vec::new());
        (indexer, writer)
    }
}

fn settle(indexer: &FileIndexer<SqliteWriter, SqliteReader>, writer: &DbWriter<SqliteWriter>) {
    indexer.dispatcher().wait_until_idle();
    let (sender, receiver) = mpsc::channel();
    writer.submit(move |_| sender.send(()).unwrap(), false);
    receiver.recv_timeout(Duration::from_secs(10)).unwrap();
}

/// No row is a junk directory or anything inside one.
fn assert_no_junk(fixture: &Fixture) {
    let rows = fixture.rows();
    for (dir, _) in JUNK {
        let dir = fixture.root().join(dir);
        assert!(
            rows.iter().all(|row| !row.starts_with(&dir)),
            "{} was indexed",
            dir.display()
        );
    }
}

#[test]
fn a_full_scan_indexes_the_files_but_none_of_the_junk() {
    let fixture = Fixture::new();
    let (indexer, writer) = fixture.indexer();
    assert!(indexer.start());
    settle(&indexer, &writer);

    for kept in KEPT {
        assert!(fixture.indexed(kept), "{kept} was not indexed");
    }
    assert_no_junk(&fixture);
}

#[test]
fn an_incremental_scan_leaves_out_a_cache_made_after_the_full_scan() {
    let fixture = Fixture::new();
    let (indexer, writer) = fixture.indexer();
    assert!(indexer.start());
    settle(&indexer, &writer);

    fixture.write("later/plan.txt", "new notes");
    fixture.write("later/cache/CACHEDIR.TAG", CACHEDIR_TAG_SIGNATURE);
    fixture.write("later/cache/blob", "cached");
    fixture.write("later/node_modules/dep.js", "vendored");
    indexer.start_single_scan(&fixture.root(), ScanType::Incremental, Vec::new());
    settle(&indexer, &writer);

    assert!(fixture.indexed("later/plan.txt"));
    assert!(!fixture.indexed("later/cache/blob"));
    assert!(!fixture.indexed("later/node_modules/dep.js"));
    assert_no_junk(&fixture);
}

#[test]
fn an_incremental_scan_inside_a_cache_forgets_what_was_indexed_there() {
    let fixture = Fixture::new();
    fixture.write("tool/cache/CACHEDIR.TAG", CACHEDIR_TAG_SIGNATURE);
    fixture.write("tool/cache/sub/blob", "cached");
    let stale = fixture.root().join("tool/cache/sub/blob");
    SqliteWriter::open(&fixture.database)
        .unwrap()
        .index_files(std::slice::from_ref(&stale));
    assert!(fixture.rows().contains(&stale));

    let (indexer, writer) = fixture.indexer();
    indexer.start_single_scan(
        &fixture.root().join("tool/cache/sub"),
        ScanType::Incremental,
        Vec::new(),
    );
    settle(&indexer, &writer);

    assert!(!fixture.rows().contains(&stale));
    assert!(
        fixture
            .dir
            .path()
            .join("home/tool/cache/sub/blob")
            .is_file()
    );
}
