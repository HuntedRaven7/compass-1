//! `org.freedesktop.portal.GlobalShortcuts`.
//!
//! Ported from the intent of `src/server/src/services/global-shortcuts/`: the
//! C++ side models a bind as `{id, trigger, description}`
//! (`GlobalShortcutRequest`) and reports activations as `(id, timestamp)` from
//! an abstract backend that can declare itself unsupported. The same shape is
//! kept here, with the platform detail swapped: on GNOME 50/51 the portal is
//! the mechanism, and there is no X11 grab or `xx-hotkey-v1` path in this
//! crate at all.
//!
//! `ashpd` types are deliberately kept inside this module. Callers see
//! [`ShortcutDescriptor`], [`Trigger`], [`BoundShortcut`], [`ShortcutEvent`]
//! and [`ShortcutsOutcome`].

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;

use ashpd::desktop::global_shortcuts::{
    BindShortcutsOptions, ConfigureShortcutsOptions, GlobalShortcuts as AshpdGlobalShortcuts,
    ListShortcutsOptions, NewShortcut, Shortcut as AshpdShortcut,
};
use ashpd::desktop::{CreateSessionOptions, ResponseError, Session as AshpdSession};
use tokio::sync::broadcast;
use zbus::export::futures_core::Stream;
use zbus::zvariant::OwnedValue;

use crate::availability::{CONFIGURE_SHORTCUTS_MIN_VERSION, GLOBAL_SHORTCUTS};
use crate::error::{PortalError, Result};

/// Keyboard modifiers, as the "shortcuts" XDG specification spells them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// `CTRL`.
    pub ctrl: bool,
    /// `ALT`.
    pub alt: bool,
    /// `SHIFT`.
    pub shift: bool,
    /// `LOGO` — the Super/Windows/Command key.
    pub logo: bool,
}

impl Modifiers {
    /// No modifiers at all.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    };

    /// Just `LOGO`, the usual launcher modifier.
    pub const LOGO: Self = Self {
        logo: true,
        ..Self::NONE
    };

    /// True when no modifier is set.
    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (set, name) in [
            (self.ctrl, "CTRL"),
            (self.alt, "ALT"),
            (self.shift, "SHIFT"),
            (self.logo, "LOGO"),
        ] {
            if set {
                write!(f, "{name}+")?;
            }
        }
        Ok(())
    }
}

/// A key combination, in the syntax the portal expects for
/// `preferred_trigger`: modifiers joined by `+`, then one key name, e.g.
/// `LOGO+space` or `CTRL+SHIFT+a`.
///
/// The key part is an XKB keysym name and is passed through verbatim; this
/// crate does not attempt to validate it against a keysym table, because the
/// portal's own parser is the authority and it varies by backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Trigger {
    /// Modifiers held down.
    pub modifiers: Modifiers,
    /// Key name, e.g. `space`, `a`, `F1`.
    pub key: String,
}

impl Trigger {
    /// Build a trigger, rejecting an empty or `+`-containing key name.
    pub fn new(
        modifiers: Modifiers,
        key: impl Into<String>,
    ) -> std::result::Result<Self, TriggerParseError> {
        let key = key.into();
        if key.is_empty() {
            return Err(TriggerParseError::NoKey(modifiers.to_string()));
        }
        if key.contains('+') {
            return Err(TriggerParseError::MultipleKeys(key));
        }
        if parse_modifier(&key).is_some() {
            return Err(TriggerParseError::NoKey(format!("{modifiers}{key}")));
        }
        Ok(Self { modifiers, key })
    }

    /// Parse `CTRL+SHIFT+a`-style syntax.
    ///
    /// Modifier names are matched case-insensitively, and `SUPER`, `META` and
    /// `WIN` are accepted as spellings of `LOGO` because users type them; the
    /// canonical rendering is always `LOGO`.
    pub fn parse(input: &str) -> std::result::Result<Self, TriggerParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(TriggerParseError::Empty);
        }
        let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().expect("split always yields one part");

        let mut modifiers = Modifiers::NONE;
        for part in mods {
            match parse_modifier(part) {
                Some(apply) => apply(&mut modifiers),
                None => return Err(TriggerParseError::UnknownModifier((*part).to_owned())),
            }
        }
        Self::new(modifiers, *key)
    }
}

type ModifierSetter = fn(&mut Modifiers);

fn parse_modifier(name: &str) -> Option<ModifierSetter> {
    let setter: ModifierSetter = match name.to_ascii_uppercase().as_str() {
        "CTRL" | "CONTROL" => |m: &mut Modifiers| m.ctrl = true,
        "ALT" => |m: &mut Modifiers| m.alt = true,
        "SHIFT" => |m: &mut Modifiers| m.shift = true,
        "LOGO" | "SUPER" | "META" | "WIN" => |m: &mut Modifiers| m.logo = true,
        _ => return None,
    };
    Some(setter)
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.modifiers, self.key)
    }
}

impl FromStr for Trigger {
    type Err = TriggerParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Why a trigger string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TriggerParseError {
    /// The string was empty or whitespace.
    #[error("shortcut trigger is empty")]
    Empty,
    /// Modifiers were given but no key.
    #[error("shortcut trigger `{0}` has modifiers but no key")]
    NoKey(String),
    /// A `+`-separated component was not a known modifier.
    #[error("unknown modifier `{0}` in shortcut trigger")]
    UnknownModifier(String),
    /// More than one key name was given.
    #[error("shortcut trigger names more than one key: `{0}`")]
    MultipleKeys(String),
}

/// A shortcut we would like the desktop to bind for us.
///
/// The `preferred_trigger` really is only a preference. The portal, and on
/// GNOME the user, may assign something else entirely; the authoritative
/// answer comes back as [`BoundShortcut::trigger_description`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutDescriptor {
    /// Application-chosen stable id, echoed back on activation.
    pub id: String,
    /// User-readable description shown in the desktop's shortcut settings.
    pub description: String,
    /// Preferred key combination, if any.
    pub preferred_trigger: Option<Trigger>,
}

impl ShortcutDescriptor {
    /// A shortcut with no preferred trigger.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            preferred_trigger: None,
        }
    }

    /// Set the preferred trigger.
    #[must_use]
    pub fn with_trigger(mut self, trigger: Trigger) -> Self {
        self.preferred_trigger = Some(trigger);
        self
    }

    fn to_ashpd(&self) -> NewShortcut {
        let shortcut = NewShortcut::new(self.id.clone(), self.description.clone());
        match &self.preferred_trigger {
            Some(trigger) => shortcut.preferred_trigger(trigger.to_string().as_str()),
            None => shortcut,
        }
    }
}

/// A shortcut the desktop has actually bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundShortcut {
    /// The id we asked for.
    pub id: String,
    /// Description as the portal echoes it.
    pub description: String,
    /// How the desktop says the user triggers it, for display. This is the
    /// only trustworthy answer: it may differ from the preferred trigger, and
    /// on some backends it is empty until the user assigns one.
    pub trigger_description: String,
}

impl From<&AshpdShortcut> for BoundShortcut {
    fn from(value: &AshpdShortcut) -> Self {
        Self {
            id: value.id().to_owned(),
            description: value.description().to_owned(),
            trigger_description: value.trigger_description().to_owned(),
        }
    }
}

/// Outcome of a bind or list request.
///
/// A user who dismisses the first-run permission dialog is not an error: it is
/// [`ShortcutsOutcome::Denied`], which callers can match on and report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShortcutsOutcome {
    /// The request succeeded, with the shortcuts the desktop bound.
    Granted {
        /// Bound shortcuts, in the portal's order.
        shortcuts: Vec<BoundShortcut>,
    },
    /// The user cancelled or denied the request (portal response code 1).
    Denied,
    /// The portal ended the request unsuccessfully without saying why
    /// (response code 2).
    Refused,
}

impl ShortcutsOutcome {
    /// The bound shortcuts, or an empty slice if the request did not succeed.
    pub fn shortcuts(&self) -> &[BoundShortcut] {
        match self {
            Self::Granted { shortcuts } => shortcuts,
            _ => &[],
        }
    }

    /// True when the request succeeded.
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted { .. })
    }
}

/// Something the desktop reported about our shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShortcutEvent {
    /// A shortcut was pressed. Mirrors the C++ `shortcutActivated(id, timestamp)`.
    Activated {
        /// The id we registered.
        id: String,
        /// Time since the Unix epoch, as the portal reports it.
        timestamp: Duration,
        /// `xdg-activation-v1` token, when the compositor supplied one.
        /// `xdg-desktop-portal-gnome` only started delivering this reliably in
        /// 50.alpha, so treat `None` as normal.
        activation_token: Option<String>,
    },
    /// A shortcut was released.
    Deactivated {
        /// The id we registered.
        id: String,
        /// Time since the Unix epoch, as the portal reports it.
        timestamp: Duration,
    },
    /// The desktop changed what our shortcuts are bound to — typically the
    /// user editing them in system settings.
    Changed {
        /// The new state of every shortcut in this session.
        shortcuts: Vec<BoundShortcut>,
    },
}

/// A subscription to [`ShortcutEvent`]s.
pub struct ShortcutEvents(broadcast::Receiver<ShortcutEvent>);

impl ShortcutEvents {
    /// Await the next event, or `None` once the session is dropped.
    ///
    /// A subscriber that falls too far behind loses the intervening events and
    /// a warning is logged; it does not disconnect. Missing a keypress is
    /// better than wedging the session.
    pub async fn recv(&mut self) -> Option<ShortcutEvent> {
        loop {
            match self.0.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "global shortcut subscriber lagged");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl fmt::Debug for ShortcutEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShortcutEvents")
    }
}

const EVENT_BUFFER: usize = 64;

/// A live GlobalShortcuts session.
///
/// Dropping it stops the signal forwarders. Call [`Self::close`] to also tell
/// the portal the session is over.
pub struct GlobalShortcutsSession {
    portal: AshpdGlobalShortcuts,
    session: AshpdSession<AshpdGlobalShortcuts>,
    events: broadcast::Sender<ShortcutEvent>,
    forwarders: Vec<tokio::task::JoinHandle<()>>,
    timeout: Duration,
    version: u32,
}

impl fmt::Debug for GlobalShortcutsSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlobalShortcutsSession")
            .field("version", &self.version)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl GlobalShortcutsSession {
    pub(crate) async fn create(
        conn: zbus::Connection,
        version: u32,
        timeout: Duration,
    ) -> Result<Self> {
        let portal = bounded(
            timeout,
            "GlobalShortcuts",
            AshpdGlobalShortcuts::with_connection(conn),
        )
        .await?;

        // Subscribe before creating the session, so an activation that races
        // session creation is not lost.
        let activated = bounded(timeout, "Activated", portal.receive_activated()).await?;
        let deactivated = bounded(timeout, "Deactivated", portal.receive_deactivated()).await?;
        let changed = bounded(
            timeout,
            "ShortcutsChanged",
            portal.receive_shortcuts_changed(),
        )
        .await?;

        let (events, _) = broadcast::channel(EVENT_BUFFER);

        let forwarders = vec![
            spawn_forwarder(activated, events.clone(), |a| ShortcutEvent::Activated {
                id: a.shortcut_id().to_owned(),
                timestamp: a.timestamp(),
                activation_token: activation_token(a.options()),
            }),
            spawn_forwarder(deactivated, events.clone(), |d| {
                ShortcutEvent::Deactivated {
                    id: d.shortcut_id().to_owned(),
                    timestamp: d.timestamp(),
                }
            }),
            spawn_forwarder(changed, events.clone(), |c| ShortcutEvent::Changed {
                shortcuts: c.shortcuts().iter().map(BoundShortcut::from).collect(),
            }),
        ];

        let session = bounded(
            timeout,
            "CreateSession",
            portal.create_session(CreateSessionOptions::default()),
        )
        .await?;

        Ok(Self {
            portal,
            session,
            events,
            forwarders,
            timeout,
            version,
        })
    }

    /// Interface version the portal reported.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Subscribe to activations. Events emitted before the first subscriber
    /// exists are dropped, so subscribe before binding.
    pub fn subscribe(&self) -> ShortcutEvents {
        ShortcutEvents(self.events.subscribe())
    }

    /// Bind shortcuts, showing the desktop's permission dialog on first run.
    ///
    /// The returned [`ShortcutsOutcome`] distinguishes success from the user
    /// denying the dialog. Both are normal.
    pub async fn bind(&self, shortcuts: &[ShortcutDescriptor]) -> Result<ShortcutsOutcome> {
        let requested: Vec<NewShortcut> =
            shortcuts.iter().map(ShortcutDescriptor::to_ashpd).collect();
        let request = bounded(
            self.timeout,
            "BindShortcuts",
            self.portal.bind_shortcuts(
                &self.session,
                &requested,
                None,
                BindShortcutsOptions::default(),
            ),
        )
        .await?;
        outcome(
            "BindShortcuts",
            request
                .response()
                .map(|r| r.shortcuts().iter().map(BoundShortcut::from).collect()),
        )
    }

    /// List the shortcuts currently bound to this session.
    pub async fn list(&self) -> Result<ShortcutsOutcome> {
        let request = bounded(
            self.timeout,
            "ListShortcuts",
            self.portal
                .list_shortcuts(&self.session, ListShortcutsOptions::default()),
        )
        .await?;
        outcome(
            "ListShortcuts",
            request
                .response()
                .map(|r| r.shortcuts().iter().map(BoundShortcut::from).collect()),
        )
    }

    /// Ask the desktop to show its shortcut-configuration UI.
    ///
    /// Requires interface v2; on v1 this returns
    /// [`PortalError::VersionTooOld`] without making a call.
    pub async fn configure(&self) -> Result<()> {
        if self.version < CONFIGURE_SHORTCUTS_MIN_VERSION {
            return Err(PortalError::VersionTooOld {
                interface: GLOBAL_SHORTCUTS.name,
                method: "ConfigureShortcuts",
                found: self.version,
                required: CONFIGURE_SHORTCUTS_MIN_VERSION,
            });
        }
        bounded(
            self.timeout,
            "ConfigureShortcuts",
            self.portal.configure_shortcuts(
                &self.session,
                None,
                ConfigureShortcutsOptions::default(),
            ),
        )
        .await
    }

    /// Close the session with the portal, unbinding every shortcut.
    pub async fn close(self) -> Result<()> {
        bounded(self.timeout, "Session.Close", self.session.close()).await
    }
}

impl Drop for GlobalShortcutsSession {
    fn drop(&mut self) {
        for handle in &self.forwarders {
            handle.abort();
        }
    }
}

fn activation_token(options: &HashMap<String, OwnedValue>) -> Option<String> {
    options
        .get("activation_token")
        .and_then(|value| <&str>::try_from(value).ok())
        .map(ToOwned::to_owned)
}

fn outcome(
    method: &'static str,
    response: std::result::Result<Vec<BoundShortcut>, ashpd::Error>,
) -> Result<ShortcutsOutcome> {
    match response {
        Ok(shortcuts) => Ok(ShortcutsOutcome::Granted { shortcuts }),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(ShortcutsOutcome::Denied),
        Err(ashpd::Error::Response(ResponseError::Other)) => Ok(ShortcutsOutcome::Refused),
        Err(source) => Err(PortalError::call(method, source)),
    }
}

fn spawn_forwarder<S, T>(
    stream: S,
    events: broadcast::Sender<ShortcutEvent>,
    map: fn(&T) -> ShortcutEvent,
) -> tokio::task::JoinHandle<()>
where
    S: Stream<Item = T> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        while let Some(item) = next(&mut stream).await {
            // An error only means nobody is subscribed right now.
            let _ = events.send(map(&item));
        }
    })
}

async fn next<S: Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    std::future::poll_fn(|cx| Pin::new(&mut *stream).poll_next(cx)).await
}

/// Bound one portal round trip.
pub(crate) async fn bounded<T>(
    timeout: Duration,
    method: &'static str,
    fut: impl Future<Output = std::result::Result<T, ashpd::Error>>,
) -> Result<T> {
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(source)) => Err(PortalError::call(method, source)),
        Err(_) => Err(PortalError::Timeout { method, timeout }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_key_parses() {
        let trigger = Trigger::parse("space").expect("parses");
        assert_eq!(trigger.modifiers, Modifiers::NONE);
        assert_eq!(trigger.key, "space");
        assert_eq!(trigger.to_string(), "space");
    }

    #[test]
    fn modifiers_round_trip_in_canonical_order() {
        let trigger = Trigger::parse("shift+ctrl+alt+logo+a").expect("parses");
        assert_eq!(trigger.to_string(), "CTRL+ALT+SHIFT+LOGO+a");
        assert_eq!(Trigger::parse(&trigger.to_string()).unwrap(), trigger);
    }

    #[test]
    fn super_is_accepted_and_normalised_to_logo() {
        for spelling in ["SUPER+space", "super+space", "Meta+space", "WIN+space"] {
            let trigger = Trigger::parse(spelling).expect(spelling);
            assert_eq!(trigger.to_string(), "LOGO+space");
        }
    }

    #[test]
    fn key_case_is_preserved_because_keysyms_are_case_sensitive() {
        assert_eq!(Trigger::parse("CTRL+F1").unwrap().key, "F1");
    }

    #[test]
    fn bad_triggers_are_errors_not_panics() {
        assert_eq!(Trigger::parse(""), Err(TriggerParseError::Empty));
        assert_eq!(Trigger::parse("   "), Err(TriggerParseError::Empty));
        assert!(matches!(
            Trigger::parse("HYPER+a"),
            Err(TriggerParseError::UnknownModifier(_))
        ));
        assert!(matches!(
            Trigger::parse("CTRL+SHIFT"),
            Err(TriggerParseError::NoKey(_))
        ));
        assert!(matches!(
            Trigger::parse("CTRL+"),
            Err(TriggerParseError::NoKey(_))
        ));
        assert!(matches!(
            Trigger::new(Modifiers::LOGO, "a+b"),
            Err(TriggerParseError::MultipleKeys(_))
        ));
    }

    #[test]
    fn outcomes_classify_the_portal_response_code() {
        assert!(matches!(
            outcome("BindShortcuts", Ok(Vec::new())),
            Ok(ShortcutsOutcome::Granted { .. })
        ));
        assert_eq!(
            outcome(
                "BindShortcuts",
                Err(ashpd::Error::Response(ResponseError::Cancelled))
            )
            .unwrap(),
            ShortcutsOutcome::Denied
        );
        assert_eq!(
            outcome(
                "BindShortcuts",
                Err(ashpd::Error::Response(ResponseError::Other))
            )
            .unwrap(),
            ShortcutsOutcome::Refused
        );
        assert!(matches!(
            outcome("BindShortcuts", Err(ashpd::Error::NoResponse)),
            Err(PortalError::Call { .. })
        ));
    }

    #[test]
    fn a_denied_outcome_carries_no_shortcuts() {
        assert!(ShortcutsOutcome::Denied.shortcuts().is_empty());
        assert!(!ShortcutsOutcome::Denied.is_granted());
    }
}
