//! The individual diagnostic checks.
//!
//! Every function here is a pure function of its arguments. Nothing reads the
//! process environment, touches the real filesystem or opens the real session
//! bus: those arrive as [`Env`](crate::doctor::env::Env),
//! [`FsProbe`](crate::doctor::fs::FsProbe) and
//! [`BusProbe`](crate::doctor::bus::BusProbe). That is what makes "no session
//! bus, no portal, GNOME with the extension uninstalled" something a unit test
//! can construct in three lines, and it is why `PLAN.md` §8.6's demand that
//! absence be detected as accurately as presence is checkable at all.
//!
//! Each function takes the *narrowest* inputs it needs rather than a single
//! god-context, so a test for the session-type check cannot accidentally
//! depend on the bus.
//!
//! # Where a check lives
//!
//! One submodule per subsystem, so adding a probe for one does not mean
//! editing a file that holds the other three:
//!
//! * [`session`] — session type, runtime directory, IPC socket.
//! * [`portal`] — session bus reachability and the XDG desktop portal.
//! * [`desktop`] — which desktop this is, and our GNOME Shell extension.
//! * [`sandbox`] — Flatpak detection and the application directories.
//! * [`input`] — the input server behind snippet keyword expansion.
//! * [`a11y`] — whether a screen reader is on, and what of the launcher it can read.
//! * [`wlroots`](mod@wlroots) — the wlroots protocols and compositor IPC this session has.
//! * [`clipboard`] — clipboard history and the keyring its key is kept in.
//! * [`config`](mod@config) — whether `compass.json` reads as its author meant.
//!
//! Every check is re-exported here, so a caller says `checks::session_type`
//! without knowing or caring which file it is in. `doctor::mod` assembles the
//! report from those flat paths and is unchanged by this layout.

pub mod a11y;
pub mod clipboard;
pub mod config;
pub mod desktop;
pub mod input;
pub mod portal;
pub mod sandbox;
pub mod session;
pub mod wlroots;

pub use a11y::{A11Y_BUS_NAME, A11Y_OBJECT_PATH, A11Y_STATUS_INTERFACE, screen_reader};
pub use clipboard::{
    COLLECTION_INTERFACE, DEFAULT_COLLECTION_PATH, SECRET_SERVICE_BUS_NAME, clipboard_history,
    keyring,
};
pub use config::{ConfigFacts, config_file};
pub use desktop::{
    EXTENSION_CONTRACT_VERSION, EXTENSION_DEGRADATION, EXTENSION_INTERFACE, EXTENSION_OBJECT_PATH,
    GNOME_SHELL_BUS_NAME, GNOME_SHELL_OBJECT_PATH, KWIN_BUS_NAME, LEGACY_WINDOWS_INTERFACE,
    desktop_environment, is_gnome, is_kde, kwin, shell_extension,
};
pub use input::{InputServerFacts, input_server};
pub use portal::{
    GLOBAL_SHORTCUTS_INTERFACE, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, desktop_portal,
    global_shortcuts, session_bus,
};
pub use sandbox::{FLATPAK_INFO_PATH, application_dir_paths, application_dirs, flatpak};
pub use session::{ipc_socket, runtime_dir, session_type};
pub use wlroots::{WaylandFindings, wlroots};

use compass_ipc::{DoctorCheck, DoctorStatus};

use crate::engine::Engine;

pub(super) fn check(name: &str, status: DoctorStatus, detail: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status,
        detail: Some(detail.into()),
    }
}

/// Which engine this invocation selected, and whether this binary can serve it.
#[must_use]
pub fn engine(selected: Engine) -> DoctorCheck {
    match selected {
        Engine::Rust => check(
            "engine.selected",
            DoctorStatus::Ok,
            "the engine built into this program",
        ),
        Engine::Cpp => check(
            "engine.selected",
            DoctorStatus::Warn,
            "COMPASS_ENGINE asks for the old Vicinae engine, which Compass no longer \
             includes, so commands that need the engine refuse to run. Unset \
             COMPASS_ENGINE to use the engine built into this program",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared by every submodule's tests: a check's detail, or the empty string.
    pub(super) fn detail(check: &DoctorCheck) -> String {
        check.detail.clone().unwrap_or_default()
    }

    // ----- engine.selected ------------------------------------------------

    #[test]
    fn engine_rust_passes() {
        let c = engine(Engine::Rust);
        assert_eq!(c.name, "engine.selected");
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("built into"));
    }

    #[test]
    fn engine_cpp_warns_and_says_why() {
        let c = engine(Engine::Cpp);
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("Unset COMPASS_ENGINE"));
        assert!(!d.contains("C++"));
    }
}
