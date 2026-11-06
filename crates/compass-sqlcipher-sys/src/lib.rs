//! Opens Compass's databases: a [`rusqlite::Connection`] over SQLCipher,
//! keyed, with the vendored `fuzzy_trigram` tokenizer registered.
//!
//! # Why this crate exists at all
//!
//! The clipboard history is an encrypted SQLCipher database whose FTS5 table
//! declares a tokenizer that ships in this repository as C. Neither is
//! optional: a stock SQLite cannot open the file, and without the tokenizer
//! *every* access to `selection_fts` fails, including a plain `SELECT`. See
//! [ADR-0014](../../../docs/rust-engine/adr/0014-clipboard-storage-is-sqlcipher-plus-a-vendored-tokenizer.md).
//!
//! SQLCipher itself is `libsqlite3-sys`'s `bundled-sqlcipher` build, reached
//! through [`rusqlite`], which this crate re-exports. Callers use that
//! re-export rather than depending on `rusqlite` themselves, so there is one
//! version in the tree and one way to get a connection: [`open`].
//!
//! # Why the `unsafe` is here and not elsewhere
//!
//! The workspace sets `unsafe_code = "forbid"`. This crate declines to inherit
//! that (see its `Cargo.toml`) because the tokenizer registration is a call on
//! a raw `sqlite3*`. The bargain is that the exception is confined: [`open`]
//! returns an ordinary connection, and no other crate in the workspace needs
//! `unsafe` to read a clipboard history.
//!
//! # The ordering this crate exists to enforce
//!
//! `clipboard-db.cpp` opens, keys, registers the tokenizer, then sets pragmas —
//! in that order. It is not stylistic. Registering an FTS5 tokenizer queries
//! the database for the `fts5` API pointer, and on an encrypted database that
//! query fails until the key is set. [`open`] does all of it, so a caller
//! cannot get the order wrong.

#![deny(missing_docs)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;
use std::time::{Duration, Instant};

pub use rusqlite;
use rusqlite::{Connection, ErrorCode, OpenFlags, ffi};

unsafe extern "C" {
    /// `vendor/fuzzy-trigram/register.c`: registers the tokenizer with the FTS5
    /// module of `db`. Shaped like an `sqlite3_auto_extension` entry point;
    /// the last two arguments are unused because the tokenizer is linked
    /// statically.
    fn vicinaeFuzzyTrigramInit(
        db: *mut ffi::sqlite3,
        err: *mut *mut c_char,
        api: *const c_void,
    ) -> c_int;
}

/// Open `path`, key it with `key`, register `fuzzy_trigram`, and apply the
/// clipboard pragmas — in that order, because that order is load-bearing.
///
/// `key` is **raw key material**, not a passphrase: the 32 bytes
/// `compass_crypto` derives. It is passed to SQLCipher in the `x'...'` form,
/// which uses the bytes directly rather than running them through a KDF,
/// exactly as `db::Database::setKey` does. Handing a passphrase here would
/// produce a database the C++ engine cannot open.
///
/// Pass an empty slice for an unencrypted database; `PRAGMA key` is then
/// skipped entirely rather than issued with an empty value, which SQLCipher
/// treats differently.
///
/// # Errors
///
/// Returns SQLite's error if the file cannot be opened, the key is rejected,
/// or the tokenizer will not register. A wrong key is reported here rather
/// than at the first query.
pub fn open(path: &Path, key: &[u8]) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    // Before anything that takes a lock: keying, registering the tokenizer and
    // switching to WAL all do, and a connection opened while another holds one
    // otherwise fails at once with "database is locked". The timeout touches no
    // page, so it is safe before the key is set.
    conn.busy_timeout(BUSY_TIMEOUT)?;

    if !key.is_empty() {
        set_key(&conn, key)?;
    }
    register_tokenizer(&conn)?;

    for pragma in PRAGMAS {
        execute_through_busy(&conn, pragma)?;
    }

    Ok(conn)
}

/// Open an existing database at `path` for reading only, keyed with `key`,
/// with `fuzzy_trigram` registered — for diagnostics that must not change the
/// file.
///
/// Unlike [`open`] this never creates the file and applies none of the
/// clipboard pragmas: switching the journal mode is a write. With `immutable`
/// set SQLite takes no locks and neither reads nor creates the `-wal` and
/// `-shm` sidecars; that is only right while no writer has the file open,
/// which a caller knows from the sidecars being absent. Without it, an existing
/// `-shm` may be updated for reader coordination, as any reader does.
///
/// # Errors
///
/// As [`open`]; a missing file is `SQLITE_CANTOPEN`.
pub fn open_read_only(path: &Path, key: &[u8], immutable: bool) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = if immutable {
        Connection::open_with_flags(immutable_uri(path), flags | OpenFlags::SQLITE_OPEN_URI)?
    } else {
        Connection::open_with_flags(path, flags)?
    };
    conn.busy_timeout(BUSY_TIMEOUT)?;
    if !key.is_empty() {
        set_key(&conn, key)?;
    }
    register_tokenizer(&conn)?;
    Ok(conn)
}

/// Every byte a `file:` URI path may not hold as it is: all but the unreserved
/// characters and `/`.
const URI_PATH: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `path` as a `file:` URI with `immutable=1`, percent-encoded so that `?`,
/// `#` and `%` in a name, and a non-UTF-8 path, reach SQLite intact.
fn immutable_uri(path: &Path) -> String {
    let encoded = percent_encoding::percent_encode(path.as_os_str().as_encoded_bytes(), URI_PATH);
    format!("file:{encoded}?immutable=1")
}

/// Set the SQLCipher key from raw bytes.
///
/// `PRAGMA` does not accept bound parameters, so the key has to be part of the
/// SQL text. Hex-encoding it into SQLCipher's `x'...'` raw-key form is what
/// makes that safe as well as correct: the output alphabet is `0-9a-f`, so no
/// byte of key material can close the quote or change the statement, whatever
/// the key happens to be.
fn set_key(conn: &Connection, key: &[u8]) -> rusqlite::Result<()> {
    let mut pragma = String::with_capacity(key.len() * 2 + 20);
    pragma.push_str("PRAGMA key = \"x'");
    for byte in key {
        use std::fmt::Write as _;
        write!(pragma, "{byte:02x}").expect("writing to a String cannot fail");
    }
    pragma.push_str("'\";");
    conn.execute_batch(&pragma)?;

    // Touch the schema so a wrong key is reported by `open` rather than by
    // whatever query happens to run first. The C++ engine does the same, and
    // without it `open` succeeds on a database it cannot read.
    conn.query_row("SELECT count(*) FROM sqlite_master;", [], |_| Ok(()))
        .map_err(|err| with_context(err, "verifying the database key"))
}

/// Register the `fuzzy_trigram` tokenizer on this connection.
///
/// Must happen after keying — see the module docs.
fn register_tokenizer(conn: &Connection) -> rusqlite::Result<()> {
    // SAFETY: `conn` is borrowed for the whole block, so the handle is live,
    // and the entry point only uses it to look up the FTS5 API and register a
    // tokenizer: it neither closes the handle nor keeps it.
    let (rc, message) = unsafe {
        let handle = conn.handle();
        let rc = vicinaeFuzzyTrigramInit(handle, std::ptr::null_mut(), std::ptr::null());
        let message = if rc == ffi::SQLITE_OK {
            String::new()
        } else {
            let text = ffi::sqlite3_errmsg(handle);
            if text.is_null() {
                "no message".to_owned()
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            }
        };
        (rc, message)
    };
    if rc == ffi::SQLITE_OK {
        return Ok(());
    }
    Err(rusqlite::Error::SqliteFailure(
        ffi::Error::new(rc),
        Some(format!(
            "registering the fuzzy_trigram tokenizer: {message}"
        )),
    ))
}

/// Prefix SQLite's own message with what was being attempted.
fn with_context(err: rusqlite::Error, context: &str) -> rusqlite::Error {
    match err {
        rusqlite::Error::SqliteFailure(code, message) => rusqlite::Error::SqliteFailure(
            code,
            Some(match message {
                Some(message) => format!("{context}: {message}"),
                None => context.to_owned(),
            }),
        ),
        other => other,
    }
}

/// `execute_batch`, retried while another connection holds a lock.
///
/// The busy timeout covers most contention, but SQLite deliberately skips the
/// busy handler where waiting could deadlock — two fresh connections racing to
/// switch a new file into WAL is one — and reports `SQLITE_BUSY` at once.
/// `open` retries its pragmas through that, within the same budget, rather
/// than failing a connection that would succeed a millisecond later.
fn execute_through_busy(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    let mut pause = Duration::from_millis(1);
    loop {
        match conn.execute_batch(sql) {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == ErrorCode::DatabaseBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// How long a connection waits on another's lock before giving up; the same
/// default `rusqlite` uses.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// The pragmas `clipboard-db.cpp` applies on every connection.
const PRAGMAS: [&str; 4] = [
    "PRAGMA journal_mode = WAL",
    "PRAGMA synchronous = normal",
    "PRAGMA journal_size_limit = 6144000",
    "PRAGMA foreign_keys = ON",
];
