//! Launch history: how often an item was launched, and how recently.
//!
//! The score itself is [`compass_search::frecency`] — the port of the C++ `fuzzy::frecency` — so
//! the Rust and C++ engines agree on ordering. This module owns the *history*: recording launches
//! and persisting them.
//!
//! Two things are deliberately abstract:
//!
//! * [`FrecencyStore`] is a trait. The plan puts SQLite in `compass-core` eventually; until then a
//!   JSON file is enough and nothing outside this module should know which it is.
//! * [`Clock`] is injected. Decay is a function of wall-clock time, and a test that has to sleep
//!   for thirty days is not a test. [`ManualClock`] makes decay assertions instant and exact.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde::{Deserialize, Serialize};

/// Current on-disk format version of the JSON store.
pub const STORE_VERSION: u32 = 1;

/// Path of the frecency store relative to `$XDG_DATA_HOME`.
pub const STORE_RELATIVE_PATH: &str = "vicinae/frecency.json";

/// A source of wall-clock time, in unix seconds.
///
/// Implementations must be cheap: ranking calls [`Clock::now_unix`] once per query, not once per
/// item, but nothing in the contract stops a caller doing otherwise.
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// Seconds since the unix epoch.
    fn now_unix(&self) -> i64;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> i64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(delta) => delta.as_secs() as i64,
            // A clock set before 1970 is not worth a panic; treat it as the epoch.
            Err(_) => 0,
        }
    }
}

/// A clock a test drives by hand.
#[derive(Debug)]
pub struct ManualClock {
    now: AtomicI64,
}

impl ManualClock {
    /// A clock reading `now` unix seconds.
    #[must_use]
    pub fn new(now: i64) -> ManualClock {
        ManualClock {
            now: AtomicI64::new(now),
        }
    }

    /// Moves the clock to `now`.
    pub fn set(&self, now: i64) {
        self.now.store(now, Ordering::SeqCst);
    }

    /// Moves the clock forward by `seconds`.
    pub fn advance(&self, seconds: i64) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }

    /// Moves the clock forward by `days`.
    pub fn advance_days(&self, days: i64) {
        self.advance(days * 86_400);
    }
}

impl Clock for ManualClock {
    fn now_unix(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// What is remembered about one launchable item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrecencyRecord {
    /// How many times the item has been launched.
    #[serde(default)]
    pub launch_count: u32,
    /// When it was last launched, in unix seconds. `None` for an item that has been recorded but
    /// never launched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launched_at: Option<i64>,
}

impl FrecencyRecord {
    /// The frecency of this record at `now`, in `[0, 1]`.
    ///
    /// Delegates to [`compass_search::frecency`] so the score matches the C++ engine's.
    #[must_use]
    pub fn score_at(&self, now: i64) -> f64 {
        let last = self
            .last_launched_at
            .map(|t| u64::try_from(t).unwrap_or_default());
        compass_search::frecency(self.launch_count, last, now)
    }
}

/// Something that remembers launches.
///
/// Keys are opaque to the store; [`crate::AppItem::key`] supplies them for applications.
pub trait FrecencyStore {
    /// The store's notion of now, in unix seconds.
    fn now(&self) -> i64;

    /// The record for `key`, if the store has one.
    fn record(&self, key: &str) -> Option<FrecencyRecord>;

    /// Every record the store holds, in an unspecified order.
    fn records(&self) -> Vec<(String, FrecencyRecord)>;

    /// Records one launch of `key` at [`FrecencyStore::now`].
    ///
    /// # Errors
    ///
    /// Whatever persisting the change failed with.
    fn record_launch(&mut self, key: &str) -> Result<(), FrecencyError>;

    /// Forgets `key`. Returns whether anything was removed.
    ///
    /// # Errors
    ///
    /// Whatever persisting the change failed with.
    fn forget(&mut self, key: &str) -> Result<bool, FrecencyError>;

    /// Persists any pending change.
    ///
    /// # Errors
    ///
    /// Whatever persisting the change failed with.
    fn flush(&mut self) -> Result<(), FrecencyError>;

    /// The frecency of `key` at [`FrecencyStore::now`], in `[0, 1]`. `0` for an unknown key.
    fn score(&self, key: &str) -> f64 {
        match self.record(key) {
            Some(record) => record.score_at(self.now()),
            None => 0.0,
        }
    }
}

/// Everything that can go wrong reading or writing a frecency store.
#[derive(Debug, thiserror::Error)]
pub enum FrecencyError {
    /// The store file exists but could not be read.
    #[error("could not read the frecency store at {path}")]
    Read {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The store file could not be written.
    #[error("could not write the frecency store to {path}")]
    Write {
        /// The offending path.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The store file is not valid JSON, or is JSON of the wrong shape.
    #[error("invalid frecency store at {path}: line {line} column {column}: {message}")]
    Parse {
        /// The offending path.
        path: PathBuf,
        /// 1-based line of the problem, `0` when the parser could not place it.
        line: usize,
        /// 1-based column of the problem, `0` when the parser could not place it.
        column: usize,
        /// What the JSON parser objected to.
        message: String,
    },

    /// `$XDG_DATA_HOME` (and its `$HOME` fallback) could not be resolved.
    #[error("could not determine the user data directory")]
    NoDataDir,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, FrecencyRecord>,
}

/// A [`FrecencyStore`] backed by a JSON file.
///
/// The whole store is held in memory; it has one entry per item the user has ever launched, which
/// is a few hundred at most. Writes are whole-file and atomic (temp file plus rename), so an
/// interrupted write cannot corrupt the history.
#[derive(Debug)]
pub struct JsonFrecencyStore {
    path: Option<PathBuf>,
    clock: Arc<dyn Clock>,
    entries: BTreeMap<String, FrecencyRecord>,
    dirty: bool,
}

impl JsonFrecencyStore {
    /// Opens the store at `path`, or starts an empty one if the file does not exist.
    ///
    /// # Errors
    ///
    /// [`FrecencyError::Read`] or [`FrecencyError::Parse`].
    pub fn open(path: impl AsRef<Path>) -> Result<JsonFrecencyStore, FrecencyError> {
        JsonFrecencyStore::open_with_clock(path, Arc::new(SystemClock))
    }

    /// Opens the store at `path` with an injected clock.
    ///
    /// # Errors
    ///
    /// [`FrecencyError::Read`] or [`FrecencyError::Parse`].
    pub fn open_with_clock(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
    ) -> Result<JsonFrecencyStore, FrecencyError> {
        let path = path.as_ref().to_path_buf();
        let entries = match std::fs::read_to_string(&path) {
            Ok(data) if data.trim().is_empty() => BTreeMap::new(),
            Ok(data) => {
                let file: StoreFile =
                    serde_json::from_str(&data).map_err(|err| FrecencyError::Parse {
                        path: path.clone(),
                        line: err.line(),
                        column: err.column(),
                        message: err.to_string(),
                    })?;
                file.entries
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(source) => {
                return Err(FrecencyError::Read { path, source });
            }
        };

        Ok(JsonFrecencyStore {
            path: Some(path),
            clock,
            entries,
            dirty: false,
        })
    }

    /// A store that is never written to disk. Useful for tests and for a read-only session.
    #[must_use]
    pub fn in_memory(clock: Arc<dyn Clock>) -> JsonFrecencyStore {
        JsonFrecencyStore {
            path: None,
            clock,
            entries: BTreeMap::new(),
            dirty: false,
        }
    }

    /// The file this store persists to, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Number of remembered items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is remembered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Overwrites the record for `key`. Mostly for seeding tests and for migration.
    pub fn set_record(&mut self, key: impl Into<String>, record: FrecencyRecord) {
        self.entries.insert(key.into(), record);
        self.dirty = true;
    }

    fn persist(&mut self) -> Result<(), FrecencyError> {
        let Some(path) = self.path.clone() else {
            self.dirty = false;
            return Ok(());
        };

        let file = StoreFile {
            version: STORE_VERSION,
            entries: self.entries.clone(),
        };
        let data = serde_json::to_vec_pretty(&file).map_err(|err| FrecencyError::Write {
            path: path.clone(),
            source: std::io::Error::other(err),
        })?;

        crate::atomic_write(&path, &data)
            .map_err(|source| FrecencyError::Write { path, source })?;
        self.dirty = false;
        Ok(())
    }
}

impl FrecencyStore for JsonFrecencyStore {
    fn now(&self) -> i64 {
        self.clock.now_unix()
    }

    fn record(&self, key: &str) -> Option<FrecencyRecord> {
        self.entries.get(key).copied()
    }

    fn records(&self) -> Vec<(String, FrecencyRecord)> {
        self.entries
            .iter()
            .map(|(key, record)| (key.clone(), *record))
            .collect()
    }

    fn record_launch(&mut self, key: &str) -> Result<(), FrecencyError> {
        let now = self.clock.now_unix();
        let entry = self.entries.entry(key.to_owned()).or_default();
        entry.launch_count = entry.launch_count.saturating_add(1);
        entry.last_launched_at = Some(now);
        self.dirty = true;
        self.persist()
    }

    fn forget(&mut self, key: &str) -> Result<bool, FrecencyError> {
        let removed = self.entries.remove(key).is_some();
        if removed {
            self.dirty = true;
            self.persist()?;
        }
        Ok(removed)
    }

    fn flush(&mut self) -> Result<(), FrecencyError> {
        if self.dirty { self.persist() } else { Ok(()) }
    }
}

/// `$XDG_DATA_HOME/vicinae/frecency.json`, falling back to `~/.local/share`.
///
/// # Errors
///
/// [`FrecencyError::NoDataDir`] when neither `$XDG_DATA_HOME` nor `$HOME` is usable.
pub fn default_store_path() -> Result<PathBuf, FrecencyError> {
    let dir = dirs::data_dir().ok_or(FrecencyError::NoDataDir)?;
    Ok(dir.join(STORE_RELATIVE_PATH))
}
