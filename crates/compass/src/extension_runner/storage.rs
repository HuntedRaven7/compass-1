//! Extensions' local storage and preference values: Compass's own encrypted
//! database ([`super::STORAGE_DATABASE`]), keyed from the login keyring.
//!
//! Separate because more than a run opens it: the engine reads and keeps
//! preference values, clears an uninstalled extension's data, and the
//! calculator history and the storage views share the database.

use std::path::PathBuf;

/// Where a run keeps its local storage, and the key that opens it.
#[derive(Clone)]
pub struct Storage {
    /// The database file.
    pub path: PathBuf,
    /// Its SQLCipher key.
    pub key: [u8; compass_crypto::KEY_SIZE],
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The local-storage namespace an extension's preference values live in,
/// beside its own `<id>:data`: in the same encrypted database, and out of
/// the extension's reach, since its `LocalStorage` is scoped to `:data`.
#[must_use]
pub fn preferences_namespace(extension_id: &str) -> String {
    format!("compass.preferences:{extension_id}")
}

/// The preference values stored for `extension_id`; empty when none are, or
/// the database will not open.
#[must_use]
pub fn load_preferences(
    storage: &Storage,
    extension_id: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let Some(db) = open_storage(storage) else {
        return serde_json::Map::new();
    };
    let local = compass_local_storage::LocalStorage::new(&db);
    let scoped = local.scoped(&preferences_namespace(extension_id));
    match scoped.list() {
        Ok(values) => values
            .into_iter()
            .map(|(name, value)| (name, value.to_json()))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "could not read extension preferences");
            serde_json::Map::new()
        }
    }
}

/// Removes everything `extension_id` kept in local storage and its stored
/// preference values; logged, never fatal, since the extension is already
/// gone by the time this runs.
pub fn clear_extension_data(storage: &Storage, extension_id: &str) {
    let Some(db) = open_storage(storage) else {
        return;
    };
    let local = compass_local_storage::LocalStorage::new(&db);
    for namespace in [
        compass_local_storage::namespace_for(extension_id),
        preferences_namespace(extension_id),
    ] {
        if let Err(err) = local.scoped(&namespace).clear() {
            tracing::warn!(%err, %namespace, "could not clear an uninstalled extension's data");
        }
    }
}

/// Keeps `values` for `extension_id`. A null or empty value removes the
/// stored one, so clearing a field falls back to its default.
///
/// # Errors
///
/// A sentence: the database would not open, or a write failed.
pub fn save_preferences(
    storage: &Storage,
    extension_id: &str,
    values: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let db = open_storage(storage).ok_or("Compass could not open its extension storage")?;
    let local = compass_local_storage::LocalStorage::new(&db);
    let scoped = local.scoped(&preferences_namespace(extension_id));
    for (name, value) in values {
        let cleared = value.is_null() || value.as_str() == Some("");
        let written = if cleared {
            scoped.remove(name).map(drop)
        } else {
            scoped.set(name, &compass_local_storage::Value::from_json(value))
        };
        written.map_err(|err| format!("Compass could not keep the preference {name}: {err}"))?;
    }
    Ok(())
}

pub(crate) fn open_storage(
    storage: &Storage,
) -> Option<compass_sqlcipher_sys::rusqlite::Connection> {
    let opened = compass_sqlcipher_sys::open(&storage.path, &storage.key)
        .map_err(|err| err.to_string())
        .and_then(|db| {
            compass_db::vicinae::run(&db)
                .map(|()| db)
                .map_err(|err| err.to_string())
        });
    match opened {
        Ok(db) => Some(db),
        Err(err) => {
            tracing::warn!(path = %storage.path.display(), %err, "extension storage unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::extension_runner::STORAGE_DATABASE;

    fn storage(dir: &Path) -> Storage {
        Storage {
            path: dir.join(STORAGE_DATABASE),
            key: [3; compass_crypto::KEY_SIZE],
        }
    }

    #[test]
    fn preferences_round_trip_and_clearing_one_removes_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = storage(dir.path());
        assert!(load_preferences(&storage, "github").is_empty());

        let values = serde_json::json!({"token": "ghp_x", "limit": 50, "private": true});
        save_preferences(&storage, "github", values.as_object().unwrap()).expect("saved");
        assert_eq!(
            serde_json::Value::Object(load_preferences(&storage, "github")),
            serde_json::json!({"token": "ghp_x", "limit": 50.0, "private": true}),
            "strings and booleans come back as they went in, and a number as the \
             double JavaScript would have had anyway"
        );
        assert!(
            load_preferences(&storage, "other").is_empty(),
            "per extension"
        );

        let cleared = serde_json::json!({"token": ""});
        save_preferences(&storage, "github", cleared.as_object().unwrap()).expect("saved");
        assert!(!load_preferences(&storage, "github").contains_key("token"));
    }

    #[test]
    fn preferences_are_out_of_the_extensions_own_storage() {
        // An extension's LocalStorage is scoped to `<id>:data`. Keeping
        // preferences there would let it read and rewrite its own token.
        assert_ne!(
            preferences_namespace("github"),
            compass_local_storage::namespace_for("github")
        );
    }
}
