//! Capabilities: what an extension is allowed to reach.
//!
//! Three rules shape this module.
//!
//! **Nothing is ambient.** A capability must be *declared* by the extension (it appears
//! in its manifest) and then *granted* by the host (the user consented, or policy allows
//! it). Declaring is not granting; granting something undeclared is refused, so a
//! capability can never appear at runtime that the manifest did not advertise.
//!
//! **The set is open.** [`Capability`] is a namespaced string, not an enum. An extension
//! built against a newer host may declare `whatever.new`; an older host parses it,
//! reports it as unknown, never grants it, and keeps running. That is the difference
//! between an extension that degrades and a host that crashes.
//!
//! **Denial is a value.** [`check`](CapabilityRegistry::check) returns
//! `Result<CapabilityGrant, Denial>`, and [`Denial`] names the extension, the capability
//! and the reason. It is serialisable, so the exact missing capability can be shown to
//! the user or sent onward; it is an [`std::error::Error`], so it can be propagated; it
//! is never a panic.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// Identity of an installed extension.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExtensionId(pub String);

impl ExtensionId {
    /// Wraps an identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A named permission, in dotted `area.verb` form.
///
/// Open by construction. The constants below are the ones the product actually has —
/// each corresponds to a service the host already exposes to extensions — but any string
/// is a valid `Capability`, and an unrecognised one is simply never granted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capability(Cow<'static, str>);

impl Capability {
    /// Read the clipboard's current contents.
    pub const CLIPBOARD_READ: Capability = Capability(Cow::Borrowed("clipboard.read"));
    /// Replace the clipboard's contents.
    pub const CLIPBOARD_WRITE: Capability = Capability(Cow::Borrowed("clipboard.write"));
    /// Paste into the focused surface.
    pub const CLIPBOARD_PASTE: Capability = Capability(Cow::Borrowed("clipboard.paste"));
    /// Read the current text selection.
    pub const SELECTION_READ: Capability = Capability(Cow::Borrowed("selection.read"));
    /// Enumerate open windows and the active workspace.
    pub const WINDOW_READ: Capability = Capability(Cow::Borrowed("window.read"));
    /// Focus, move or resize windows.
    pub const WINDOW_MANAGE: Capability = Capability(Cow::Borrowed("window.manage"));
    /// Enumerate installed applications and default handlers.
    pub const APPLICATION_LIST: Capability = Capability(Cow::Borrowed("application.list"));
    /// Open a file, URL or application.
    pub const APPLICATION_OPEN: Capability = Capability(Cow::Borrowed("application.open"));
    /// Reveal a path in the file browser.
    pub const FILE_REVEAL: Capability = Capability(Cow::Borrowed("file.reveal"));
    /// Query the file index.
    pub const FILE_SEARCH: Capability = Capability(Cow::Borrowed("file.search"));
    /// Read the extension's own key/value store.
    pub const STORAGE_READ: Capability = Capability(Cow::Borrowed("storage.read"));
    /// Write the extension's own key/value store.
    pub const STORAGE_WRITE: Capability = Capability(Cow::Borrowed("storage.write"));
    /// Make outbound network requests.
    pub const NETWORK_REQUEST: Capability = Capability(Cow::Borrowed("network.request"));
    /// Run a command in a terminal or spawn a process.
    pub const PROCESS_EXECUTE: Capability = Capability(Cow::Borrowed("process.execute"));
    /// Post a desktop notification.
    pub const NOTIFICATION_SEND: Capability = Capability(Cow::Borrowed("notification.send"));
    /// Run an OAuth flow and hold the resulting tokens.
    pub const OAUTH_TOKENS: Capability = Capability(Cow::Borrowed("oauth.tokens"));
    /// Launch another command, in this extension or another.
    pub const COMMAND_LAUNCH: Capability = Capability(Cow::Borrowed("command.launch"));
    /// Focus a browser tab through the browser integration.
    pub const BROWSER_TABS: Capability = Capability(Cow::Borrowed("browser.tabs"));
    /// Set the desktop wallpaper.
    pub const WALLPAPER_SET: Capability = Capability(Cow::Borrowed("wallpaper.set"));

    /// Every capability this build knows about.
    pub const KNOWN: &'static [Capability] = &[
        Self::CLIPBOARD_READ,
        Self::CLIPBOARD_WRITE,
        Self::CLIPBOARD_PASTE,
        Self::SELECTION_READ,
        Self::WINDOW_READ,
        Self::WINDOW_MANAGE,
        Self::APPLICATION_LIST,
        Self::APPLICATION_OPEN,
        Self::FILE_REVEAL,
        Self::FILE_SEARCH,
        Self::STORAGE_READ,
        Self::STORAGE_WRITE,
        Self::NETWORK_REQUEST,
        Self::PROCESS_EXECUTE,
        Self::NOTIFICATION_SEND,
        Self::OAUTH_TOKENS,
        Self::COMMAND_LAUNCH,
        Self::BROWSER_TABS,
        Self::WALLPAPER_SET,
    ];

    /// Wraps an arbitrary capability name, known or not.
    pub fn new(name: impl Into<String>) -> Self {
        Self(Cow::Owned(name.into()))
    }

    /// The dotted name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this build recognises the name. An unknown capability is still a perfectly
    /// valid value; it just can never be granted.
    #[must_use]
    pub fn is_known(&self) -> bool {
        Self::KNOWN.contains(self)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&'static str> for Capability {
    fn from(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }
}

/// Why a capability check failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DenialReason {
    /// The registry has never heard of this extension.
    UnknownExtension,
    /// The extension's manifest does not ask for this capability.
    NotDeclared,
    /// Declared, but the host has not granted it — typically the user has not consented.
    NotGranted,
    /// Granted once and taken away since.
    Revoked,
    /// The name is not one this build understands, so it cannot be granted at all.
    /// Produced when an extension built against a newer host declares something new.
    UnknownCapability,
}

impl fmt::Display for DenialReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DenialReason::UnknownExtension => "the extension is not registered",
            DenialReason::NotDeclared => "it is not declared in the extension's manifest",
            DenialReason::NotGranted => "it has not been granted",
            DenialReason::Revoked => "it was revoked",
            DenialReason::UnknownCapability => "this version does not support that capability",
        };
        f.write_str(s)
    }
}

/// A refused capability check: a value, not a panic and not a string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denial {
    /// Who asked.
    pub extension: ExtensionId,
    /// Exactly which capability was missing. This is the field a caller shows the user.
    pub capability: Capability,
    /// Why.
    pub reason: DenialReason,
}

impl fmt::Display for Denial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "extension `{}` cannot use capability `{}`: {}",
            self.extension, self.capability, self.reason
        )
    }
}

impl std::error::Error for Denial {}

/// Proof that a capability was held at the moment of the check.
///
/// Hosts should take one of these by value in the function that actually performs the
/// privileged work, so that "did we check?" is answered by the type system rather than by
/// review. It cannot be constructed outside this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrant {
    extension: ExtensionId,
    capability: Capability,
}

impl CapabilityGrant {
    /// Who holds it.
    #[must_use]
    pub fn extension(&self) -> &ExtensionId {
        &self.extension
    }

    /// What is held.
    #[must_use]
    pub fn capability(&self) -> &Capability {
        &self.capability
    }
}

/// What one extension declared and what it holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityState {
    /// Everything the manifest asks for, unknown names included — kept verbatim so that
    /// upgrading the host is enough to make a newer extension work.
    #[serde(default)]
    pub declared: BTreeSet<Capability>,
    /// The subset actually granted.
    #[serde(default)]
    pub granted: BTreeSet<Capability>,
    /// Grants that were taken away, so denial can say "revoked" rather than "not granted".
    #[serde(default)]
    pub revoked: BTreeSet<Capability>,
}

impl CapabilityState {
    /// Declarations this build does not understand.
    pub fn unknown_declarations(&self) -> Vec<&Capability> {
        self.declared.iter().filter(|c| !c.is_known()).collect()
    }
}

/// The host's record of who may do what.
///
/// A host asks it one question, in one shape:
///
/// ```
/// use compass_extension_api::{Capability, CapabilityRegistry, ExtensionId};
///
/// let mut registry = CapabilityRegistry::default();
/// let ext = ExtensionId::new("com.example.clip");
/// registry.declare(&ext, [Capability::CLIPBOARD_READ]);
/// registry.grant(&ext, &Capability::CLIPBOARD_READ).unwrap();
///
/// match registry.check(&ext, &Capability::CLIPBOARD_READ) {
///     Ok(grant) => { let _ = grant; /* now do the privileged thing */ }
///     Err(denial) => panic!("{denial}"),
/// }
///
/// let denial = registry.check(&ext, &Capability::NETWORK_REQUEST).unwrap_err();
/// assert_eq!(denial.capability, Capability::NETWORK_REQUEST);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRegistry {
    #[serde(default)]
    extensions: BTreeMap<ExtensionId, CapabilityState>,
}

impl CapabilityRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what an extension's manifest asks for, replacing any previous declaration.
    ///
    /// Unknown names are kept. Grants for capabilities no longer declared are dropped:
    /// an extension cannot retain a grant it stopped asking for.
    pub fn declare(&mut self, extension: &ExtensionId, caps: impl IntoIterator<Item = Capability>) {
        let declared: BTreeSet<Capability> = caps.into_iter().collect();
        let state = self.extensions.entry(extension.clone()).or_default();
        state.granted.retain(|c| declared.contains(c));
        state.revoked.retain(|c| declared.contains(c));
        for cap in declared.iter().filter(|c| !c.is_known()) {
            tracing::debug!(
                extension = %extension,
                capability = %cap,
                "extension declares a capability this build does not know; it will never be granted"
            );
        }
        state.declared = declared;
    }

    /// Grants a declared capability.
    ///
    /// Refuses, with the same [`Denial`] the host would surface at call time, when the
    /// extension is unknown, has not declared the capability, or when the capability is
    /// not one this build understands.
    pub fn grant(&mut self, extension: &ExtensionId, cap: &Capability) -> Result<(), Denial> {
        let deny = |reason| {
            Err(Denial {
                extension: extension.clone(),
                capability: cap.clone(),
                reason,
            })
        };
        let Some(state) = self.extensions.get_mut(extension) else {
            return deny(DenialReason::UnknownExtension);
        };
        if !state.declared.contains(cap) {
            return deny(DenialReason::NotDeclared);
        }
        if !cap.is_known() {
            return deny(DenialReason::UnknownCapability);
        }
        state.revoked.remove(cap);
        state.granted.insert(cap.clone());
        Ok(())
    }

    /// Takes a grant away. Returns whether anything was held.
    pub fn revoke(&mut self, extension: &ExtensionId, cap: &Capability) -> bool {
        let Some(state) = self.extensions.get_mut(extension) else {
            return false;
        };
        let held = state.granted.remove(cap);
        if held {
            state.revoked.insert(cap.clone());
        }
        held
    }

    /// Forgets an extension entirely, on uninstall.
    pub fn forget(&mut self, extension: &ExtensionId) {
        self.extensions.remove(extension);
    }

    /// The question a host asks before every privileged action.
    pub fn check(
        &self,
        extension: &ExtensionId,
        cap: &Capability,
    ) -> Result<CapabilityGrant, Denial> {
        let deny = |reason| Denial {
            extension: extension.clone(),
            capability: cap.clone(),
            reason,
        };
        let Some(state) = self.extensions.get(extension) else {
            return Err(deny(DenialReason::UnknownExtension));
        };
        if state.granted.contains(cap) {
            return Ok(CapabilityGrant {
                extension: extension.clone(),
                capability: cap.clone(),
            });
        }
        if state.revoked.contains(cap) {
            return Err(deny(DenialReason::Revoked));
        }
        if !state.declared.contains(cap) {
            return Err(deny(DenialReason::NotDeclared));
        }
        if !cap.is_known() {
            return Err(deny(DenialReason::UnknownCapability));
        }
        Err(deny(DenialReason::NotGranted))
    }

    /// Convenience predicate. Prefer [`check`](Self::check): the denial carries the
    /// reason, and a `bool` throws it away.
    #[must_use]
    pub fn holds(&self, extension: &ExtensionId, cap: &Capability) -> bool {
        self.check(extension, cap).is_ok()
    }

    /// Checks several capabilities at once, returning every denial rather than the first,
    /// so a consent prompt can list them all.
    pub fn check_all<'a>(
        &self,
        extension: &ExtensionId,
        caps: impl IntoIterator<Item = &'a Capability>,
    ) -> Result<(), Vec<Denial>> {
        let denials: Vec<Denial> = caps
            .into_iter()
            .filter_map(|c| self.check(extension, c).err())
            .collect();
        if denials.is_empty() {
            Ok(())
        } else {
            Err(denials)
        }
    }

    /// The recorded state for one extension.
    #[must_use]
    pub fn state(&self, extension: &ExtensionId) -> Option<&CapabilityState> {
        self.extensions.get(extension)
    }

    /// Every registered extension.
    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionId> {
        self.extensions.keys()
    }
}
