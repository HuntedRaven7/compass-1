//! Clipboard history: Compass's own store, fed by the Shell extension.
//!
//! # What this owns, and what it does not
//!
//! The history lives in `compass-clipboard.db` next to Compass's file index,
//! not in Vicinae's `clipboard.db` (ADR-0017 decision 3: user data is imported,
//! not shared). It is encrypted at rest (decision 4) under a master key Compass
//! keeps in the login keyring under its own attributes, so it never reads or
//! rewrites Vicinae's key either.
//!
//! What gets recorded, and how, is `compass_clipboard::ingest`'s business; this
//! module only wires it to the two things around it: the keyring, and the
//! `ClipboardChanged` signal of the GNOME Shell extension. Without the
//! extension there is nothing to record on GNOME (Mutter has no data-control
//! protocol), and the store simply stays as it is. On a wlroots compositor the
//! selection is watched over data-control instead ([`crate::wlroots`]).
//!
//! # Why the key is behind a trait
//!
//! The one decision here that can lose a user's history is what happens when
//! the keyring already holds something. That logic is [`master_key`], and it is
//! tested against [`SecretStore`] fakes; the `oo7` implementation is a thin
//! adapter with nothing to decide.
//!
//! # Status and recovery
//!
//! What the service is doing is kept in [`Control`] and answered to
//! `compass doctor` as a [`ClipboardStatus`]: the backend chosen, whether it
//! delivers copies, and the history database's state. A keyring that is
//! locked or not running yet is waited for, with the same capped backoff as the
//! Shell extension, and without asking to unlock it again after the first
//! attempt. A database the key does not open, or a keyring entry that is not a
//! key, is not retried: only `compass clipboard reset` (`crate::clipboard_cmd`)
//! gets out of that, so the status says so. A data-control connection that
//! ends is reopened with the same backoff.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_clipboard::files::{self, DATABASE_FILE_NAME, PAYLOAD_DIR_NAME, ReadError, Survey};
use compass_clipboard::ingest::{self, Decision, Incoming};
use compass_clipboard::kind::OfferKind;
use compass_clipboard::store::{self, ListSettings};
use compass_crypto::KEY_SIZE;
use compass_ipc::{
    ClipboardBackend, ClipboardDetail, ClipboardEntry, ClipboardKind, ClipboardStatus,
    ClipboardStorage,
};
use compass_sqlcipher_sys::rusqlite::{Connection, named_params};
use tokio::sync::RwLock;

use crate::serve::EngineState;

/// Attributes identifying Compass's master key in the keyring.
///
/// Deliberately not Vicinae's (`compass_crypto::keyring`): a key Compass
/// created must never be one Vicinae could overwrite, or the reverse.
pub const KEYRING_ATTRIBUTES: [(&str, &str); 2] = [
    ("application", "compass"),
    ("purpose", "clipboard-master-key"),
];

/// The keyring entry's label, which is what a keyring manager shows.
pub const KEYRING_LABEL: &str = "Compass clipboard history key";

/// Why clipboard history is unavailable.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The keyring could not be reached or refused.
    #[error("keyring: {0}")]
    Keyring(String),
    /// The keyring holds an entry that is not a key Compass could have written.
    #[error(
        "the keyring entry for Compass's clipboard key is {0} bytes, not {KEY_SIZE}; \
         it was left untouched rather than replaced, since replacing it would make any \
         existing history unreadable"
    )]
    MalformedKey(usize),
    /// No randomness to make a key from.
    #[error("{0}")]
    Randomness(#[from] compass_crypto::NoRandomness),
    /// The database would not open or migrate.
    #[error("clipboard database: {0}")]
    Database(String),
    /// The database exists but the key does not open it: it was made under a
    /// key that is no longer in the keyring, or it is damaged.
    #[error(
        "the clipboard history cannot be read with the key in the keyring ({0}); \
         run `compass clipboard reset` to move it into a backup and start a new one"
    )]
    Unreadable(String),
    /// A query or write failed.
    #[error("clipboard store: {0}")]
    Store(String),
    /// No XDG data directory to keep the history in.
    #[error("no XDG data directory")]
    NoDataDir,
}

/// Where the master key is kept.
pub trait SecretStore {
    /// The stored secret, if there is one.
    fn lookup(&self) -> impl Future<Output = Result<Option<Vec<u8>>, Error>> + Send;
    /// Store `secret`, replacing nothing (it is only called after a miss).
    fn store(&self, secret: &[u8]) -> impl Future<Output = Result<(), Error>> + Send;
    /// Whether the keyring is locked, so that storing would ask the user to
    /// unlock it. `false` when that cannot be told.
    fn locked(&self) -> impl Future<Output = bool> + Send;
}

/// Held while the master key is looked up and, on a miss, made and stored.
///
/// The engine asks for the key from several tasks at once (clipboard history
/// as it starts, and extension storage for each command, the calculator and
/// Rhai scripts), each with its own connection to the keyring. Two that both
/// miss would each store a key of their own: the keyring then holds two
/// entries, and which one a later lookup finds decides whether a database made
/// under the other opens at all.
static KEY_LOOKUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Returns the master key, creating and storing one on first use.
///
/// An entry of the wrong length is an error and is **not** replaced: the only
/// way it exists is that something else wrote it, and overwriting it could
/// orphan a history encrypted under it. Nor is a key created while the
/// keyring is locked: storing one would ask the user to unlock it, and a
/// retry loop must not do that again and again. Callers in this process are
/// served one at a time, so a first run makes exactly one key.
///
/// # Errors
///
/// [`Error::Keyring`] when the store fails or is locked, [`Error::MalformedKey`]
/// as above, [`Error::Randomness`] when no key can be generated.
pub async fn master_key<S: SecretStore>(store: &S) -> Result<[u8; KEY_SIZE], Error> {
    let _one_at_a_time = KEY_LOOKUP.lock().await;
    if let Some(existing) = store.lookup().await? {
        return <[u8; KEY_SIZE]>::try_from(existing.as_slice())
            .map_err(|_| Error::MalformedKey(existing.len()));
    }
    if store.locked().await {
        return Err(Error::Keyring("the keyring is locked".to_owned()));
    }
    let key = compass_crypto::generate_key()?;
    store.store(&key).await?;
    Ok(key)
}

/// The login keyring, through `oo7`: the Secret Service on the host, the
/// Secret portal inside a Flatpak.
pub struct Oo7Store {
    keyring: oo7::Keyring,
}

impl std::fmt::Debug for Oo7Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Oo7Store").finish_non_exhaustive()
    }
}

impl Oo7Store {
    /// Connects to whichever keyring this process can reach, asking the user
    /// to unlock it if it is locked.
    ///
    /// # Errors
    ///
    /// [`Error::Keyring`] when neither backend is available.
    pub async fn connect() -> Result<Self, Error> {
        let store = Self::connect_quietly().await?;
        store
            .keyring
            .unlock()
            .await
            .map_err(|err| Error::Keyring(err.to_string()))?;
        Ok(store)
    }

    /// Connects without asking to unlock anything: for retries, and for
    /// diagnostics that only look.
    ///
    /// # Errors
    ///
    /// [`Error::Keyring`] when neither backend is available.
    pub async fn connect_quietly() -> Result<Self, Error> {
        let keyring = oo7::Keyring::new()
            .await
            .map_err(|err| Error::Keyring(err.to_string()))?;
        Ok(Self { keyring })
    }

    /// Removes Compass's clipboard key from the keyring, and nothing else:
    /// only the entries with [`KEYRING_ATTRIBUTES`].
    ///
    /// # Errors
    ///
    /// [`Error::Keyring`] when the keyring refuses.
    pub async fn delete_key(&self) -> Result<(), Error> {
        self.keyring
            .delete(&Self::attributes())
            .await
            .map_err(|err| Error::Keyring(err.to_string()))
    }

    fn attributes() -> std::collections::HashMap<&'static str, &'static str> {
        KEYRING_ATTRIBUTES.into_iter().collect()
    }

    /// Vicinae's own master key, read only, for the import. `None` when it
    /// has none or the entry is not one (logged; the import then reads only
    /// what is unencrypted).
    pub async fn vicinae_master_key(&self) -> Option<[u8; KEY_SIZE]> {
        let attributes: std::collections::HashMap<_, _> =
            compass_crypto::keyring::master_key_attributes()
                .into_iter()
                .collect();
        let item = self
            .keyring
            .search_items(&attributes)
            .await
            .ok()?
            .into_iter()
            .next()?;
        let secret = item.secret().await.ok()?;
        let text = std::str::from_utf8(secret.as_bytes()).ok()?;
        match compass_crypto::keyring::master_key_from_secret(text) {
            Ok(key) => Some(key),
            Err(err) => {
                tracing::warn!(error = %err, "Vicinae's keyring entry is not a key; not used");
                None
            }
        }
    }
}

impl SecretStore for Oo7Store {
    async fn lookup(&self) -> Result<Option<Vec<u8>>, Error> {
        let items = self
            .keyring
            .search_items(&Self::attributes())
            .await
            .map_err(|err| Error::Keyring(err.to_string()))?;
        let Some(item) = items.first() else {
            return Ok(None);
        };
        let secret = item
            .secret()
            .await
            .map_err(|err| Error::Keyring(err.to_string()))?;
        Ok(Some(secret.as_bytes().to_vec()))
    }

    async fn store(&self, secret: &[u8]) -> Result<(), Error> {
        self.keyring
            .create_item(
                KEYRING_LABEL,
                &Self::attributes(),
                oo7::Secret::blob(secret),
                false,
            )
            .await
            .map_err(|err| Error::Keyring(err.to_string()))
    }

    async fn locked(&self) -> bool {
        // Inside a sandbox oo7 keeps secrets in a file unlocked through the
        // Secret portal, which has no lock; only the host's Secret Service
        // collection can be locked.
        if oo7::ashpd::is_sandboxed().await {
            return false;
        }
        let Ok(service) = oo7::dbus::Service::new().await else {
            return false;
        };
        let Ok(collection) = service.default_collection().await else {
            return false;
        };
        collection.is_locked().await.unwrap_or(false)
    }
}

/// What the keyring holds for clipboard history, found without asking to
/// unlock anything or writing anything.
#[derive(Clone, PartialEq, Eq)]
pub enum KeyState {
    /// A key of the right size.
    Found([u8; KEY_SIZE]),
    /// No entry.
    Missing,
    /// An entry that is not a key Compass wrote, by its length.
    Malformed(usize),
    /// The keyring is locked, so the entry could not be read.
    Locked,
    /// No keyring answered.
    Unavailable(String),
}

impl std::fmt::Debug for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key itself: this ends up in logs and test output.
        match self {
            Self::Found(_) => f.write_str("Found(..)"),
            Self::Missing => f.write_str("Missing"),
            Self::Malformed(len) => f.debug_tuple("Malformed").field(len).finish(),
            Self::Locked => f.write_str("Locked"),
            Self::Unavailable(reason) => f.debug_tuple("Unavailable").field(reason).finish(),
        }
    }
}

/// Reads the key from `store` as [`master_key`] would, but makes none.
pub async fn key_state<S: SecretStore>(store: &S) -> KeyState {
    match store.lookup().await {
        Ok(Some(bytes)) => <[u8; KEY_SIZE]>::try_from(bytes.as_slice())
            .map_or(KeyState::Malformed(bytes.len()), KeyState::Found),
        // A locked keyring can hide the entry as well as refuse to read it.
        Ok(None) | Err(_) if store.locked().await => KeyState::Locked,
        Ok(None) => KeyState::Missing,
        Err(err) => KeyState::Unavailable(err.to_string()),
    }
}

/// How long the keyring has to answer a diagnostic: a Secret Service that
/// D-Bus activates but that never starts would otherwise hold it for the bus's
/// own, much longer, timeout.
const KEYRING_PATIENCE: Duration = Duration::from_secs(5);

/// The history on disk and its key, as `compass doctor` and
/// `compass clipboard reset` see them.
#[derive(Debug)]
pub struct Inspection {
    /// Where the history lives.
    pub dir: PathBuf,
    /// What is there, or why it could not be listed.
    pub survey: Result<Survey, String>,
    /// What the keyring holds.
    pub key: KeyState,
    /// The entry count, when there is a database and a key to try it with.
    pub read: Option<Result<u64, ReadError>>,
}

/// Looks at the history and its key without creating, migrating or
/// unlocking anything.
///
/// # Errors
///
/// [`Error::NoDataDir`] when there is nowhere the history could be.
pub async fn inspect() -> Result<Inspection, Error> {
    let dir = data_dir()?;
    let key = tokio::time::timeout(KEYRING_PATIENCE, async {
        match Oo7Store::connect_quietly().await {
            Ok(store) => key_state(&store).await,
            Err(err) => KeyState::Unavailable(err.to_string()),
        }
    })
    .await
    .unwrap_or_else(|_| KeyState::Unavailable("the keyring did not answer in time".to_owned()));
    let master = match &key {
        KeyState::Found(master) => Some(*master),
        _ => None,
    };
    let surveyed = dir.clone();
    let (survey, read) =
        tokio::task::spawn_blocking(move || inspect_files(&surveyed, master.as_ref()))
            .await
            .map_err(|err| Error::Store(err.to_string()))?;
    Ok(Inspection {
        dir,
        survey,
        key,
        read,
    })
}

/// The files half of [`inspect`]: what is in `dir`, and how many entries
/// `master` reads from it. Blocking.
pub fn inspect_files(
    dir: &Path,
    master: Option<&[u8; KEY_SIZE]>,
) -> (Result<Survey, String>, Option<Result<u64, ReadError>>) {
    let survey = files::survey(dir).map_err(|err| err.to_string());
    let read = match (&survey, master) {
        (Ok(survey), Some(master)) if survey.part(files::PartKind::Database).is_some() => {
            Some(files::count_entries(survey, master))
        }
        _ => None,
    };
    (survey, read)
}

/// The open history.
///
/// The connection is used from blocking tasks, one at a time, so a plain `Mutex`
/// is enough; callers on the async runtime go through `spawn_blocking`.
pub struct ClipboardStore {
    db: Mutex<Connection>,
    payload_dir: PathBuf,
    payload_key: [u8; KEY_SIZE],
}

impl std::fmt::Debug for ClipboardStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardStore")
            .field("payload_dir", &self.payload_dir)
            .finish_non_exhaustive()
    }
}

impl ClipboardStore {
    /// Opens (creating if needed) the history in `dir`, keyed from `master`.
    ///
    /// # Errors
    ///
    /// [`Error::Unreadable`] when the file exists and SQLCipher refuses the
    /// key, [`Error::Database`] when it cannot be opened otherwise or will not
    /// migrate.
    pub fn open(dir: &Path, master: &[u8; KEY_SIZE]) -> Result<Self, Error> {
        std::fs::create_dir_all(dir).map_err(|err| Error::Database(err.to_string()))?;
        let keys = compass_crypto::keys::derive_all(master);
        let path = dir.join(DATABASE_FILE_NAME);
        let existed = path.exists();
        let db = compass_sqlcipher_sys::open(&path, &keys.database).map_err(|err| {
            match err.sqlite_error_code() {
                Some(compass_sqlcipher_sys::rusqlite::ErrorCode::NotADatabase) if existed => {
                    Error::Unreadable(err.to_string())
                }
                _ => Error::Database(err.to_string()),
            }
        })?;
        compass_clipboard::schema::run(&db).map_err(|err| Error::Database(err.to_string()))?;
        Ok(Self {
            db: Mutex::new(db),
            payload_dir: dir.join(PAYLOAD_DIR_NAME),
            payload_key: keys.clipboard,
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves the database itself intact
        // (every write is a transaction), so a poisoned lock is still usable.
        self.db
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records one copy.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when `ingest` refuses it.
    pub fn record(
        &self,
        data: &[u8],
        mime_type: &str,
        source_app: Option<&str>,
    ) -> Result<Decision, Error> {
        std::fs::create_dir_all(&self.payload_dir).map_err(|err| Error::Store(err.to_string()))?;
        let copy = Incoming {
            data,
            mime_type,
            source_app,
        };
        ingest::ingest(
            &self.db(),
            &self.payload_dir,
            &copy,
            Some(&self.payload_key),
            &mut new_id,
        )
        .map_err(|err| Error::Store(err.to_string()))
    }

    /// One entry's content, decrypted: `(mime_type, bytes)`, or `None` when no
    /// entry has that id.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the lookup fails, the payload file is missing, or
    /// it will not decrypt.
    pub fn content(&self, id: &str) -> Result<Option<(String, Vec<u8>)>, Error> {
        let Some(offer) = compass_clipboard::write::find_preferred_offer(&self.db(), id)
            .map_err(|err| Error::Store(err.to_string()))?
        else {
            return Ok(None);
        };
        let path = ingest::payload_path(&self.payload_dir, &offer.id);
        let stored = std::fs::read(&path)
            .map_err(|err| Error::Store(format!("reading {}: {err}", path.display())))?;
        let data = match offer.encryption {
            compass_clipboard::kind::EncryptionType::None => stored,
            compass_clipboard::kind::EncryptionType::Local => {
                compass_crypto::decrypt(&stored, &self.payload_key)
                    .map_err(|err| Error::Store(format!("decrypting {}: {err}", offer.id)))?
            }
        };
        Ok(Some((offer.mime_type, data)))
    }

    /// Records one entry from another history with its own times, pin and
    /// keywords. `false` when the same content is already here, which is
    /// left exactly as it is: going through [`Self::record`] alone would
    /// bubble it to now.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the lookup, the insert or the metadata update
    /// fails.
    pub fn import(&self, entry: &crate::vicinae_import::VicinaeEntry) -> Result<bool, Error> {
        let hash = ingest::content_hash(&entry.data);
        {
            let db = self.db();
            let known = db
                .prepare_cached("SELECT 1 FROM selection WHERE hash_md5 = :hash LIMIT 1")
                .and_then(|mut stmt| stmt.exists(named_params! { ":hash": hash }))
                .map_err(|err| Error::Store(err.to_string()))?;
            if known {
                return Ok(false);
            }
        }
        let Decision::Inserted { selection_id, .. } =
            self.record(&entry.data, &entry.mime_type, entry.source.as_deref())?
        else {
            // Ignored: empty, blank or of no kind history keeps.
            return Ok(false);
        };
        let db = self.db();
        db.execute(
            "UPDATE selection SET created_at = :created, updated_at = :updated, \
             pinned_at = :pinned WHERE id = :id",
            named_params! {
                ":created": entry.created_at,
                ":updated": entry.updated_at,
                ":pinned": entry.pinned_at,
                ":id": selection_id,
            },
        )
        .map_err(|err| Error::Store(err.to_string()))?;
        if !entry.keywords.is_empty() {
            compass_clipboard::write::set_keywords(&db, &selection_id, &entry.keywords)
                .map_err(|err| Error::Store(err.to_string()))?;
        }
        Ok(true)
    }

    /// Pins or unpins an entry; `false` when no entry has that id.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the update fails.
    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<bool, Error> {
        compass_clipboard::write::set_pinned(&self.db(), id, pinned)
            .map_err(|err| Error::Store(err.to_string()))
    }

    /// Removes an entry and unlinks its payloads; `false` when no entry has
    /// that id.
    ///
    /// The rows go first, in one transaction. A payload that then fails to
    /// unlink is logged and left: it is encrypted, and unreferenced once its
    /// row is gone, where an unlink-first order could leave a row pointing at
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the delete fails.
    pub fn remove(&self, id: &str) -> Result<bool, Error> {
        let removed = compass_clipboard::write::remove_selection(&self.db(), id)
            .map_err(|err| Error::Store(err.to_string()))?;
        for offer in &removed {
            let path = ingest::payload_path(&self.payload_dir, offer);
            if let Err(err) = std::fs::remove_file(&path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), %err, "could not unlink a removed payload");
            }
        }
        Ok(!removed.is_empty())
    }

    /// Pinned entries first, then newest first; `query` filters, empty lists.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the query fails; a zero `limit` is one.
    pub fn history(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>, Error> {
        self.history_of_kind(query, limit, None)
    }

    /// [`Self::history`] restricted to one kind, the view's filter.
    ///
    /// # Errors
    ///
    /// As [`Self::history`].
    pub fn history_of_kind(
        &self,
        query: &str,
        limit: u32,
        kind: Option<ClipboardKind>,
    ) -> Result<Vec<ClipboardEntry>, Error> {
        let settings = ListSettings {
            query: query.to_owned(),
            kind: kind.map(kind_in_the_store),
        };
        let page = store::query(&self.db(), i64::from(limit), 0, &settings)
            .map_err(|err| Error::Store(err.to_string()))?;
        Ok(page
            .data
            .into_iter()
            .map(|entry| ClipboardEntry {
                id: entry.id,
                preview: entry.text_preview,
                mime_type: entry.mime_type,
                kind: kind_on_the_wire(entry.kind),
                pinned: entry.pinned_at != 0,
                updated_at: entry.updated_at,
                url_host: entry.url_host,
            })
            .collect())
    }
}

impl ClipboardStore {
    /// What the detail pane shows about `id`; `None` when no entry has it.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the lookup fails.
    pub fn detail(&self, id: &str) -> Result<Option<ClipboardDetail>, Error> {
        let entry = store::entry(&self.db(), id).map_err(|err| Error::Store(err.to_string()))?;
        Ok(entry.map(|entry| ClipboardDetail {
            id: entry.id,
            mime_type: entry.mime_type,
            kind: kind_on_the_wire(entry.kind),
            size: entry.size,
            md5: entry.md5sum,
            updated_at: entry.updated_at,
            encrypted: entry.encryption != compass_clipboard::kind::EncryptionType::None,
            keywords: entry.keywords,
            pinned: entry.pinned_at != 0,
        }))
    }

    /// Sets the words `id` is also found by; `false` when no entry has it.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the update fails.
    pub fn set_keywords(&self, id: &str, keywords: &str) -> Result<bool, Error> {
        compass_clipboard::write::set_keywords(&self.db(), id, keywords.trim())
            .map_err(|err| Error::Store(err.to_string()))
    }

    /// Removes every entry — every one but the pinned and keyworded when
    /// `preserve_tagged` — and unlinks their payloads, as
    /// `removeAllSelections` does. Returns how many payloads went.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the delete fails.
    pub fn remove_all(&self, preserve_tagged: bool) -> Result<usize, Error> {
        let removed = compass_clipboard::write::remove_all(&self.db(), preserve_tagged)
            .map_err(|err| Error::Store(err.to_string()))?;
        Ok(self.unlink(&removed))
    }

    /// One eviction pass (`runEvictionPass`): removes what was last copied
    /// longer than `threshold` ago and unlinks its payloads, then reports
    /// how many went and the oldest `updated_at` still evictable, for the
    /// next pass to be timed from.
    ///
    /// # Errors
    ///
    /// [`Error::Store`] when the sweep or the lookup fails.
    pub fn evict(
        &self,
        threshold: Duration,
        preserve_tagged: bool,
    ) -> Result<(usize, Option<i64>), Error> {
        let evicted =
            compass_clipboard::write::evict_older_than(&self.db(), threshold, preserve_tagged)
                .map_err(|err| Error::Store(err.to_string()))?;
        let count = self.unlink(&evicted);
        let oldest = compass_clipboard::write::oldest_evictable(&self.db(), preserve_tagged)
            .map_err(|err| Error::Store(err.to_string()))?;
        Ok((count, oldest))
    }

    /// Unlinks the payloads of `offers`, logging any that will not go.
    fn unlink(&self, offers: &[String]) -> usize {
        offers
            .iter()
            .filter(|offer| {
                let path = ingest::payload_path(&self.payload_dir, offer);
                match std::fs::remove_file(&path) {
                    Ok(()) => true,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                    Err(err) => {
                        tracing::warn!(path = %path.display(), %err, "could not unlink a payload");
                        false
                    }
                }
            })
            .count()
    }
}

/// The clipboard extension's id, which its preferences are kept under
/// (`providers.clipboard.preferences`).
pub const PROVIDER_ID: &str = "clipboard";

/// The clipboard extension's preferences, as `preferenceValuesChanged` reads
/// them, with its defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `monitoring`: whether copies are recorded. Default on.
    pub monitoring: bool,
    /// `ignorePasswords`: whether a copy a password manager marked is left
    /// out. Default on.
    pub ignore_passwords: bool,
    /// `preserveTagged`: whether eviction and remove-all spare pinned and
    /// keyworded entries. Default on.
    pub preserve_tagged: bool,
    /// `evictionThreshold`: how long history is kept; `None` is for ever,
    /// the default.
    pub eviction: Option<Duration>,
    /// `eraseOnStartup`: whether the history is cleared when the engine
    /// starts. Default off.
    pub erase_on_startup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_preferences(None)
    }
}

impl Settings {
    /// Reads the preferences object, each missing or mistyped value taking
    /// its default.
    #[must_use]
    pub fn from_preferences(
        preferences: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        let flag = |key: &str, default: bool| {
            preferences
                .and_then(|values| values.get(key))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default)
        };
        Self {
            monitoring: flag("monitoring", true),
            ignore_passwords: flag("ignorePasswords", true),
            preserve_tagged: flag("preserveTagged", true),
            eviction: compass_clipboard::retention::parse_threshold(
                preferences
                    .and_then(|values| values.get("evictionThreshold"))
                    .and_then(serde_json::Value::as_str),
            ),
            erase_on_startup: flag("eraseOnStartup", false),
        }
    }
}

/// What the running service is told after it starts: the monitoring switch
/// and the two preferences the history view acts on, shared between the
/// recording loop, the eviction timer and the requests.
#[derive(Debug, Default)]
pub struct Control {
    monitoring: std::sync::atomic::AtomicBool,
    supported: std::sync::atomic::AtomicBool,
    ignore_passwords: std::sync::atomic::AtomicBool,
    preserve_tagged: std::sync::atomic::AtomicBool,
    /// Woken after each copy is recorded, so an idle eviction timer re-arms
    /// (`armEvictionTimer(now)` in the C++'s insert handler).
    inserted: tokio::sync::Notify,
    /// What `compass doctor` is told.
    health: Mutex<Health>,
}

/// The service's state beyond the switches, as [`ClipboardStatus`] reports it.
#[derive(Debug, Default)]
struct Health {
    backend: Option<ClipboardBackend>,
    capture_problem: Option<String>,
    /// `None` while the history is still being opened.
    storage: Option<ClipboardStorage>,
}

impl Control {
    /// A control holding `settings`, not yet recording anything.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let control = Self::default();
        control.apply(settings);
        control
    }

    /// Takes the preferences in.
    pub fn apply(&self, settings: &Settings) {
        use std::sync::atomic::Ordering::Relaxed;
        self.monitoring.store(settings.monitoring, Relaxed);
        self.ignore_passwords
            .store(settings.ignore_passwords, Relaxed);
        self.preserve_tagged
            .store(settings.preserve_tagged, Relaxed);
    }

    /// Whether copies are being recorded.
    #[must_use]
    pub fn monitoring(&self) -> bool {
        self.monitoring.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turns recording on or off (`setMonitoring`).
    pub fn set_monitoring(&self, enabled: bool) {
        self.monitoring
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether something is watching the selection at all
    /// (`supportsMonitoring`): false until a watcher has started.
    #[must_use]
    pub fn supported(&self) -> bool {
        self.supported.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_supported(&self, supported: bool) {
        self.supported
            .store(supported, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether a copy a password manager marked is left out.
    #[must_use]
    pub fn ignore_passwords(&self) -> bool {
        self.ignore_passwords
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether eviction and remove-all spare tagged entries.
    #[must_use]
    pub fn preserve_tagged(&self) -> bool {
        self.preserve_tagged
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// What the service is doing, for `compass doctor`.
    #[must_use]
    pub fn status(&self) -> ClipboardStatus {
        let health = self.health();
        ClipboardStatus {
            backend: health.backend.clone(),
            capturing: self.supported(),
            monitoring: self.monitoring(),
            capture_problem: health.capture_problem.clone(),
            storage: health.storage.clone().unwrap_or(ClipboardStorage::Opening),
        }
    }

    fn health(&self) -> std::sync::MutexGuard<'_, Health> {
        // Plain assignments under the lock cannot leave it half-written.
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_storage(&self, storage: ClipboardStorage) {
        self.health().storage = Some(storage);
    }

    /// Notes the backend in use and why it is not delivering copies, if it
    /// is not.
    fn set_capture(&self, backend: ClipboardBackend, problem: Option<String>) {
        let mut health = self.health();
        health.backend = Some(backend);
        health.capture_problem = problem;
    }
}

/// Records one copy if monitoring is on, waking the eviction timer when it
/// went in. `None` when monitoring is off and nothing was tried.
fn record_if_monitoring(
    store: &ClipboardStore,
    control: &Control,
    data: &[u8],
    mime_type: &str,
    source_app: Option<&str>,
) -> Option<Result<Decision, Error>> {
    if !control.monitoring() {
        return None;
    }
    let recorded = store.record(data, mime_type, source_app);
    if matches!(recorded, Ok(Decision::Inserted { .. })) {
        control.inserted.notify_one();
    }
    Some(recorded)
}

/// Keeps history within `threshold` for the life of the engine: a first pass
/// after [`compass_clipboard::retention::MISCONFIGURATION_GRACE`], then one
/// each time the oldest evictable entry comes due, and — when nothing is
/// evictable — one a threshold after the next copy.
pub async fn run_eviction(store: Arc<ClipboardStore>, control: Arc<Control>, threshold: Duration) {
    use compass_clipboard::retention;
    tokio::time::sleep(retention::MISCONFIGURATION_GRACE).await;
    loop {
        let pass_store = Arc::clone(&store);
        let preserve = control.preserve_tagged();
        let pass = tokio::task::spawn_blocking(move || pass_store.evict(threshold, preserve)).await;
        let oldest = match pass {
            Ok(Ok((evicted, oldest))) => {
                if evicted > 0 {
                    tracing::info!(evicted, "evicted clipboard offers");
                }
                oldest
            }
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "clipboard eviction failed");
                None
            }
            Err(err) => {
                tracing::warn!(error = %err, "the clipboard eviction task failed");
                None
            }
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        let delay = match oldest {
            Some(oldest) => retention::next_delay(oldest, threshold, now),
            None => {
                control.inserted.notified().await;
                retention::next_delay(now, threshold, now)
            }
        };
        tokio::time::sleep(delay).await;
    }
}

fn kind_in_the_store(kind: ClipboardKind) -> OfferKind {
    match kind {
        ClipboardKind::Text => OfferKind::Text,
        ClipboardKind::Link => OfferKind::Link,
        ClipboardKind::Image => OfferKind::Image,
        ClipboardKind::File => OfferKind::File,
        ClipboardKind::Unknown => OfferKind::Unknown,
    }
}

fn kind_on_the_wire(kind: OfferKind) -> ClipboardKind {
    match kind {
        OfferKind::Text => ClipboardKind::Text,
        OfferKind::Link => ClipboardKind::Link,
        OfferKind::Image => ClipboardKind::Image,
        OfferKind::File => ClipboardKind::File,
        OfferKind::Unknown | OfferKind::Count => ClipboardKind::Unknown,
    }
}

/// A fresh random id: 16 bytes of the system CSPRNG, as hex.
fn new_id() -> String {
    let mut bytes = [0u8; 16];
    // An id only has to be unique; if the CSPRNG is gone, the key could not
    // have been made either, and the store would not be open.
    getrandom::fill(&mut bytes).expect("the system random number generator is unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the history lives: `$XDG_DATA_HOME/compass`, beside the file index.
fn data_dir() -> Result<PathBuf, Error> {
    compass_core::xdg_dirs::data_home()
        .map(|home| home.join("compass"))
        .ok_or(Error::NoDataDir)
}

/// How long to wait before asking for the extension, the keyring or the
/// compositor again; the wait doubles up to [`RETRY_MAX`].
const RETRY_INITIAL: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Opens the history and records copies for the life of the engine.
///
/// Spawned, never awaited: a keyring that is slow to answer or absent must
/// not hold up the socket. A keyring that is locked or absent is waited for;
/// any other failure is logged once and leaves clipboard history unavailable
/// (the request then says so, as does [`Control::status`]) rather than
/// stopping the engine.
pub async fn run(state: Arc<RwLock<EngineState>>) {
    // Read when the service starts, as `initialized` and
    // `preferenceValuesChanged` read them.
    let settings = Settings::from_preferences(
        compass_core::Config::load()
            .unwrap_or_default()
            .provider_preferences(PROVIDER_ID),
    );
    let control = state.read().await.clipboard_control();
    control.apply(&settings);
    let Some((store, dir, keyring)) = open_until_ready(&control).await else {
        return;
    };
    let store = Arc::new(store);
    tracing::info!("clipboard history open");
    control.set_storage(ClipboardStorage::Open);
    state.write().await.set_clipboard(Arc::clone(&store));
    import_from_vicinae(Arc::clone(&store), dir, &keyring).await;
    if settings.erase_on_startup {
        let erased = Arc::clone(&store);
        let preserve = settings.preserve_tagged;
        match tokio::task::spawn_blocking(move || erased.remove_all(preserve)).await {
            Ok(Ok(count)) => tracing::info!(count, "erased clipboard history on startup"),
            Ok(Err(err)) => tracing::warn!(error = %err, "could not erase clipboard history"),
            Err(err) => tracing::warn!(error = %err, "the erase task failed"),
        }
    }
    if let Some(threshold) = settings.eviction {
        tokio::spawn(run_eviction(
            Arc::clone(&store),
            Arc::clone(&control),
            threshold,
        ));
    }
    match crate::wlroots::detect().await {
        Some(wlroots) if wlroots.capabilities.data_control => {
            record_from_data_control(store, control).await;
        }
        _ => record_from_shell(store, control).await,
    }
}

/// Opens the history, waiting out a locked or absent keyring.
///
/// Only the first attempt may ask the user to unlock the keyring; the retries
/// look quietly, so a dismissed prompt is not shown again every minute. `None`
/// once a failure no retry can fix has been recorded in `control`.
async fn open_until_ready(control: &Control) -> Option<(ClipboardStore, PathBuf, Oo7Store)> {
    let mut delay = RETRY_INITIAL;
    let mut reported: Option<String> = None;
    let mut ask_to_unlock = true;
    loop {
        match open_from_environment(ask_to_unlock).await {
            Ok(opened) => return Some(opened),
            Err(err) => {
                let storage = storage_after(&err);
                if matches!(storage, ClipboardStorage::Failed { .. }) {
                    tracing::warn!(error = %err, "clipboard history unavailable");
                    control.set_storage(storage);
                    return None;
                }
                // Said once per reason, as the Shell extension's absence is.
                let reason = err.to_string();
                if reported.as_deref() != Some(reason.as_str()) {
                    tracing::info!(error = %err, "clipboard history waits for the keyring");
                    reported = Some(reason);
                }
                control.set_storage(storage);
            }
        }
        ask_to_unlock = false;
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RETRY_MAX);
    }
}

/// What a failure to open the history means for its status: a keyring that
/// may yet answer is waited for; the rest need someone to act, and two of
/// them are what `compass clipboard reset` is for.
fn storage_after(err: &Error) -> ClipboardStorage {
    match err {
        Error::Keyring(reason) => ClipboardStorage::Waiting {
            reason: reason.clone(),
        },
        Error::MalformedKey(_) | Error::Unreadable(_) => ClipboardStorage::Failed {
            reason: err.to_string(),
            reset: true,
        },
        Error::Randomness(_) | Error::Database(_) | Error::Store(_) | Error::NoDataDir => {
            ClipboardStorage::Failed {
                reason: err.to_string(),
                reset: false,
            }
        }
    }
}

/// Follows the selection over `ext-data-control-v1` / `wlr-data-control` —
/// the wlroots path, where there is no Shell extension and none is needed —
/// reconnecting with backoff when the compositor cannot be reached or stops.
///
/// A selection a password manager marked (`x-kde-passwordManagerHint`, or
/// our own `vicinae/concealed`) is not recorded while `ignorePasswords` is
/// on, and nothing is recorded while monitoring is off.
async fn record_from_data_control(store: Arc<ClipboardStore>, control: Arc<Control>) {
    let mut delay = RETRY_INITIAL;
    let mut reported_absent = false;
    let mut backend = ClipboardBackend::DataControl { protocol: None };
    loop {
        let (tx, changes) = tokio::sync::mpsc::unbounded_channel();
        let watcher =
            tokio::task::spawn_blocking(move || compass_wayland::clipboard::watch(tx)).await;
        let problem = match watcher {
            Ok(Ok(watcher)) => {
                tracing::info!(
                    protocol = watcher.protocol(),
                    "recording clipboard changes over data-control"
                );
                backend = ClipboardBackend::DataControl {
                    protocol: Some(watcher.protocol().to_owned()),
                };
                control.set_capture(backend.clone(), None);
                control.set_supported(true);
                reported_absent = false;
                delay = RETRY_INITIAL;
                record_data_control_changes(&store, &control, changes).await;
                control.set_supported(false);
                tracing::info!("the compositor stopped sending clipboard changes; will retry");
                "the compositor stopped sending clipboard changes".to_owned()
            }
            Ok(Err(err)) => {
                if !reported_absent {
                    tracing::warn!(error = %err, "clipboard changes unavailable; will retry");
                    reported_absent = true;
                }
                err.to_string()
            }
            Err(err) => {
                tracing::warn!(error = %err, "the data-control task failed");
                err.to_string()
            }
        };
        control.set_capture(backend.clone(), Some(problem));
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RETRY_MAX);
    }
}

/// Records what one data-control connection delivers until it ends.
async fn record_data_control_changes(
    store: &Arc<ClipboardStore>,
    control: &Arc<Control>,
    mut changes: tokio::sync::mpsc::UnboundedReceiver<compass_wayland::clipboard::SelectionChange>,
) {
    while let Some(change) = changes.recv().await {
        if change.concealed() && control.ignore_passwords() {
            tracing::debug!("a concealed selection was not recorded");
            continue;
        }
        let Some(offer) = change.preferred().cloned() else {
            continue;
        };
        let store = Arc::clone(store);
        let control = Arc::clone(control);
        let recorded = tokio::task::spawn_blocking(move || {
            record_if_monitoring(&store, &control, &offer.data, &offer.mime_type, None)
        })
        .await;
        match recorded {
            Ok(Some(Ok(decision))) => tracing::debug!(?decision, "clipboard change"),
            Ok(None) => tracing::debug!("monitoring is off; a copy was not recorded"),
            Ok(Some(Err(err))) => tracing::warn!(error = %err, "clipboard change not recorded"),
            Err(err) => tracing::warn!(error = %err, "clipboard recording task failed"),
        }
    }
}

/// Connects to the keyring — asking to unlock it only when `ask_to_unlock` —
/// takes or makes the key, and opens the history.
async fn open_from_environment(
    ask_to_unlock: bool,
) -> Result<(ClipboardStore, PathBuf, Oo7Store), Error> {
    let dir = data_dir()?;
    let keyring = if ask_to_unlock {
        Oo7Store::connect().await?
    } else {
        Oo7Store::connect_quietly().await?
    };
    let key = master_key(&keyring).await?;
    let opened = dir.clone();
    let store = tokio::task::spawn_blocking(move || ClipboardStore::open(&opened, &key))
        .await
        .map_err(|err| Error::Database(err.to_string()))??;
    Ok((store, dir, keyring))
}

/// Brings Vicinae's history across on the first start that can read it.
///
/// After the store is published, so a long import never makes history look
/// unavailable, and before copies are recorded, so the import is the only
/// writer while it runs. Vicinae keeps its files in the same data directory; its
/// key is its own keyring entry, which may be absent (Vicinae never ran, or
/// ran without a keyring) and is then simply not used.
async fn import_from_vicinae(store: Arc<ClipboardStore>, dir: PathBuf, keyring: &Oo7Store) {
    if crate::vicinae_import::marker_path(&dir).exists() {
        return;
    }
    let master = keyring.vicinae_master_key().await;
    let imported = tokio::task::spawn_blocking(move || {
        crate::vicinae_import::import_once(&store, &dir, &dir, master.as_ref())
    })
    .await;
    match imported {
        Ok(Ok(Some(report))) if report != crate::vicinae_import::ImportReport::default() => {
            tracing::info!(
                imported = report.imported,
                already_present = report.already_present,
                skipped = report.skipped,
                "imported Vicinae's clipboard history"
            );
        }
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "Vicinae's clipboard history was not imported; will retry");
        }
        Err(err) => tracing::warn!(error = %err, "the Vicinae import task failed"),
    }
}

/// Follows `ClipboardChanged` for as long as the extension provides it,
/// reconnecting with backoff when it is absent or goes away.
async fn record_from_shell(store: Arc<ClipboardStore>, control: Arc<Control>) {
    let client = match compass_shell::ShellClient::connect_session().await {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "no session bus; clipboard changes will not be recorded");
            control.set_capture(ClipboardBackend::ShellExtension, Some(err.to_string()));
            return;
        }
    };
    let mut delay = RETRY_INITIAL;
    let mut reported_absent = false;
    loop {
        match client.clipboard_changes().await {
            Ok(mut changes) => {
                tracing::info!("recording clipboard changes from the Shell extension");
                control.set_capture(ClipboardBackend::ShellExtension, None);
                control.set_supported(true);
                reported_absent = false;
                delay = RETRY_INITIAL;
                while let Some(change) = changes.next().await {
                    let change = match change {
                        Ok(change) => change,
                        Err(err) => {
                            tracing::warn!(error = %err, "unreadable clipboard change; skipped");
                            continue;
                        }
                    };
                    let store = Arc::clone(&store);
                    let control = Arc::clone(&control);
                    let recorded = tokio::task::spawn_blocking(move || {
                        record_if_monitoring(
                            &store,
                            &control,
                            &change.content.data,
                            &change.content.mime_type,
                            change.source_app.as_deref(),
                        )
                    })
                    .await;
                    match recorded {
                        Ok(Some(Ok(decision))) => tracing::debug!(?decision, "clipboard change"),
                        Ok(None) => tracing::debug!("monitoring is off; a copy was not recorded"),
                        Ok(Some(Err(err))) => {
                            tracing::warn!(error = %err, "clipboard change not recorded")
                        }
                        Err(err) => tracing::warn!(error = %err, "clipboard recording task failed"),
                    }
                }
                control.set_supported(false);
                tracing::info!("the Shell extension stopped sending clipboard changes");
                control.set_capture(
                    ClipboardBackend::ShellExtension,
                    Some("the Shell extension stopped sending clipboard changes".to_owned()),
                );
            }
            Err(err) => {
                // Said once per absence: an extension that is simply not
                // installed would otherwise log every minute forever.
                if !reported_absent {
                    tracing::info!(error = %err, "clipboard changes unavailable; will retry");
                    reported_absent = true;
                }
                control.set_capture(ClipboardBackend::ShellExtension, Some(err.to_string()));
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RETRY_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeStore {
        held: std::sync::Mutex<Option<Vec<u8>>>,
        stores: std::sync::atomic::AtomicUsize,
        fail: bool,
        locked: bool,
    }

    impl SecretStore for FakeStore {
        // Every call is a round trip to the keyring, so it yields, as the
        // real one does, and concurrent callers interleave.
        async fn lookup(&self) -> Result<Option<Vec<u8>>, Error> {
            tokio::task::yield_now().await;
            if self.fail {
                return Err(Error::Keyring("locked".into()));
            }
            Ok(self.held.lock().unwrap().clone())
        }
        async fn store(&self, secret: &[u8]) -> Result<(), Error> {
            tokio::task::yield_now().await;
            self.stores
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.held.lock().unwrap() = Some(secret.to_vec());
            Ok(())
        }
        async fn locked(&self) -> bool {
            tokio::task::yield_now().await;
            self.locked
        }
    }

    #[tokio::test]
    async fn the_first_run_creates_a_key_and_later_runs_reuse_it() {
        let store = FakeStore::default();
        let first = master_key(&store).await.expect("created");
        let second = master_key(&store).await.expect("reused");
        assert_eq!(first, second);
        assert_eq!(store.stores.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_ne!(first, [0; KEY_SIZE], "a real key, not zeroes");
    }

    #[tokio::test]
    async fn callers_racing_on_a_first_run_make_one_key_and_all_get_it() {
        // The engine's first moments: clipboard history and an extension's
        // storage both ask while the keyring is still empty. Two keys would
        // leave the extension database openable under only one of them.
        let store = FakeStore::default();
        let (first, second, third) =
            tokio::join!(master_key(&store), master_key(&store), master_key(&store));
        let first = first.expect("created");
        assert_eq!(second.expect("shared"), first);
        assert_eq!(third.expect("shared"), first);
        assert_eq!(store.stores.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_malformed_entry_is_refused_and_left_alone() {
        let store = FakeStore {
            held: std::sync::Mutex::new(Some(vec![1, 2, 3])),
            ..FakeStore::default()
        };
        let err = master_key(&store).await.expect_err("wrong length");
        assert!(matches!(err, Error::MalformedKey(3)), "{err}");
        assert_eq!(store.stores.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            store.held.lock().unwrap().as_deref(),
            Some(&[1u8, 2, 3][..])
        );
    }

    #[tokio::test]
    async fn a_keyring_failure_creates_nothing() {
        let store = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        assert!(matches!(master_key(&store).await, Err(Error::Keyring(_))));
        assert_eq!(store.stores.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn no_key_is_made_while_the_keyring_is_locked() {
        // Making one would ask the user to unlock the keyring, from a retry
        // loop that must not keep asking.
        let store = FakeStore {
            locked: true,
            ..FakeStore::default()
        };
        assert!(matches!(master_key(&store).await, Err(Error::Keyring(_))));
        assert_eq!(store.stores.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn the_key_state_tells_every_case_apart_and_creates_nothing() {
        let found = FakeStore {
            held: std::sync::Mutex::new(Some(vec![4; KEY_SIZE])),
            ..FakeStore::default()
        };
        assert_eq!(key_state(&found).await, KeyState::Found([4; KEY_SIZE]));
        let malformed = FakeStore {
            held: std::sync::Mutex::new(Some(vec![1, 2, 3])),
            ..FakeStore::default()
        };
        assert_eq!(key_state(&malformed).await, KeyState::Malformed(3));
        let missing = FakeStore::default();
        assert_eq!(key_state(&missing).await, KeyState::Missing);
        assert_eq!(missing.stores.load(std::sync::atomic::Ordering::SeqCst), 0);
        let locked = FakeStore {
            locked: true,
            ..FakeStore::default()
        };
        assert_eq!(key_state(&locked).await, KeyState::Locked);
        let unreachable = FakeStore {
            fail: true,
            ..FakeStore::default()
        };
        assert!(matches!(
            key_state(&unreachable).await,
            KeyState::Unavailable(_)
        ));
        assert_eq!(
            format!("{:?}", KeyState::Found([4; KEY_SIZE])),
            "Found(..)",
            "the key never reaches a log"
        );
    }

    #[test]
    fn only_a_keyring_that_may_yet_answer_is_waited_for() {
        assert!(matches!(
            storage_after(&Error::Keyring("locked".into())),
            ClipboardStorage::Waiting { .. }
        ));
        for needs_reset in [Error::Unreadable("x".into()), Error::MalformedKey(3)] {
            assert!(matches!(
                storage_after(&needs_reset),
                ClipboardStorage::Failed { reset: true, .. }
            ));
        }
        assert!(matches!(
            storage_after(&Error::NoDataDir),
            ClipboardStorage::Failed { reset: false, .. }
        ));
    }

    #[test]
    fn the_status_reports_what_the_service_last_noted() {
        let control = Control::new(&Settings::default());
        let status = control.status();
        assert_eq!(status.storage, ClipboardStorage::Opening);
        assert_eq!(status.backend, None);
        assert!(status.monitoring);
        assert!(!status.capturing);

        control.set_storage(ClipboardStorage::Open);
        control.set_capture(ClipboardBackend::ShellExtension, None);
        control.set_supported(true);
        let status = control.status();
        assert_eq!(status.storage, ClipboardStorage::Open);
        assert_eq!(status.backend, Some(ClipboardBackend::ShellExtension));
        assert!(status.capturing);
        assert_eq!(status.capture_problem, None);
    }

    const MASTER: [u8; KEY_SIZE] = [7; KEY_SIZE];

    #[test]
    fn a_recorded_copy_is_listed_and_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store
            .record(
                b"hello from the clipboard",
                "text/plain",
                Some("org.gnome.TextEditor"),
            )
            .expect("recorded");
        store
            .record(b"https://example.org/page", "text/plain", None)
            .expect("recorded");

        let all = store.history("", 10).expect("listed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].preview, "https://example.org/page", "newest first");
        assert_eq!(all[0].kind, ClipboardKind::Link);
        assert_eq!(all[1].kind, ClipboardKind::Text);

        let found = store.history("clipboard", 10).expect("searched");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].preview, "hello from the clipboard");
    }

    #[test]
    fn copying_the_same_thing_again_does_not_duplicate_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store.record(b"once", "text/plain", None).expect("first");
        store.record(b"twice", "text/plain", None).expect("second");
        store.record(b"once", "text/plain", None).expect("again");
        let all = store.history("", 10).expect("listed");
        let previews: Vec<&str> = all.iter().map(|e| e.preview.as_str()).collect();
        assert_eq!(
            previews,
            ["once", "twice"],
            "moved to the top, not duplicated"
        );
    }

    #[test]
    fn the_history_survives_a_reopen_and_needs_the_same_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
            store.record(b"kept", "text/plain", None).expect("recorded");
        }
        let reopened = ClipboardStore::open(dir.path(), &MASTER).expect("reopen");
        assert_eq!(reopened.history("", 10).expect("listed").len(), 1);

        let wrong = ClipboardStore::open(dir.path(), &[8; KEY_SIZE]);
        assert!(
            matches!(wrong, Err(Error::Unreadable(_))),
            "encrypted at rest: another key cannot read it, and that is told apart"
        );
        assert!(matches!(
            storage_after(&wrong.expect_err("refused")),
            ClipboardStorage::Failed { reset: true, .. }
        ));
    }

    #[test]
    fn a_new_database_that_cannot_be_made_is_not_called_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory where the database file should be: no key could open it,
        // but nor is there a history to set aside.
        std::fs::create_dir(dir.path().join(DATABASE_FILE_NAME)).expect("mkdir");
        let err = ClipboardStore::open(dir.path(), &MASTER).expect_err("not a file");
        assert!(!matches!(err, Error::Unreadable(_)), "{err}");
    }

    #[test]
    fn inspecting_counts_entries_and_tells_a_lost_key_apart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (survey, read) = inspect_files(dir.path(), Some(&MASTER));
        assert!(survey.expect("survey").is_empty());
        assert_eq!(read, None, "no database, nothing to read");
        assert!(
            !dir.path().join(DATABASE_FILE_NAME).exists(),
            "none created"
        );
        {
            let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
            store.record(b"one", "text/plain", None).expect("recorded");
            store.record(b"two", "text/plain", None).expect("recorded");
        }
        assert_eq!(inspect_files(dir.path(), Some(&MASTER)).1, Some(Ok(2)));
        assert_eq!(
            inspect_files(dir.path(), Some(&[8; KEY_SIZE])).1,
            Some(Err(ReadError::Unreadable))
        );
        assert_eq!(inspect_files(dir.path(), None).1, None);
    }

    #[test]
    fn the_store_is_compass_s_own_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        ClipboardStore::open(dir.path(), &MASTER).expect("open");
        assert!(dir.path().join(DATABASE_FILE_NAME).exists());
        assert!(
            !dir.path().join("clipboard.db").exists(),
            "not Vicinae's file"
        );
    }

    #[test]
    fn content_comes_back_whole_and_decrypted_not_as_the_preview() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        let long = "a line of text that goes on ".repeat(40);
        store
            .record(long.as_bytes(), "text/plain", None)
            .expect("recorded");
        let entry = store.history("", 1).expect("listed").remove(0);
        assert!(entry.preview.len() < long.len(), "the preview is truncated");

        let (mime, data) = store.content(&entry.id).expect("read").expect("present");
        assert_eq!(mime, "text/plain");
        assert_eq!(data, long.as_bytes());

        let payload_dir = dir.path().join(PAYLOAD_DIR_NAME);
        let on_disk: Vec<u8> = std::fs::read_dir(&payload_dir)
            .expect("payloads")
            .map(|file| std::fs::read(file.expect("entry").path()).expect("read"))
            .next()
            .expect("one payload");
        assert_ne!(on_disk, long.as_bytes(), "encrypted at rest");

        assert!(store.content("no-such-entry").expect("lookup").is_none());
    }

    #[test]
    fn pinning_lifts_an_entry_to_the_top_and_unpinning_lets_it_fall() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store
            .record(b"older", "text/plain", None)
            .expect("recorded");
        store
            .record(b"newer", "text/plain", None)
            .expect("recorded");
        let older = store.history("older", 1).expect("listed").remove(0);

        assert!(store.set_pinned(&older.id, true).expect("pinned"));
        let top = store.history("", 2).expect("listed").remove(0);
        assert_eq!((top.id.as_str(), top.pinned), (older.id.as_str(), true));

        assert!(store.set_pinned(&older.id, false).expect("unpinned"));
        let top = store.history("", 2).expect("listed").remove(0);
        assert_eq!(top.preview, "newer");
        assert!(!store.set_pinned("no-such-entry", true).expect("lookup"));
    }

    #[test]
    fn removing_an_entry_deletes_its_row_and_its_payload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store
            .record(b"secret", "text/plain", None)
            .expect("recorded");
        store.record(b"kept", "text/plain", None).expect("recorded");
        let payloads = || {
            std::fs::read_dir(dir.path().join(PAYLOAD_DIR_NAME))
                .expect("payloads")
                .count()
        };
        assert_eq!(payloads(), 2);
        let gone = store.history("secret", 1).expect("listed").remove(0);

        assert!(store.remove(&gone.id).expect("removed"));
        let left: Vec<String> = store
            .history("", 10)
            .expect("listed")
            .into_iter()
            .map(|entry| entry.preview)
            .collect();
        assert_eq!(left, ["kept"]);
        assert_eq!(payloads(), 1, "the removed entry's payload is unlinked");
        assert!(store.content(&gone.id).expect("lookup").is_none());
        assert!(
            !store.remove(&gone.id).expect("second remove"),
            "already gone"
        );
    }

    #[test]
    fn a_zero_limit_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        assert!(store.history("", 0).is_err());
    }

    fn payload_count(dir: &Path) -> usize {
        std::fs::read_dir(dir.join(PAYLOAD_DIR_NAME))
            .map(Iterator::count)
            .unwrap_or(0)
    }

    /// Moves every entry's last copy `ago` into the past.
    fn age_everything(store: &ClipboardStore, ago: Duration) {
        let ago = i64::try_from(ago.as_millis()).expect("fits");
        store
            .db()
            .execute(
                "UPDATE selection SET updated_at = updated_at - :ago",
                named_params! { ":ago": ago },
            )
            .expect("aged");
    }

    #[test]
    fn the_kind_filter_keeps_one_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store
            .record(b"plain words", "text/plain", None)
            .expect("text");
        store
            .record(b"https://example.org/", "text/plain", None)
            .expect("link");
        let links = store
            .history_of_kind("", 10, Some(ClipboardKind::Link))
            .expect("listed");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, ClipboardKind::Link);
        assert_eq!(
            store
                .history_of_kind("", 10, Some(ClipboardKind::Image))
                .expect("listed")
                .len(),
            0
        );
        assert_eq!(store.history_of_kind("", 10, None).expect("all").len(), 2);
    }

    #[test]
    fn the_detail_says_size_hash_encryption_and_keywords_which_are_searchable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store
            .record(b"twelve bytes", "text/plain", None)
            .expect("recorded");
        let id = store.history("", 1).expect("listed").remove(0).id;

        let detail = store.detail(&id).expect("read").expect("found");
        assert_eq!(detail.size, 12);
        assert!(detail.encrypted, "the store encrypts at rest");
        assert_eq!(detail.kind, ClipboardKind::Text);
        assert!(!detail.md5.is_empty());
        assert_eq!(detail.keywords, "");

        assert!(store.set_keywords(&id, " receipt ").expect("set"));
        let detail = store.detail(&id).expect("read").expect("found");
        assert_eq!(detail.keywords, "receipt");
        assert_eq!(store.history("receipt", 10).expect("searched").len(), 1);

        assert!(!store.set_keywords("nobody", "x").expect("no such entry"));
        assert!(store.detail("nobody").expect("read").is_none());
    }

    #[test]
    fn remove_all_spares_tagged_entries_when_asked_and_unlinks_the_rest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        store.record(b"loose", "text/plain", None).expect("loose");
        store.record(b"pinned", "text/plain", None).expect("pinned");
        store.record(b"tagged", "text/plain", None).expect("tagged");
        let id_of = |preview: &str| store.history(preview, 1).expect("found").remove(0).id;
        store.set_pinned(&id_of("pinned"), true).expect("pin");
        store.set_keywords(&id_of("tagged"), "keep").expect("tag");

        assert_eq!(store.remove_all(true).expect("removed"), 1);
        assert_eq!(store.history("", 10).expect("listed").len(), 2);
        assert_eq!(payload_count(dir.path()), 2);

        assert_eq!(store.remove_all(false).expect("removed"), 2);
        assert!(store.history("", 10).expect("listed").is_empty());
        assert_eq!(payload_count(dir.path()), 0);
    }

    #[test]
    fn eviction_removes_what_is_older_than_the_threshold_and_reports_the_next() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        let hour = Duration::from_secs(3600);
        store
            .record(b"old and loose", "text/plain", None)
            .expect("old");
        store
            .record(b"old but pinned", "text/plain", None)
            .expect("pinned");
        let pinned = store.history("pinned", 1).expect("found").remove(0).id;
        store.set_pinned(&pinned, true).expect("pin");
        age_everything(&store, 2 * hour);
        store.record(b"fresh", "text/plain", None).expect("fresh");

        let (evicted, oldest) = store.evict(hour, true).expect("swept");
        assert_eq!(evicted, 1, "the pinned entry is preserved");
        let left: Vec<String> = store
            .history("", 10)
            .expect("listed")
            .into_iter()
            .map(|entry| entry.preview)
            .collect();
        assert_eq!(left, ["old but pinned", "fresh"]);
        assert_eq!(payload_count(dir.path()), 2);
        let fresh = store.detail(&store.history("fresh", 1).expect("f").remove(0).id);
        assert_eq!(oldest, fresh.expect("read").map(|d| d.updated_at));

        let (evicted, _) = store.evict(hour, false).expect("swept");
        assert_eq!(evicted, 1, "unpreserved, the old pinned one goes too");
    }

    #[test]
    fn nothing_is_recorded_while_monitoring_is_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ClipboardStore::open(dir.path(), &MASTER).expect("open");
        let control = Control::new(&Settings::default());
        assert!(control.monitoring(), "on by default");

        control.set_monitoring(false);
        assert!(record_if_monitoring(&store, &control, b"private", "text/plain", None).is_none());
        assert!(store.history("", 10).expect("listed").is_empty());

        control.set_monitoring(true);
        assert!(matches!(
            record_if_monitoring(&store, &control, b"public", "text/plain", None),
            Some(Ok(Decision::Inserted { .. }))
        ));
        assert_eq!(store.history("", 10).expect("listed").len(), 1);
    }

    #[test]
    fn the_preferences_are_read_with_the_cpp_defaults() {
        let defaults = Settings::default();
        assert!(defaults.monitoring && defaults.ignore_passwords && defaults.preserve_tagged);
        assert!(!defaults.erase_on_startup);
        assert_eq!(defaults.eviction, None);

        let set: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"monitoring":false,"ignorePasswords":false,"preserveTagged":false,
                "evictionThreshold":"3600","eraseOnStartup":true}"#,
        )
        .expect("json");
        let settings = Settings::from_preferences(Some(&set));
        assert_eq!(
            settings,
            Settings {
                monitoring: false,
                ignore_passwords: false,
                preserve_tagged: false,
                eviction: Some(Duration::from_secs(3600)),
                erase_on_startup: true,
            }
        );
        let never: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(r#"{"evictionThreshold":"never"}"#).expect("json");
        assert_eq!(Settings::from_preferences(Some(&never)).eviction, None);
    }
}
