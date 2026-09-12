//! `compass-portals` — the XDG desktop portals Compass needs, and an honest
//! account of whether they are there.
//!
//! # Why this crate exists
//!
//! On our first target, GNOME 50/51, the **GlobalShortcuts portal is the only
//! hotkey mechanism that works**. `xdg-desktop-portal-gnome` added the backend
//! in 48.rc, improved it in 49.beta, fixed activation-token delivery in
//! 50.alpha and is still fixing it in 51.rc. There is no X11 grab on Wayland,
//! Mutter implements no hotkey protocol, and this crate deliberately does not
//! contain the `xx-hotkey-v1` or X11 backends the C++ build has — those are
//! for other compositors (`PLAN.md` §3.4, `REFERENCES.md` §3.5).
//!
//! The flip side is that `xdg-desktop-portal-wlr` ships **no** GlobalShortcuts
//! backend at all. On Sway, river and labwc a portal frontend is running and
//! owns `org.freedesktop.portal.Desktop`, but the interface does not exist. So
//! the launcher must degrade there, not fail — and detecting that requires
//! reading the interface's `version` property, because the bus name is owned
//! either way. See [`probe`] and [`Availability`].
//!
//! # The availability model
//!
//! [`Availability`] has three arms, and keeping them apart is the point:
//!
//! - [`Availability::Available`] — we asked, and it is there.
//! - [`Availability::Unavailable`] — we asked, and it is definitively not
//!   usable: no frontend at all, the interface is not exported, or it is older
//!   than we can use.
//! - [`Availability::NotQueryable`] — we could not ask: no session bus, the
//!   probe timed out, or the answer was malformed. This says nothing about
//!   whether a backend exists, and `vicinae doctor` must not claim otherwise.
//!
//! [`PortalCapabilities::degraded`] turns that into the list of product
//! features the user actually loses.
//!
//! # Nothing here is fatal
//!
//! [`Portals::connect`] fails only when there is no D-Bus session bus. A
//! missing portal surfaces as [`Availability`], and calling into a missing one
//! anyway yields [`PortalError::Unavailable`], for which
//! [`PortalError::is_unavailable`] is true. The user denying a permission
//! dialog is not an error at all: it is [`ShortcutsOutcome::Denied`],
//! [`OpenOutcome::Dismissed`] or [`FileChooserOutcome::Cancelled`].
//!
//! Every call is bounded by a timeout. A portal request completes only when
//! the backend emits `Response`, which a wedged backend never does, and D-Bus
//! itself imposes no deadline.
//!
//! # What this crate cannot know
//!
//! Probing tells you the interface is exported. It does **not** tell you that
//! a bind will be permitted (the user may deny the dialog — that is
//! [`ShortcutsOutcome::Denied`], only learnable by asking), that the trigger
//! you asked for is the one you will get ([`BoundShortcut::trigger_description`]
//! is the authority), or that the compositor will actually deliver the
//! keypress. GlobalShortcuts is inert under XWayland, and nothing on the bus
//! reveals that.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), compass_portals::PortalError> {
//! use compass_portals::{Modifiers, PortalConfig, Portals, ShortcutDescriptor, Trigger};
//!
//! let portals = Portals::connect(PortalConfig::default()).await?;
//!
//! for feature in portals.capabilities().degraded() {
//!     eprintln!("degraded: {} — {}", feature.title(), feature.explanation());
//! }
//!
//! if portals.capabilities().global_shortcuts.is_available() {
//!     let session = portals.global_shortcuts().await?;
//!     let mut events = session.subscribe();
//!     let outcome = session
//!         .bind(&[ShortcutDescriptor::new("@toggle-launcher", "Toggle Compass")
//!             .with_trigger(Trigger::new(Modifiers::LOGO, "space")?)])
//!         .await?;
//!     println!("{outcome:?}");
//!     while let Some(event) = events.recv().await {
//!         println!("{event:?}");
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

pub mod availability;
pub mod error;
pub mod file_chooser;
pub mod open_uri;
pub mod probe;
pub mod shortcuts;

use std::sync::RwLock;
use std::time::Duration;

pub use availability::{
    Availability, CONFIGURE_SHORTCUTS_MIN_VERSION, DESKTOP_DESTINATION, DESKTOP_PATH,
    DegradedFeature, FILE_CHOOSER, GLOBAL_SHORTCUTS, NotQueryable, OPEN_URI, PortalCapabilities,
    PortalInterface, SCHEME_SUPPORTED_MIN_VERSION, Unavailable,
};
pub use error::{PortalError, Result};
pub use file_chooser::{FileChooserOutcome, FileChooserPortal, FileChooserRequest};
pub use open_uri::{OpenOutcome, OpenUriPortal};
pub use shortcuts::{
    BoundShortcut, GlobalShortcutsSession, Modifiers, ShortcutDescriptor, ShortcutEvent,
    ShortcutEvents, ShortcutsOutcome, Trigger, TriggerParseError,
};

/// Where to find the bus, and how long to wait.
#[derive(Debug, Clone)]
pub struct PortalConfig {
    /// Explicit bus address. `None` means the ambient session bus.
    ///
    /// Tests use this to run against a private `dbus-daemon` without ever
    /// consulting `DBUS_SESSION_BUS_ADDRESS`.
    pub address: Option<String>,
    /// Deadline for a portal round trip that involves no human: version
    /// probes, session creation, binding.
    ///
    /// Binding *can* show a permission dialog on first run, so this needs to
    /// be generous rather than snappy.
    pub call_timeout: Duration,
    /// Deadline for a call that shows a dialog the user must answer, such as
    /// the file chooser. A hang detector, not an impatience detector.
    pub dialog_timeout: Duration,
}

impl Default for PortalConfig {
    fn default() -> Self {
        Self {
            address: None,
            call_timeout: Duration::from_secs(30),
            dialog_timeout: Duration::from_secs(600),
        }
    }
}

impl PortalConfig {
    /// Point the client at a specific bus address.
    #[must_use]
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Override the non-interactive deadline.
    #[must_use]
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Override the interactive deadline.
    #[must_use]
    pub fn with_dialog_timeout(mut self, timeout: Duration) -> Self {
        self.dialog_timeout = timeout;
        self
    }
}

/// Entry point: a session-bus connection plus what the portals on it can do.
#[derive(Debug)]
pub struct Portals {
    conn: zbus::Connection,
    config: PortalConfig,
    caps: RwLock<PortalCapabilities>,
}

impl Portals {
    /// Connect to the session bus and probe every interface this crate speaks.
    ///
    /// Fails only if there is no session bus. A bus with no portal frontend on
    /// it connects successfully and reports everything unavailable.
    pub async fn connect(config: PortalConfig) -> Result<Self> {
        let conn = match &config.address {
            Some(address) => zbus::connection::Builder::address(address.as_str())
                .map_err(PortalError::Bus)?
                .build()
                .await
                .map_err(PortalError::Bus)?,
            None => zbus::Connection::session()
                .await
                .map_err(PortalError::Bus)?,
        };
        let portals = Self {
            conn,
            config,
            caps: RwLock::new(PortalCapabilities::none()),
        };
        portals.probe().await;
        Ok(portals)
    }

    /// The underlying connection, for callers that need to share it.
    pub fn connection(&self) -> &zbus::Connection {
        &self.conn
    }

    /// The configuration in force.
    pub fn config(&self) -> &PortalConfig {
        &self.config
    }

    /// Availability as of the last probe.
    pub fn capabilities(&self) -> PortalCapabilities {
        self.caps
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Re-probe every interface and update [`Self::capabilities`].
    ///
    /// Worth calling after the portal frontend restarts, which happens when a
    /// backend is installed or a `portals.conf` changes.
    pub async fn probe(&self) -> PortalCapabilities {
        let next = PortalCapabilities {
            global_shortcuts: self.availability_of(GLOBAL_SHORTCUTS).await,
            open_uri: self.availability_of(OPEN_URI).await,
            file_chooser: self.availability_of(FILE_CHOOSER).await,
        };
        let mut guard = self
            .caps
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *guard != next {
            tracing::info!(
                global_shortcuts = %next.global_shortcuts,
                open_uri = %next.open_uri,
                file_chooser = %next.file_chooser,
                "portal availability changed"
            );
            *guard = next.clone();
        }
        next
    }

    /// Probe one interface without touching the cache.
    pub async fn availability_of(&self, interface: PortalInterface) -> Availability {
        probe::probe(&self.conn, interface, self.config.call_timeout).await
    }

    /// Open a GlobalShortcuts session.
    ///
    /// Gated on the cached probe: if the interface is not there this returns
    /// [`PortalError::Unavailable`] without touching the bus, because `ashpd`
    /// would otherwise happily build a proxy against a name nobody exports and
    /// fail later, or hang.
    pub async fn global_shortcuts(&self) -> Result<GlobalShortcutsSession> {
        let availability = self.capabilities().global_shortcuts;
        let Some(version) = availability.version() else {
            return Err(PortalError::unavailable(GLOBAL_SHORTCUTS, availability));
        };
        GlobalShortcutsSession::create(self.conn.clone(), version, self.config.call_timeout).await
    }

    /// The OpenURI client, if the interface is available.
    pub fn open_uri(&self) -> Result<OpenUriPortal> {
        let availability = self.capabilities().open_uri;
        if !availability.is_available() {
            return Err(PortalError::unavailable(OPEN_URI, availability));
        }
        let version = availability.version().expect("checked available above");
        Ok(OpenUriPortal::new(
            self.conn.clone(),
            self.config.call_timeout,
            version,
        ))
    }

    /// The FileChooser client, if the interface is available.
    pub fn file_chooser(&self) -> Result<FileChooserPortal> {
        let availability = self.capabilities().file_chooser;
        if !availability.is_available() {
            return Err(PortalError::unavailable(FILE_CHOOSER, availability));
        }
        Ok(FileChooserPortal::new(
            self.conn.clone(),
            self.config.dialog_timeout,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pre_probe_state_is_fully_degraded_and_explains_itself() {
        let caps = PortalCapabilities::none();
        assert!(!caps.any_available());
        let degraded = caps.degraded();
        assert_eq!(degraded.len(), 3);
        for feature in degraded {
            assert!(!feature.title().is_empty());
            assert!(feature.explanation().len() > 40, "{feature:?}");
        }
    }

    #[test]
    fn availability_distinguishes_absence_from_ignorance() {
        let absent = Availability::Unavailable {
            reason: Unavailable::InterfaceMissing,
        };
        let unknown = Availability::NotQueryable {
            reason: NotQueryable::ProbeTimedOut {
                timeout: Duration::from_secs(1),
            },
        };
        assert_ne!(absent, unknown);
        assert!(!absent.is_indeterminate());
        assert!(unknown.is_indeterminate());
        assert!(!absent.is_available());
        assert!(!unknown.is_available());
    }

    #[test]
    fn version_gating_is_explicit() {
        let v1 = Availability::Available { version: 1 };
        assert!(v1.supports(1));
        assert!(!v1.supports(CONFIGURE_SHORTCUTS_MIN_VERSION));
        assert!(Availability::Available { version: 5 }.supports(SCHEME_SUPPORTED_MIN_VERSION));
    }
}
