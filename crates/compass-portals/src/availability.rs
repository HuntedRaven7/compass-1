//! Whether a given XDG desktop portal interface is usable, and what we lose
//! when it is not.
//!
//! Every portal this crate speaks is optional. `PLAN.md` §3.5.1 requires that
//! nothing in the critical path hard-fails when a portal is missing, and
//! §3.4/`REFERENCES.md` §3.5 make the reason concrete: on GNOME 50/51 the
//! GlobalShortcuts backend exists and is the only hotkey path, while
//! `xdg-desktop-portal-wlr` ships **no** GlobalShortcuts backend at all. On
//! Sway, river and labwc a portal *frontend* is running and owns
//! `org.freedesktop.portal.Desktop`, but the interface is simply not exported.
//!
//! That is why availability is decided by reading the interface's `version`
//! property and not by asking whether a bus name is owned: the bus name is
//! owned in both the working and the broken case.

use std::time::Duration;

/// One portal interface, with the version range this crate understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortalInterface {
    /// Fully-qualified D-Bus interface name.
    pub name: &'static str,
    /// Lowest interface version this crate can do anything useful with.
    pub minimum_version: u32,
    /// Newest version this crate was written against. A portal reporting more
    /// than this is still [`Availability::Available`] — portal interfaces are
    /// additive — but it is worth saying so in `vicinae doctor`.
    pub newest_known_version: u32,
}

impl PortalInterface {
    /// Whether `version` is one this crate was actually written against.
    pub fn is_known_version(self, version: u32) -> bool {
        version >= self.minimum_version && version <= self.newest_known_version
    }
}

/// `org.freedesktop.portal.GlobalShortcuts`.
///
/// Added to `xdg-desktop-portal-gnome` in 48.rc; version 2 added
/// `ConfigureShortcuts`.
pub const GLOBAL_SHORTCUTS: PortalInterface = PortalInterface {
    name: "org.freedesktop.portal.GlobalShortcuts",
    minimum_version: 1,
    newest_known_version: 2,
};

/// `org.freedesktop.portal.OpenURI`.
pub const OPEN_URI: PortalInterface = PortalInterface {
    name: "org.freedesktop.portal.OpenURI",
    minimum_version: 1,
    newest_known_version: 5,
};

/// `org.freedesktop.portal.FileChooser`.
pub const FILE_CHOOSER: PortalInterface = PortalInterface {
    name: "org.freedesktop.portal.FileChooser",
    minimum_version: 1,
    newest_known_version: 4,
};

/// Interface version required by `GlobalShortcuts.ConfigureShortcuts`.
pub const CONFIGURE_SHORTCUTS_MIN_VERSION: u32 = 2;

/// Interface version required by `OpenURI.SchemeSupported`.
pub const SCHEME_SUPPORTED_MIN_VERSION: u32 = 5;

/// The well-known name every desktop portal interface lives behind.
pub const DESKTOP_DESTINATION: &str = "org.freedesktop.portal.Desktop";

/// The object path every desktop portal interface lives at.
pub const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";

/// Result of probing one portal interface.
///
/// The three arms answer three different questions, and conflating them is the
/// thing this type exists to prevent:
///
/// - [`Availability::Available`] — we asked, and it is there.
/// - [`Availability::Unavailable`] — we asked, and it is definitively not
///   there (or not there in a usable version).
/// - [`Availability::NotQueryable`] — we could not ask. This is *not* the same
///   as absence: a wedged portal frontend, or no session bus at all, tells us
///   nothing about whether a backend exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Availability {
    /// Present at a version we can use.
    Available {
        /// Version the interface reported.
        version: u32,
    },
    /// Asked and answered: not usable.
    Unavailable {
        /// Why.
        reason: Unavailable,
    },
    /// The question could not be put.
    NotQueryable {
        /// Why.
        reason: NotQueryable,
    },
}

impl Availability {
    /// True only when calls against this interface are worth attempting.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    /// The reported interface version, when there is one.
    pub fn version(&self) -> Option<u32> {
        match self {
            Self::Available { version } => Some(*version),
            _ => None,
        }
    }

    /// True when the probe itself failed, so the answer is "unknown", not "no".
    pub fn is_indeterminate(&self) -> bool {
        matches!(self, Self::NotQueryable { .. })
    }

    /// True when the interface is available *and* at least `required`.
    ///
    /// Used to gate the handful of calls that need a newer interface than the
    /// floor, such as `ConfigureShortcuts` (v2) and `SchemeSupported` (v5).
    pub fn supports(&self, required: u32) -> bool {
        self.version().is_some_and(|v| v >= required)
    }
}

impl std::fmt::Display for Availability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available { version } => write!(f, "available (interface v{version})"),
            Self::Unavailable { reason } => write!(f, "unavailable: {reason}"),
            Self::NotQueryable { reason } => write!(f, "not queryable: {reason}"),
        }
    }
}

/// Why an interface we successfully asked about is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unavailable {
    /// Nothing owns `org.freedesktop.portal.Desktop`. There is no portal
    /// frontend running at all — a bare session, a broken Flatpak sandbox, or
    /// `xdg-desktop-portal` not installed.
    NoPortalFrontend,
    /// A frontend answered, but does not export this interface. Some backend
    /// would have to provide it and none does.
    ///
    /// This is exactly the wlroots case for GlobalShortcuts:
    /// `xdg-desktop-portal` is running and healthy, and
    /// `xdg-desktop-portal-wlr` implements no GlobalShortcuts backend.
    InterfaceMissing,
    /// Exported, but older than [`PortalInterface::minimum_version`].
    VersionTooOld {
        /// Version the interface reported.
        found: u32,
        /// Version we need.
        required: u32,
    },
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPortalFrontend => {
                f.write_str("no portal frontend owns org.freedesktop.portal.Desktop")
            }
            Self::InterfaceMissing => f.write_str(
                "the portal frontend is running but exports no such interface, so no backend \
                 implements it",
            ),
            Self::VersionTooOld { found, required } => {
                write!(
                    f,
                    "interface is v{found}, this build needs at least v{required}"
                )
            }
        }
    }
}

/// Why we could not determine availability at all.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotQueryable {
    /// The session bus could not be reached.
    NoSessionBus {
        /// Transport error, rendered.
        message: String,
    },
    /// The frontend owns the name but did not answer the probe in time. A
    /// D-Bus method call has no inherent deadline, so this is what a hung
    /// `xdg-desktop-portal` looks like from here.
    ProbeTimedOut {
        /// Deadline that elapsed.
        timeout: Duration,
    },
    /// The probe failed in a way we cannot interpret as presence or absence.
    ProbeFailed {
        /// Error, rendered.
        message: String,
    },
    /// The `version` property came back as something other than a `u`.
    MalformedVersion {
        /// What we got instead.
        message: String,
    },
}

impl std::fmt::Display for NotQueryable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSessionBus { message } => write!(f, "no D-Bus session bus ({message})"),
            Self::ProbeTimedOut { timeout } => {
                write!(
                    f,
                    "the portal did not answer the version probe within {timeout:?}"
                )
            }
            Self::ProbeFailed { message } => write!(f, "version probe failed ({message})"),
            Self::MalformedVersion { message } => {
                write!(f, "the portal reported a malformed version ({message})")
            }
        }
    }
}

/// Availability of every portal interface this crate speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCapabilities {
    /// `org.freedesktop.portal.GlobalShortcuts`.
    pub global_shortcuts: Availability,
    /// `org.freedesktop.portal.OpenURI`.
    pub open_uri: Availability,
    /// `org.freedesktop.portal.FileChooser`.
    pub file_chooser: Availability,
}

impl PortalCapabilities {
    /// The "nothing was found" state, which is also the pre-probe state.
    pub fn none() -> Self {
        let absent = Availability::Unavailable {
            reason: Unavailable::NoPortalFrontend,
        };
        Self {
            global_shortcuts: absent.clone(),
            open_uri: absent.clone(),
            file_chooser: absent,
        }
    }

    /// True when at least one interface is usable.
    pub fn any_available(&self) -> bool {
        self.global_shortcuts.is_available()
            || self.open_uri.is_available()
            || self.file_chooser.is_available()
    }

    /// Product features unusable in this state, for `vicinae doctor`.
    pub fn degraded(&self) -> Vec<DegradedFeature> {
        let mut out = Vec::new();
        if !self.global_shortcuts.is_available() {
            out.push(DegradedFeature::GlobalHotkeys);
        }
        if !self.open_uri.is_available() {
            out.push(DegradedFeature::OpenExternally);
        }
        if !self.file_chooser.is_available() {
            out.push(DegradedFeature::FilePicker);
        }
        out
    }
}

impl Default for PortalCapabilities {
    fn default() -> Self {
        Self::none()
    }
}

/// A product feature lost when a portal is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DegradedFeature {
    /// Binding a launcher hotkey such as Super+Space.
    GlobalHotkeys,
    /// Handing a URL or file to the desktop's default handler.
    OpenExternally,
    /// Asking the user to pick a file or folder.
    FilePicker,
}

impl DegradedFeature {
    /// Short label.
    pub fn title(self) -> &'static str {
        match self {
            Self::GlobalHotkeys => "Global hotkeys",
            Self::OpenExternally => "Open in the default application",
            Self::FilePicker => "File picker",
        }
    }

    /// One-line explanation of what the user actually loses, phrased for
    /// `vicinae doctor`.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::GlobalHotkeys => {
                "Compass cannot bind a launcher hotkey. On GNOME the GlobalShortcuts portal is \
                 the only mechanism that works, so the launcher has to be opened from the CLI, a \
                 desktop-defined shortcut, or another app. Note that xdg-desktop-portal-wlr ships \
                 no GlobalShortcuts backend at all, so this is the expected state on Sway, river \
                 and labwc."
            }
            Self::OpenExternally => {
                "Compass cannot ask the desktop to open a URL or file. Inside a Flatpak sandbox \
                 there is no fallback: spawning a host handler directly requires \
                 flatpak-spawn --host, which is a separate permission."
            }
            Self::FilePicker => {
                "Compass cannot show a file chooser, so commands that ask the user to pick a file \
                 or folder are unavailable. Inside a Flatpak sandbox the portal is also what grants \
                 access to the chosen path, so a built-in browser would not be a substitute."
            }
        }
    }
}
