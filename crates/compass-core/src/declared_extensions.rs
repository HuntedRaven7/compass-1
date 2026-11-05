//! `extensions.installed`: the store extensions `compass.json` asks for, and
//! which of them still have to be installed.
//!
//! An entry names a store extension in one of two ways:
//!
//! * `<store>:<owner>/<name>`, such as `raycast:raycast/github` or
//!   `vicinae:zoe/clock`. The owner is the handle the store files the
//!   extension under, which is what its detail and download addresses are
//!   built from, so this form installs without a lookup.
//! * The id the extension installs under, such as `store.raycast.github`.
//!   This is what the key could hold before stores were named, so it keeps
//!   working: the engine finds the owner by looking the name up, and refuses
//!   when the store lists more than one extension with that name.
//!
//! Any other entry is the id of an extension that does not come from a store
//! (one built locally, or shipped by the system). It is satisfied when it is
//! installed and reported otherwise; nothing can install it.
//!
//! Reconciling only ever installs. An extension that is installed but not
//! declared is left alone, and so is its data, because uninstalling deletes
//! the extension's preferences and storage too, and nothing records whether
//! the configuration or the user installed it. Removing an entry therefore
//! keeps the extension; uninstalling a declared one gets it back on the next
//! reconcile.

use crate::raycast_overrides::Manifest;
use crate::store_bundle::is_safe_id;
use crate::store_listing::Store;

/// A store extension an entry asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreExtension {
    /// The store it comes from.
    pub store: Store,
    /// The handle the store files it under; `None` when the entry gave only
    /// the installed id, so it has to be looked up.
    pub owner: Option<String>,
    /// Its name in that store.
    pub name: String,
}

impl StoreExtension {
    /// The id it installs under, which is how an installed copy is found.
    #[must_use]
    pub fn id(&self) -> String {
        self.store.extension_id(&self.name)
    }
}

/// What one entry of `extensions.installed` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// An extension one of the stores can install.
    Store(StoreExtension),
    /// The id of an extension no store provides.
    Local(String),
}

/// Why an entry cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    /// The entry is empty or only spaces.
    #[error("An empty entry names no extension.")]
    Empty,
    /// The part before `:` is not a store Compass installs from.
    #[error("\"{0}\" is not a store Compass installs from; use raycast or vicinae.")]
    UnknownStore(String),
    /// The part after `<store>:` is not `<owner>/<name>`.
    #[error("\"{0}\" does not name an extension as <store>:<owner>/<name>.")]
    Malformed(String),
}

/// Reads one entry of `extensions.installed`.
///
/// # Errors
///
/// When the entry is empty, names an unknown store, or is not
/// `<store>:<owner>/<name>` after naming one.
pub fn parse(entry: &str) -> Result<Entry, EntryError> {
    let entry = entry.trim();
    if entry.is_empty() {
        return Err(EntryError::Empty);
    }
    if let Some((store, rest)) = entry.split_once(':') {
        let store = match store {
            "raycast" => Store::Raycast,
            "vicinae" => Store::Vicinae,
            other => return Err(EntryError::UnknownStore(other.to_owned())),
        };
        let malformed = || EntryError::Malformed(entry.to_owned());
        let (owner, name) = rest.split_once('/').ok_or_else(malformed)?;
        if owner.is_empty()
            || owner.contains(['/', '\\'])
            || name.is_empty()
            || !is_safe_id(&store.extension_id(name))
        {
            return Err(malformed());
        }
        return Ok(Entry::Store(StoreExtension {
            store,
            owner: Some(owner.to_owned()),
            name: name.to_owned(),
        }));
    }
    for store in [Store::Raycast, Store::Vicinae] {
        let prefix = store.extension_id("");
        if let Some(name) = entry.strip_prefix(&prefix)
            && !name.is_empty()
            && is_safe_id(entry)
        {
            return Ok(Entry::Store(StoreExtension {
                store,
                owner: None,
                name: name.to_owned(),
            }));
        }
    }
    Ok(Entry::Local(entry.to_owned()))
}

/// What reconciling does with one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// It is installed already, under `id`.
    Present {
        /// The entry as written.
        entry: String,
        /// The id it is installed under.
        id: String,
    },
    /// It has to be installed.
    Install {
        /// The entry as written.
        entry: String,
        /// What to install. A Raycast extension that a Linux replacement
        /// stands in for has already been swapped for the replacement, as
        /// installing it from the store would.
        extension: StoreExtension,
    },
    /// It is not installed and cannot be: the entry does not read, or names
    /// an extension no store provides.
    Unresolvable {
        /// The entry as written.
        entry: String,
        /// Why, as a sentence.
        reason: String,
    },
}

/// What reconciling `entries` against the extensions installed now does,
/// one step per entry in order. An entry naming the same extension as an
/// earlier one is dropped.
///
/// `is_installed` answers for an extension id; `overrides` names the
/// Raycast extensions a Linux replacement stands in for.
#[must_use]
pub fn plan(
    entries: &[String],
    is_installed: impl Fn(&str) -> bool,
    overrides: &Manifest,
) -> Vec<Step> {
    let mut seen = std::collections::HashSet::new();
    let mut steps = Vec::new();
    for written in entries {
        let entry = written.clone();
        let extension = match parse(written) {
            Ok(Entry::Store(extension)) => extension,
            Ok(Entry::Local(id)) => {
                if !seen.insert(id.clone()) {
                    continue;
                }
                steps.push(if is_installed(&id) {
                    Step::Present { entry, id }
                } else {
                    Step::Unresolvable {
                        entry,
                        reason: format!(
                            "No extension is installed with the id {id}, and it does not name a store extension. Write a store extension as raycast:<owner>/<name> or vicinae:<owner>/<name>."
                        ),
                    }
                });
                continue;
            }
            Err(error) => {
                steps.push(Step::Unresolvable {
                    entry,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let declared_id = extension.id();
        let extension = replacement(extension, overrides);
        let id = extension.id();
        if !seen.insert(id.clone()) {
            continue;
        }
        steps.push(
            if let Some(id) = [&id, &declared_id]
                .into_iter()
                .find(|candidate| is_installed(candidate))
            {
                Step::Present {
                    entry,
                    id: id.clone(),
                }
            } else {
                Step::Install { entry, extension }
            },
        );
    }
    steps
}

/// The Linux replacement of a Raycast extension when the overrides manifest
/// names one, as the store's install does; the extension itself otherwise.
fn replacement(extension: StoreExtension, overrides: &Manifest) -> StoreExtension {
    if extension.store != Store::Raycast {
        return extension;
    }
    match overrides.raycast_redirect(&extension.name) {
        Some(redirect) if redirect.store == Store::Vicinae.key() => StoreExtension {
            store: Store::Vicinae,
            owner: Some(redirect.author.clone()),
            name: redirect.name.clone(),
        },
        _ => extension,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(store: Store, owner: Option<&str>, name: &str) -> Entry {
        Entry::Store(StoreExtension {
            store,
            owner: owner.map(str::to_owned),
            name: name.to_owned(),
        })
    }

    #[test]
    fn both_forms_name_a_store_extension() {
        assert_eq!(
            parse("raycast:raycast/github"),
            Ok(store(Store::Raycast, Some("raycast"), "github"))
        );
        assert_eq!(
            parse(" vicinae:zoe/clock "),
            Ok(store(Store::Vicinae, Some("zoe"), "clock"))
        );
        assert_eq!(
            parse("store.raycast.github"),
            Ok(store(Store::Raycast, None, "github"))
        );
        assert_eq!(
            parse("store.vicinae.clock"),
            Ok(store(Store::Vicinae, None, "clock"))
        );
        assert_eq!(
            parse("com.example.clock"),
            Ok(Entry::Local("com.example.clock".into()))
        );
    }

    #[test]
    fn an_entry_that_could_escape_the_extensions_directory_is_refused() {
        assert_eq!(parse("  "), Err(EntryError::Empty));
        assert_eq!(
            parse("brew:me/thing"),
            Err(EntryError::UnknownStore("brew".into()))
        );
        for bad in [
            "raycast:github",
            "raycast:/github",
            "raycast:raycast/",
            "raycast:raycast/../../x",
            "raycast:a/b/c",
            "vicinae:a\\b/c",
        ] {
            assert!(matches!(parse(bad), Err(EntryError::Malformed(_))), "{bad}");
        }
        // A bare prefix, or a name that would be a path, is not a store id.
        assert_eq!(
            parse("store.raycast."),
            Ok(Entry::Local("store.raycast.".into()))
        );
    }

    fn entries(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn only_what_is_missing_is_installed_and_each_extension_once() {
        let installed = ["store.vicinae.clock", "local.notes"];
        let steps = plan(
            &entries(&[
                "vicinae:zoe/clock",
                "raycast:raycast/github",
                "store.raycast.github",
                "local.notes",
                "local.gone",
                "brew:x/y",
            ]),
            |id| installed.contains(&id),
            &Manifest::default(),
        );
        assert_eq!(
            steps,
            [
                Step::Present {
                    entry: "vicinae:zoe/clock".into(),
                    id: "store.vicinae.clock".into(),
                },
                Step::Install {
                    entry: "raycast:raycast/github".into(),
                    extension: StoreExtension {
                        store: Store::Raycast,
                        owner: Some("raycast".into()),
                        name: "github".into(),
                    },
                },
                Step::Present {
                    entry: "local.notes".into(),
                    id: "local.notes".into(),
                },
                Step::Unresolvable {
                    entry: "local.gone".into(),
                    reason: "No extension is installed with the id local.gone, and it does not name a store extension. Write a store extension as raycast:<owner>/<name> or vicinae:<owner>/<name>.".into(),
                },
                Step::Unresolvable {
                    entry: "brew:x/y".into(),
                    reason: "\"brew\" is not a store Compass installs from; use raycast or vicinae."
                        .into(),
                },
            ]
        );
    }

    #[test]
    fn a_plan_with_everything_installed_does_nothing() {
        let declared = entries(&["vicinae:zoe/clock", "store.raycast.github"]);
        let steps = plan(&declared, |_| true, &Manifest::default());
        assert!(
            steps
                .iter()
                .all(|step| matches!(step, Step::Present { .. }))
        );
    }

    #[test]
    fn a_replaced_raycast_extension_installs_its_replacement() {
        let overrides = Manifest::parse(
            r#"{"version": 1, "extensions": {"store.raycast.mac-thing": {"why": "macOS only.", "redirect": {
                "store": "vicinae", "author": "linux-dev", "name": "linux-thing",
                "why": "It drives a macOS app."
            }}}}"#,
        )
        .expect("a manifest");
        let declared = entries(&["raycast:someone/mac-thing"]);
        assert_eq!(
            plan(&declared, |_| false, &overrides),
            [Step::Install {
                entry: "raycast:someone/mac-thing".into(),
                extension: StoreExtension {
                    store: Store::Vicinae,
                    owner: Some("linux-dev".into()),
                    name: "linux-thing".into(),
                },
            }]
        );
        let replaced = plan(
            &declared,
            |id| id == "store.vicinae.linux-thing",
            &overrides,
        );
        assert!(
            matches!(&replaced[..], [Step::Present { id, .. }] if id == "store.vicinae.linux-thing")
        );
    }
}
