//! An injectable snapshot of the environment.
//!
//! No check in [`super::checks`] calls [`std::env::var`]. They all read an
//! [`Env`], which is either a snapshot of the real process environment
//! ([`Env::from_process`]) or a map a test built by hand ([`Env::from_pairs`]).
//! That is the whole reason the doctor is testable: "`XDG_RUNTIME_DIR` is
//! unset" is a value, not a global mutation, and edition 2024 makes
//! `std::env::set_var` `unsafe` anyway — which this crate forbids.

use std::collections::BTreeMap;

/// A read-only set of environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    /// Snapshots the real process environment.
    ///
    /// Non-UTF-8 values are dropped: every variable the doctor looks at is a
    /// path or an identifier, and reporting a lossy rendering of a broken value
    /// as if it were the real one would be worse than reporting it as unset.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Builds an environment from explicit pairs. The injection point.
    pub fn from_pairs<K, V, I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// An empty environment: every variable unset.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The value of `key`, treating an empty value as unset.
    ///
    /// An empty `WAYLAND_DISPLAY` means "no Wayland" everywhere in the desktop
    /// stack, so collapsing empty into absent here keeps every call site from
    /// having to remember it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// Whether `key` is set to a non-empty value.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Splits `key` on `:` into its non-empty components, as XDG list-valued
    /// variables (`XDG_DATA_DIRS`, `XDG_CURRENT_DESKTOP`) are specified.
    #[must_use]
    pub fn list(&self, key: &str) -> Vec<&str> {
        self.get(key)
            .map(|v| v.split(':').filter(|p| !p.is_empty()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_value_reads_as_unset() {
        let env = Env::from_pairs([("WAYLAND_DISPLAY", "")]);
        assert_eq!(env.get("WAYLAND_DISPLAY"), None);
        assert!(!env.has("WAYLAND_DISPLAY"));
    }

    #[test]
    fn set_value_is_returned() {
        let env = Env::from_pairs([("WAYLAND_DISPLAY", "wayland-0")]);
        assert_eq!(env.get("WAYLAND_DISPLAY"), Some("wayland-0"));
        assert!(env.has("WAYLAND_DISPLAY"));
    }

    #[test]
    fn missing_key_is_none() {
        assert_eq!(Env::empty().get("HOME"), None);
    }

    #[test]
    fn list_splits_and_drops_empty_components() {
        let env = Env::from_pairs([("XDG_DATA_DIRS", "/usr/share::/usr/local/share:")]);
        assert_eq!(
            env.list("XDG_DATA_DIRS"),
            ["/usr/share", "/usr/local/share"]
        );
    }

    #[test]
    fn list_of_unset_key_is_empty() {
        assert!(Env::empty().list("XDG_DATA_DIRS").is_empty());
    }
}
