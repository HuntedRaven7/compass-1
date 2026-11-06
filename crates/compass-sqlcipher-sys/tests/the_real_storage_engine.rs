//! The vendored stack, exercised against real files.
//!
//! Every assertion here is about something that cannot be checked by reading
//! SQL strings: that the database is genuinely encrypted, that the tokenizer is
//! genuinely registered, and that the registration order this crate enforces is
//! genuinely load-bearing rather than cargo-culted from `clipboard-db.cpp`.

use std::path::PathBuf;

use compass_sqlcipher_sys::open;
use compass_sqlcipher_sys::rusqlite::{Error, named_params};

/// A path in a fresh temporary directory, returned with the directory so the
/// caller keeps it alive — dropping a `TempDir` deletes the file underneath.
fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join(name);
    (dir, path)
}

/// Raw key material, as `compass_crypto` derives it: 32 bytes, used directly.
const KEY: &[u8] = &[0x5a; 32];

#[test]
fn the_file_on_disk_is_actually_encrypted() {
    let (_dir, path) = scratch("enc.db");
    {
        let db = open(&path, KEY).expect("open");
        db.execute_batch("CREATE TABLE t (v TEXT)").expect("create");
        db.execute_batch("INSERT INTO t VALUES ('secret-payload')")
            .expect("insert");
    }

    let bytes = std::fs::read(&path).expect("read the file back");

    // The control for the two assertions below: an unencrypted SQLite file
    // starts with this magic and contains its own string data in the clear. If
    // encryption silently stopped happening, these are what would catch it.
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "the header says this is a plain SQLite file, so it was not encrypted"
    );
    assert!(
        !bytes.windows(14).any(|w| w == b"secret-payload"),
        "the inserted text is readable in the raw file"
    );
}

#[test]
fn an_unencrypted_database_is_recognisably_different() {
    // Scoping the test above: with no key, the same code path produces a file
    // that DOES have the magic and DOES contain the payload. Without this, a
    // bug that wrote no data at all would pass `the_file_on_disk_is_actually_encrypted`.
    let (_dir, path) = scratch("plain.db");
    {
        let db = open(&path, &[]).expect("open");
        db.execute_batch("CREATE TABLE t (v TEXT)").expect("create");
        db.execute_batch("INSERT INTO t VALUES ('secret-payload')")
            .expect("insert");
    }

    let bytes = std::fs::read(&path).expect("read the file back");
    assert!(
        bytes.starts_with(b"SQLite format 3"),
        "an unkeyed database should be a plain SQLite file"
    );
    assert!(
        bytes.windows(14).any(|w| w == b"secret-payload"),
        "an unkeyed database should hold its payload in the clear"
    );
}

#[test]
fn the_wrong_key_does_not_open_the_database() {
    let (_dir, path) = scratch("wrongkey.db");
    {
        let db = open(&path, KEY).expect("open");
        db.execute_batch("CREATE TABLE t (v TEXT)").expect("create");
    }

    let err = open(&path, &[0x17; 32])
        .expect_err("the wrong key must be refused by open, not by a later query");
    assert!(
        matches!(err, Error::SqliteFailure(..)),
        "expected SQLite to refuse, got {err:?}"
    );

    // And the right key does, so the failure above is about the key rather
    // than about the file being broken.
    let db = open(&path, KEY).expect("open");
    db.execute_batch("SELECT * FROM t")
        .expect("the right key reads");
}

#[test]
fn the_vendored_tokenizer_is_registered_on_every_connection() {
    let (_dir, path) = scratch("fts.db");
    let db = open(&path, KEY).expect("open");

    // This statement is the whole point of the crate: without fuzzy_trigram it
    // fails with "no such tokenizer", and so does every later access.
    db.execute_batch(
        "CREATE VIRTUAL TABLE selection_fts USING fts5(content, selection_id UNINDEXED, \
         tokenize='fuzzy_trigram remove_diacritics 2')",
    )
    .expect("create the FTS table the clipboard schema declares");

    db.execute_batch(
        "INSERT INTO selection_fts(selection_id, content) VALUES ('s1','firefox homepage')",
    )
    .expect("insert");

    let found: String = db
        .query_row(
            "SELECT selection_id FROM selection_fts WHERE selection_fts MATCH :q",
            named_params! { ":q": "\"fir\"" },
            |row| row.get(0),
        )
        .expect("expected a match for \"fir\"");
    assert_eq!(found, "s1");

    // A second connection to an EXISTING ENCRYPTED file. This is the load-
    // bearing half of the test and the reason it is not just "create a table":
    //
    //  * registration is per connection, so it catches `open` registering only
    //    once per process;
    //  * and registering before keying fails only here. On a fresh file it
    //    succeeds, which is how an earlier attempt at this (an
    //    `sqlite3_auto_extension` hook, which necessarily runs before any
    //    `PRAGMA key`) passed its tests and would still have failed on every
    //    real clipboard history. See ADR-0014.
    let db2 = open(&path, KEY).expect("reopen");
    let count: i64 = db2
        .query_row(
            "SELECT count(*) FROM selection_fts WHERE selection_fts MATCH :q",
            named_params! { ":q": "\"hom\"" },
            |row| row.get(0),
        )
        .expect("query on the second connection");
    assert_eq!(count, 1);
}

#[test]
fn a_short_term_cannot_reach_a_longer_document() {
    // The premise `compass_clipboard::search` rests on, checked here against
    // the real tokenizer rather than a stand-in.
    let (_dir, path) = scratch("short.db");
    let db = open(&path, KEY).expect("open");
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(content, tokenize='fuzzy_trigram remove_diacritics 2')",
    )
    .expect("create");
    for doc in ["firefox homepage", "ab", "abc"] {
        db.execute(
            "INSERT INTO t(content) VALUES (:c)",
            named_params! { ":c": doc },
        )
        .expect("insert");
    }

    let count = |term: &str| -> i64 {
        db.query_row(
            "SELECT count(*) FROM t WHERE t MATCH :q",
            named_params! { ":q": term },
            |row| row.get(0),
        )
        .expect("count")
    };

    assert_eq!(count("\"fir\""), 1, "a three-run term reaches the document");
    assert_eq!(
        count("\"fi\""),
        0,
        "a two-character term cannot reach `firefox`"
    );
    assert_eq!(count("\"bc\""), 0, "nor can it reach `abc`");

    // And the case that makes "MATCH finds nothing" too strong: a short term
    // still matches a document that is itself that short. Stated as a test so
    // the claim in `search.rs` cannot quietly become the wrong one again.
    assert_eq!(
        count("\"ab\""),
        1,
        "a short term still matches a short document"
    );
}

#[test]
fn binding_a_parameter_that_does_not_exist_is_an_error() {
    // The C++ wrapper's `bind` returns early when `paramIndex` is 0, so a typo
    // in a parameter name runs the query with NULL in that position instead of
    // failing. rusqlite refuses, and this is the test that says so.
    let (_dir, path) = scratch("bind.db");
    let db = open(&path, KEY).expect("open");
    db.execute_batch("CREATE TABLE t (v TEXT)").expect("create");

    let mut stmt = db
        .prepare("SELECT * FROM t WHERE v = :actual")
        .expect("prepare");
    assert!(matches!(
        stmt.query(named_params! { ":typo": "x" }).map(drop),
        Err(Error::InvalidParameterName(_))
    ));
    stmt.query(named_params! { ":actual": "x" })
        .map(drop)
        .expect("the real name binds");
}

#[test]
fn the_pragmas_are_applied() {
    let (_dir, path) = scratch("pragma.db");
    let db = open(&path, KEY).expect("open");

    let mode: String = db
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("a row");
    assert_eq!(mode, "wal", "clipboard-db.cpp sets WAL on every connection");

    let cipher: String = db
        .query_row("PRAGMA cipher_version", [], |row| row.get(0))
        .expect("a row");
    assert!(
        cipher.starts_with('4'),
        "expected a SQLCipher 4.x file format, got {cipher:?}"
    );
}

#[test]
fn a_read_only_open_reads_without_creating_or_writing() {
    let (dir, path) = scratch("ro.db");
    assert!(
        compass_sqlcipher_sys::open_read_only(&path, KEY, true).is_err(),
        "a missing database is not created"
    );
    assert!(!path.exists());
    {
        let db = open(&path, KEY).expect("open");
        db.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('a')")
            .expect("create");
    }
    // The writer closed cleanly, so the sidecars went with it.
    let sidecars = |dir: &std::path::Path| {
        std::fs::read_dir(dir)
            .expect("list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "ro.db")
            .count()
    };
    assert_eq!(sidecars(dir.path()), 0);
    let before = std::fs::read(&path).expect("read");

    let db = compass_sqlcipher_sys::open_read_only(&path, KEY, true).expect("read-only open");
    let count: i64 = db
        .query_row("SELECT count(*) FROM t", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
    assert!(
        db.execute_batch("INSERT INTO t VALUES ('b')").is_err(),
        "the connection cannot write"
    );
    drop(db);
    assert_eq!(sidecars(dir.path()), 0, "no -wal or -shm was created");
    assert_eq!(std::fs::read(&path).expect("read"), before);

    assert!(
        compass_sqlcipher_sys::open_read_only(&path, &[0x17; 32], true).is_err(),
        "the wrong key is refused at open"
    );
}

#[test]
fn a_read_only_open_survives_a_path_a_uri_would_misread() {
    let (_dir, path) = scratch("odd ?#% name.db");
    {
        let db = open(&path, KEY).expect("open");
        db.execute_batch("CREATE TABLE t (v TEXT)").expect("create");
    }
    compass_sqlcipher_sys::open_read_only(&path, KEY, true)
        .expect("the escaped URI names the same file");
}
