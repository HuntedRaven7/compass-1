//! The extension deny policy from `compass.json`.
//!
//! Two lists are accepted: `extensions.deny`, written by the user, and
//! `extensions.admin_deny`, which is only read from the system-wide defaults
//! under `$XDG_CONFIG_DIRS`. They are combined here; the admin list cannot be
//! removed by a user entry because the two are separate keys. A rule with no
//! criteria matches nothing — a malformed or half-written entry must never
//! silently block every extension.

use globset::{GlobBuilder, GlobMatcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One deny rule. All fields that are set must match (they are ANDed).
///
/// `id`, `name` and `author` are case-insensitive globs, so `"spammer-*"`
/// denies a publisher's namespace and `"store.raycast.news*"` a prefix.
/// `store` is one of `raycast`, `vicinae` or `local`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DenyRule {
    /// Installed id, store listing id, or `raycast:<owner>/<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Extension name or the title of one of its commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Author or owner handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Source store: `raycast`, `vicinae`, or `local` for sideloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// Why this extension is blocked, shown in the refusal message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The identity a rule is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionIdentity<'a> {
    /// Installed directory id or store listing id.
    pub id: &'a str,
    /// Manifest or listing name.
    pub name: &'a str,
    /// Human title, including a command title when that is what was invoked.
    pub title: &'a str,
    /// Manifest author or listing author handle.
    pub author: &'a str,
    /// Listing owner, when the store has one.
    pub owner: Option<&'a str>,
    /// `raycast`, `vicinae`, or `local`.
    pub store: &'a str,
}

impl<'a> ExtensionIdentity<'a> {
    /// The identity of an installed extension command, using its extension's
    /// title when the command itself has none.
    #[must_use]
    pub fn from_command(command: &'a crate::extension_commands::ExtensionCommand) -> Self {
        let store = match crate::manifest::Provenance::from_id(&command.extension_id) {
            crate::manifest::Provenance::Raycast => "raycast",
            crate::manifest::Provenance::Vicinae => "vicinae",
            crate::manifest::Provenance::Local => "local",
        };
        Self {
            id: &command.extension_id,
            name: &command.extension_name,
            title: &command.title,
            author: &command.author,
            owner: command.owner.as_deref(),
            store,
        }
    }
}

#[derive(Debug, Clone)]
struct CompiledRule {
    id: Option<GlobMatcher>,
    name: Option<GlobMatcher>,
    author: Option<GlobMatcher>,
    store: Option<String>,
    reason: Option<String>,
}

fn glob(pattern: &Option<String>) -> Option<GlobMatcher> {
    pattern.as_ref().and_then(|pattern| {
        GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(false)
            .build()
            .map(|glob| glob.compile_matcher())
            .map_err(|error| tracing::warn!(%pattern, %error, "ignoring an invalid deny pattern"))
            .ok()
    })
}

/// The compiled policy, cheap to consult on every search and install.
#[derive(Debug, Clone, Default)]
pub struct ExtensionDeny {
    rules: Vec<CompiledRule>,
}

impl ExtensionDeny {
    /// Compiles the rules, dropping an empty or unparseable one with a warning.
    #[must_use]
    pub fn new(rules: &[DenyRule]) -> Self {
        let rules = rules
            .iter()
            .filter_map(|rule| {
                let id = glob(&rule.id);
                let name = glob(&rule.name);
                let author = glob(&rule.author);
                let store = rule.store.as_ref().map(|store| store.to_ascii_lowercase());
                if id.is_none() && name.is_none() && author.is_none() && store.is_none() {
                    tracing::warn!("ignoring an empty extension deny rule");
                    return None;
                }
                Some(CompiledRule {
                    id,
                    name,
                    author,
                    store,
                    reason: rule.reason.clone(),
                })
            })
            .collect();
        Self { rules }
    }

    /// Whether any rule is active.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The reason the identity is blocked, or `None` when it is allowed.
    #[must_use]
    pub fn denies(&self, identity: &ExtensionIdentity<'_>) -> Option<&str> {
        for rule in &self.rules {
            if let Some(pattern) = &rule.id
                && !pattern.is_match(identity.id)
            {
                continue;
            }
            if let Some(pattern) = &rule.name
                && !pattern.is_match(identity.name)
                && !pattern.is_match(identity.title)
            {
                continue;
            }
            if let Some(pattern) = &rule.author
                && !pattern.is_match(identity.author)
                && !identity.owner.is_some_and(|owner| pattern.is_match(owner))
            {
                continue;
            }
            if let Some(store) = &rule.store
                && store != identity.store
            {
                continue;
            }
            return Some(
                rule.reason
                    .as_deref()
                    .unwrap_or("blocked by the extension deny list"),
            );
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(rules: &[DenyRule]) -> ExtensionDeny {
        ExtensionDeny::new(rules)
    }

    fn identity() -> ExtensionIdentity<'static> {
        ExtensionIdentity {
            id: "store.raycast.github",
            name: "github",
            title: "Search GitHub Issues",
            author: "raycast",
            owner: Some("github"),
            store: "raycast",
        }
    }

    #[test]
    fn an_empty_rule_denies_nothing() {
        assert!(rules(&[DenyRule::default()]).denies(&identity()).is_none());
    }

    #[test]
    fn criteria_are_anded() {
        let policy = rules(&[DenyRule {
            store: Some("vicinae".into()),
            author: Some("raycast".into()),
            ..DenyRule::default()
        }]);
        assert!(policy.denies(&identity()).is_none());
        let vicinae = rules(&[DenyRule {
            store: Some("raycast".into()),
            author: Some("raycast".into()),
            ..DenyRule::default()
        }]);
        assert!(vicinae.denies(&identity()).is_some());
    }

    #[test]
    fn globs_match_id_name_title_and_owner() {
        for rule in [
            DenyRule {
                id: Some("store.raycast.*".into()),
                ..DenyRule::default()
            },
            DenyRule {
                name: Some("GitHUB".into()),
                ..DenyRule::default()
            },
            DenyRule {
                name: Some("*Issues".into()),
                ..DenyRule::default()
            },
            DenyRule {
                author: Some("Git*".into()),
                ..DenyRule::default()
            },
        ] {
            assert!(
                rules(std::slice::from_ref(&rule))
                    .denies(&identity())
                    .is_some(),
                "{rule:?} should match"
            );
        }
    }

    #[test]
    fn a_bad_glob_does_not_block_everything() {
        let policy = rules(&[DenyRule {
            name: Some("[".into()),
            ..DenyRule::default()
        }]);
        assert!(policy.is_empty());
        assert!(policy.denies(&identity()).is_none());
    }

    #[test]
    fn the_reason_reaches_the_caller() {
        let policy = rules(&[DenyRule {
            store: Some("raycast".into()),
            reason: Some("not allowed on this image".into()),
            ..DenyRule::default()
        }]);
        assert_eq!(
            policy.denies(&identity()),
            Some("not allowed on this image")
        );
    }
}
