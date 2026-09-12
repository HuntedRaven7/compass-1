//! The individual diagnostic checks.
//!
//! Every function here is a pure function of its arguments. Nothing reads the
//! process environment, touches the real filesystem or opens the real session
//! bus: those arrive as [`Env`], [`FsProbe`] and [`BusProbe`]. That is what
//! makes "no session bus, no portal, GNOME with the extension uninstalled"
//! something a unit test can construct in three lines, and it is why
//! `PLAN.md` §8.6's demand that absence be detected as accurately as presence
//! is checkable at all.
//!
//! Each function takes the *narrowest* inputs it needs rather than a single
//! god-context, so a test for the session-type check cannot accidentally
//! depend on the bus.

use std::path::{Path, PathBuf};

use compass_ipc::{DoctorCheck, DoctorStatus, SocketPath};

use super::bus::BusProbe;
use super::env::Env;
use super::fs::FsProbe;
use crate::engine::Engine;

/// Bus name of the XDG desktop portal.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
/// Object path the portal exports its interfaces on.
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
/// Interface providing the only hotkey mechanism available on GNOME.
pub const GLOBAL_SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// Bus name GNOME Shell (and therefore every Shell extension) is reached at.
pub const GNOME_SHELL_BUS_NAME: &str = "org.gnome.Shell";
/// Object path of GNOME Shell itself.
pub const GNOME_SHELL_OBJECT_PATH: &str = "/org/gnome/Shell";

/// Object path of the versioned Compass Shell-extension contract (PLAN §3.5.3).
pub const EXTENSION_OBJECT_PATH: &str = "/org/gnome/Shell/Extensions/Vicinae";
/// Interface name of that contract.
pub const EXTENSION_INTERFACE: &str = "org.gnome.Shell.Extensions.Vicinae";
/// Contract version this build speaks.
pub const EXTENSION_CONTRACT_VERSION: u32 = 1;

/// Interface name of the pre-existing, unversioned window-management contract
/// that the C++ engine talks to (`src/server/.../gnome-window-manager.hpp`).
///
/// Recorded for reference only: it exposes no properties, so its presence
/// cannot be probed without an Introspect call, and this build requires the
/// versioned [`EXTENSION_INTERFACE`] regardless.
pub const LEGACY_WINDOWS_INTERFACE: &str = "org.gnome.Shell.Extensions.Windows";

/// Exactly what stops working without the Shell extension, per PLAN §3.5.1.
pub const EXTENSION_DEGRADATION: &str = "window switching, clipboard history and paste are unavailable; app search, launching, \
     calculator, emoji, snippets and extensions are unaffected";

/// Marker file the Flatpak runtime places in every sandbox.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

fn check(name: &str, status: DoctorStatus, detail: impl Into<String>) -> DoctorCheck {
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
            "rust — served by this binary",
        ),
        Engine::Cpp => check(
            "engine.selected",
            DoctorStatus::Warn,
            "cpp — this binary is the Rust engine and cannot dispatch to the C++ one \
             (PLAN.md §5; the dispatching front-end lands with the Phase 7 cutover). \
             Commands that need the engine will refuse; run the C++ `vicinae` directly, \
             or pass --engine rust / COMPASS_ENGINE=rust",
        ),
    }
}

/// Wayland, X11, or no graphical session at all.
#[must_use]
pub fn session_type(env: &Env) -> DoctorCheck {
    const NAME: &str = "session.type";
    match (env.get("WAYLAND_DISPLAY"), env.get("DISPLAY")) {
        (Some(wayland), Some(x11)) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "Wayland (WAYLAND_DISPLAY={wayland}), with XWayland available at DISPLAY={x11}"
            ),
        ),
        (Some(wayland), None) => check(
            NAME,
            DoctorStatus::Ok,
            format!("Wayland (WAYLAND_DISPLAY={wayland})"),
        ),
        (None, Some(x11)) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "X11 only (DISPLAY={x11}, WAYLAND_DISPLAY unset). The Rust engine targets \
                 Wayland; the X11 hotkey backend is Phase 5 work, so the global hotkey will \
                 not bind here"
            ),
        ),
        (None, None) => check(
            NAME,
            DoctorStatus::Fail,
            "no graphical session: neither WAYLAND_DISPLAY nor DISPLAY is set. The CLI and \
             this diagnostic work, but the launcher window cannot be opened",
        ),
    }
}

/// `XDG_RUNTIME_DIR`, and whether the socket fell back because of its absence.
#[must_use]
pub fn runtime_dir(env: &Env, socket: &SocketPath) -> DoctorCheck {
    const NAME: &str = "xdg.runtime-dir";
    match env.get("XDG_RUNTIME_DIR") {
        Some(dir) => check(NAME, DoctorStatus::Ok, format!("XDG_RUNTIME_DIR={dir}")),
        None if socket.is_fallback() => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_RUNTIME_DIR is unset, so the socket falls back to {socket}. That path is \
                 not tmpfs-backed, survives logout and lives under a world-readable /tmp. \
                 Common in bare ssh sessions, cron and minimal containers; inside a normal \
                 desktop session it means the session manager did not set it up"
            ),
        ),
        None => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_RUNTIME_DIR is unset. The socket path was set explicitly ({socket}), so \
                 the /tmp fallback is not in use, but anything else resolving runtime state \
                 from the environment will still fall back"
            ),
        ),
    }
}

/// Where the IPC socket is, and whether an engine is answering on it.
#[must_use]
pub fn ipc_socket(socket: &SocketPath, file_exists: bool, listening: bool) -> DoctorCheck {
    const NAME: &str = "ipc.socket";
    match (listening, file_exists) {
        (true, _) => check(
            NAME,
            DoctorStatus::Ok,
            format!("an engine is listening on {socket}"),
        ),
        (false, true) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "{socket} exists but nothing accepts connections on it: a stale socket left by \
                 a crashed engine. The next engine start reclaims it automatically"
            ),
        ),
        (false, false) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "no engine running: {socket} does not exist. Start the engine, then retry; \
                 `vicinae toggle`, `show`, `hide` and `ping` all need it"
            ),
        ),
    }
}

/// Whether a session bus is configured and reachable.
pub async fn session_bus<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "dbus.session";
    let address = env.get("DBUS_SESSION_BUS_ADDRESS");
    match (bus.connect().await, address) {
        (Ok(()), Some(address)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("session bus reachable (DBUS_SESSION_BUS_ADDRESS={address})"),
        ),
        (Ok(()), None) => check(
            NAME,
            DoctorStatus::Warn,
            "DBUS_SESSION_BUS_ADDRESS is unset, but a session bus was still reachable via the \
             default $XDG_RUNTIME_DIR/bus path. Child processes that read the variable \
             directly — notably anything launched on the host from a sandbox — will not find it",
        ),
        (Err(err), Some(address)) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "DBUS_SESSION_BUS_ADDRESS={address} but connecting failed: {err}. Every \
                 portal, the global hotkey and all GNOME Shell integration are unavailable"
            ),
        ),
        (Err(err), None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "no session bus: DBUS_SESSION_BUS_ADDRESS is unset and connecting to the \
                 default path failed ({err}). Every portal, the global hotkey and all GNOME \
                 Shell integration are unavailable"
            ),
        ),
    }
}

/// Whether `org.freedesktop.portal.Desktop` is there.
///
/// The portal is D-Bus-activatable, so an unowned name is not proof of absence.
/// When nobody owns it we read its `version` property, which starts it if it is
/// installed; only then is "absent" an honest answer.
pub async fn desktop_portal<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "portal.desktop";

    match bus.name_has_owner(PORTAL_BUS_NAME).await {
        Ok(true) => {
            return check(
                NAME,
                DoctorStatus::Ok,
                format!("{PORTAL_BUS_NAME} is owned on the session bus"),
            );
        }
        Ok(false) => {}
        Err(err) => {
            return check(
                NAME,
                DoctorStatus::Fail,
                format!("could not ask the session bus who owns {PORTAL_BUS_NAME}: {err}"),
            );
        }
    }

    match bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            "org.freedesktop.portal.OpenURI",
            "version",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "{PORTAL_BUS_NAME} was not running but activated on demand (OpenURI v{version})"
            ),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "{PORTAL_BUS_NAME} is not owned and could not be activated: xdg-desktop-portal \
                 is not installed or not running. The global hotkey, file chooser and host \
                 URI opening are all unavailable"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Fail,
            format!("could not query {PORTAL_BUS_NAME}: {err}"),
        ),
    }
}

/// Whether the GlobalShortcuts portal interface is actually implemented.
///
/// This one carries real weight. On GNOME it is the *only* hotkey mechanism
/// (PLAN §3.4); `xdg-desktop-portal-gnome` has shipped a backend since 48.rc,
/// while `xdg-desktop-portal-wlr` ships none at all — so a running portal
/// proves nothing and the interface has to be probed directly.
pub async fn global_shortcuts<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "portal.global-shortcuts";
    match bus
        .property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("{GLOBAL_SHORTCUTS_INTERFACE} v{version} is available"),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Fail,
            format!(
                "{GLOBAL_SHORTCUTS_INTERFACE} is not implemented by the running portal backend, \
                 so the global hotkey cannot be bound. xdg-desktop-portal-gnome provides it from \
                 48.rc onwards; xdg-desktop-portal-wlr ships no GlobalShortcuts backend at all, \
                 so on wlroots compositors there is currently no hotkey path"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Fail,
            format!("could not query {GLOBAL_SHORTCUTS_INTERFACE}: {err}"),
        ),
    }
}

/// Whether the environment claims a GNOME session, with the desktop name(s).
#[must_use]
pub fn is_gnome(env: &Env) -> bool {
    env.list("XDG_CURRENT_DESKTOP")
        .iter()
        .any(|name| name.eq_ignore_ascii_case("GNOME"))
}

/// Which desktop this is, and GNOME Shell's version when it will tell us.
pub async fn desktop_environment<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "desktop.environment";

    let Some(current) = env.get("XDG_CURRENT_DESKTOP") else {
        let session = env.get("XDG_SESSION_DESKTOP").unwrap_or("also unset");
        return check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP is unset (XDG_SESSION_DESKTOP: {session}), so the desktop \
                 cannot be identified and desktop-specific integration is skipped"
            ),
        );
    };

    if !is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} — not a GNOME session. The Rust engine currently \
                 targets GNOME 50/51; wlroots (Hyprland/Sway/niri) and KDE support is Phase 5 \
                 work, so window switching, clipboard history and the global hotkey may all be \
                 unavailable here"
            ),
        );
    }

    match bus
        .property(
            GNOME_SHELL_BUS_NAME,
            GNOME_SHELL_OBJECT_PATH,
            GNOME_SHELL_BUS_NAME,
            "ShellVersion",
        )
        .await
    {
        Ok(Some(version)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("GNOME (XDG_CURRENT_DESKTOP={current}), GNOME Shell {version}"),
        ),
        Ok(None) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} claims GNOME, but {GNOME_SHELL_BUS_NAME} did not \
                 answer ShellVersion. Either GNOME Shell is not running or this is a \
                 GNOME-flavoured session without it; the Shell version gates extension \
                 compatibility, so it could not be assessed"
            ),
        ),
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "XDG_CURRENT_DESKTOP={current} claims GNOME, but the Shell version could not be \
                 read: {err}"
            ),
        ),
    }
}

/// Whether the Compass GNOME Shell extension is installed, and at what version.
///
/// **Provisional.** This probes the bus name and the versioned contract
/// directly, because `compass-shell` — which will own this knowledge — is being
/// written in parallel. The shape is the part that matters: three outcomes
/// (present / absent / version mismatch), each with the precise degradation
/// spelled out, so swapping the probe for `compass-shell`'s richer client is a
/// body change, not an interface change.
pub async fn shell_extension<B: BusProbe>(env: &Env, bus: &B) -> DoctorCheck {
    const NAME: &str = "gnome.shell-extension";

    if !is_gnome(env) {
        return check(
            NAME,
            DoctorStatus::Ok,
            "not a GNOME session, so the GNOME Shell extension is not used here",
        );
    }

    let version = bus
        .property(
            GNOME_SHELL_BUS_NAME,
            EXTENSION_OBJECT_PATH,
            EXTENSION_INTERFACE,
            "Version",
        )
        .await;

    match version {
        Ok(Some(raw)) => match raw.parse::<u32>() {
            Ok(found) if found == EXTENSION_CONTRACT_VERSION => check(
                NAME,
                DoctorStatus::Ok,
                format!("extension present, contract v{found}"),
            ),
            Ok(found) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "version mismatch: the installed extension speaks contract v{found}, this \
                     build speaks v{EXTENSION_CONTRACT_VERSION}. Until they match, \
                     {EXTENSION_DEGRADATION}"
                ),
            ),
            Err(_) => check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "the extension reported a Version of {raw:?}, which is not a contract \
                     number this build can compare against v{EXTENSION_CONTRACT_VERSION}. \
                     Treating it as incompatible: {EXTENSION_DEGRADATION}"
                ),
            ),
        },
        Ok(None) => absent_extension(bus).await,
        Err(err) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "could not query {EXTENSION_INTERFACE} on the session bus: {err}. Extension \
                 status is unknown; if it is in fact missing then {EXTENSION_DEGRADATION}"
            ),
        ),
    }
}

async fn absent_extension<B: BusProbe>(bus: &B) -> DoctorCheck {
    const NAME: &str = "gnome.shell-extension";

    // "Nothing answered" has two causes with different advice: the extension is
    // missing, or GNOME Shell itself is not on the bus (a GNOME-flavoured
    // session without Shell, or Shell still starting). Telling someone to
    // install an extension when Shell is not running is a wrong diagnosis.
    match bus.name_has_owner(GNOME_SHELL_BUS_NAME).await {
        Ok(false) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "nobody owns {GNOME_SHELL_BUS_NAME} on the session bus, so whether the \
                     extension is installed could not be determined. If GNOME Shell is not \
                     running, {EXTENSION_DEGRADATION}"
                ),
            );
        }
        Ok(true) => {}
        Err(err) => {
            return check(
                NAME,
                DoctorStatus::Warn,
                format!(
                    "could not ask the session bus about {GNOME_SHELL_BUS_NAME}: {err}. \
                     Extension status is unknown"
                ),
            );
        }
    }

    check(
        NAME,
        DoctorStatus::Warn,
        format!(
            "GNOME Shell is running but the Compass extension is not installed: nothing exports \
             {EXTENSION_INTERFACE} at {EXTENSION_OBJECT_PATH} on {GNOME_SHELL_BUS_NAME}. On \
             GNOME 50/51 the extension is the only mechanism available for these, so \
             {EXTENSION_DEGRADATION}"
        ),
    )
}

/// Whether we are running inside a Flatpak sandbox.
///
/// Informational either way — both answers are supported configurations — but
/// which one it is changes how apps get launched and what has to be in the
/// manifest, so a bug report needs it stated.
pub fn flatpak<F: FsProbe>(fs: &F) -> DoctorCheck {
    const NAME: &str = "flatpak.sandbox";
    if fs.exists(Path::new(FLATPAK_INFO_PATH)) {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "inside a Flatpak sandbox ({FLATPAK_INFO_PATH} present). Host applications are \
                 launched through `flatpak-spawn --host` with an OpenURI fallback, and \
                 .desktop/icon indexing needs --filesystem=host-os:ro plus read access to \
                 ~/.local/share/{{applications,icons}}"
            ),
        )
    } else {
        check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "not sandboxed ({FLATPAK_INFO_PATH} absent); applications are launched directly"
            ),
        )
    }
}

/// The XDG application directories this session searches, in order.
#[must_use]
pub fn application_dir_paths(env: &Env) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    let data_home = env
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }

    let data_dirs = env.list("XDG_DATA_DIRS");
    let data_dirs: Vec<&str> = if data_dirs.is_empty() {
        vec!["/usr/local/share", "/usr/share"]
    } else {
        data_dirs
    };
    for dir in data_dirs {
        dirs.push(Path::new(dir).join("applications"));
    }

    // Order-preserving dedupe: XDG_DATA_DIRS routinely repeats an entry, and
    // counting the same directory twice would inflate the .desktop total.
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|dir| seen.insert(dir.clone()));
    dirs
}

/// Which application directories exist, are readable, and how much is in them.
pub fn application_dirs<F: FsProbe>(env: &Env, fs: &F) -> DoctorCheck {
    const NAME: &str = "xdg.application-dirs";

    let dirs = application_dir_paths(env);
    if dirs.is_empty() {
        return check(
            NAME,
            DoctorStatus::Fail,
            "no application directories resolve: neither XDG_DATA_HOME nor HOME is set and \
             XDG_DATA_DIRS is empty. App search will return nothing",
        );
    }

    let mut lines = Vec::new();
    let mut total = 0usize;
    let mut unreadable = 0usize;

    for dir in &dirs {
        if !fs.exists(dir) {
            lines.push(format!("{} — absent", dir.display()));
            continue;
        }
        match fs.dir_entries(dir) {
            Ok(entries) => {
                let count = entries.iter().filter(|e| e.ends_with(".desktop")).count();
                total += count;
                lines.push(format!("{} — {count} .desktop files", dir.display()));
            }
            Err(err) => {
                unreadable += 1;
                lines.push(format!("{} — unreadable: {err}", dir.display()));
            }
        }
    }

    let status = if total == 0 {
        DoctorStatus::Fail
    } else if unreadable > 0 {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Ok
    };

    let headline = match status {
        DoctorStatus::Ok => format!("{total} .desktop files across {} directories", dirs.len()),
        DoctorStatus::Warn => format!(
            "{total} .desktop files, but {unreadable} of {} directories could not be read, so \
             some applications will be missing from search",
            dirs.len()
        ),
        DoctorStatus::Fail => format!(
            "no .desktop files found in any of the {} application directories; app search will \
             return nothing",
            dirs.len()
        ),
    };

    check(NAME, status, format!("{headline}\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::bus::FakeBus;
    use crate::doctor::fs::FakeFs;

    fn detail(check: &DoctorCheck) -> String {
        check.detail.clone().unwrap_or_default()
    }

    // ----- engine.selected ------------------------------------------------

    #[test]
    fn engine_rust_passes() {
        let c = engine(Engine::Rust);
        assert_eq!(c.name, "engine.selected");
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("rust"));
    }

    #[test]
    fn engine_cpp_warns_and_says_why() {
        let c = engine(Engine::Cpp);
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("cannot dispatch"));
        assert!(d.contains("--engine rust"));
    }

    // ----- session.type ---------------------------------------------------

    #[test]
    fn session_type_wayland_passes() {
        let c = session_type(&Env::from_pairs([("WAYLAND_DISPLAY", "wayland-0")]));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("wayland-0"));
        assert!(!detail(&c).contains("XWayland"));
    }

    #[test]
    fn session_type_reports_xwayland_when_both_are_set() {
        let c = session_type(&Env::from_pairs([
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
        ]));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("XWayland"));
    }

    #[test]
    fn session_type_x11_only_warns_about_the_hotkey() {
        let c = session_type(&Env::from_pairs([("DISPLAY", ":0")]));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("X11"));
        assert!(detail(&c).contains("hotkey"));
    }

    #[test]
    fn session_type_headless_fails() {
        let c = session_type(&Env::empty());
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("no graphical session"));
    }

    #[test]
    fn session_type_treats_an_empty_wayland_display_as_absent() {
        let c = session_type(&Env::from_pairs([("WAYLAND_DISPLAY", ""), ("DISPLAY", "")]));
        assert_eq!(c.status, DoctorStatus::Fail);
    }

    // ----- xdg.runtime-dir ------------------------------------------------

    #[test]
    fn runtime_dir_present_passes() {
        let env = Env::from_pairs([("XDG_RUNTIME_DIR", "/run/user/1000")]);
        let c = runtime_dir(&env, &SocketPath::in_dir("/run/user/1000"));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("/run/user/1000"));
    }

    #[test]
    fn runtime_dir_absent_surfaces_the_fallback_as_a_degradation() {
        let socket = SocketPath::from_env();
        // Construct the fallback shape explicitly rather than depending on the
        // test runner's own environment.
        let fallback = if socket.is_fallback() {
            socket
        } else {
            // `SocketPath` only produces a fallback from `from_env`, so when the
            // runner has XDG_RUNTIME_DIR set we exercise the other absent arm.
            let c = runtime_dir(&Env::empty(), &SocketPath::exact("/tmp/explicit.sock"));
            assert_eq!(c.status, DoctorStatus::Warn);
            assert!(detail(&c).contains("set explicitly"));
            return;
        };

        let c = runtime_dir(&Env::empty(), &fallback);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("XDG_RUNTIME_DIR is unset"));
        assert!(detail(&c).contains("tmpfs"));
    }

    #[test]
    fn runtime_dir_absent_with_an_explicit_socket_still_warns() {
        let c = runtime_dir(&Env::empty(), &SocketPath::exact("/tmp/explicit.sock"));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("/tmp/explicit.sock"));
        assert!(!detail(&c).contains("world-readable"));
    }

    #[test]
    fn runtime_dir_empty_value_reads_as_unset() {
        let c = runtime_dir(
            &Env::from_pairs([("XDG_RUNTIME_DIR", "")]),
            &SocketPath::exact("/tmp/x.sock"),
        );
        assert_eq!(c.status, DoctorStatus::Warn);
    }

    // ----- ipc.socket -----------------------------------------------------

    #[test]
    fn ipc_socket_listening_passes() {
        let c = ipc_socket(&SocketPath::exact("/tmp/a.sock"), true, true);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("/tmp/a.sock"));
    }

    #[test]
    fn ipc_socket_stale_file_is_named_as_stale() {
        let c = ipc_socket(&SocketPath::exact("/tmp/a.sock"), true, false);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("stale"));
    }

    #[test]
    fn ipc_socket_absent_says_no_engine_is_running() {
        let c = ipc_socket(&SocketPath::exact("/tmp/a.sock"), false, false);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("no engine running"));
        assert!(detail(&c).contains("does not exist"));
    }

    // ----- dbus.session ---------------------------------------------------

    #[tokio::test]
    async fn session_bus_reachable_with_address_passes() {
        let env = Env::from_pairs([("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")]);
        let c = session_bus(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("unix:path=/run/user/1000/bus"));
    }

    #[tokio::test]
    async fn session_bus_reachable_without_the_variable_warns() {
        let c = session_bus(&Env::empty(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("DBUS_SESSION_BUS_ADDRESS is unset"));
    }

    #[tokio::test]
    async fn session_bus_unreachable_with_an_address_fails() {
        let env = Env::from_pairs([("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nope")]);
        let c = session_bus(&env, &FakeBus::unreachable("No such file or directory")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("unix:path=/nope"));
        assert!(detail(&c).contains("No such file or directory"));
    }

    #[tokio::test]
    async fn session_bus_absent_entirely_fails() {
        let c = session_bus(&Env::empty(), &FakeBus::unreachable("connection refused")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("no session bus"));
    }

    // ----- portal.desktop -------------------------------------------------

    #[tokio::test]
    async fn portal_owned_passes() {
        let c = desktop_portal(&FakeBus::new().with_name(PORTAL_BUS_NAME)).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("is owned"));
    }

    #[tokio::test]
    async fn portal_activatable_but_not_running_passes_and_says_so() {
        let bus = FakeBus::new().with_property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            "org.freedesktop.portal.OpenURI",
            "version",
            "4",
        );
        let c = desktop_portal(&bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("activated on demand"));
    }

    #[tokio::test]
    async fn portal_absent_fails_and_names_what_breaks() {
        let c = desktop_portal(&FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        let d = detail(&c);
        assert!(d.contains("not installed or not running"));
        assert!(d.contains("global hotkey"));
    }

    #[tokio::test]
    async fn portal_unqueryable_fails_as_could_not_ask_not_as_absent() {
        let c = desktop_portal(&FakeBus::failing_queries("Connection reset by peer")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("could not ask"));
        assert!(!detail(&c).contains("not installed"));
    }

    // ----- portal.global-shortcuts ----------------------------------------

    #[tokio::test]
    async fn global_shortcuts_present_passes_with_its_version() {
        let bus = FakeBus::new().with_property(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            GLOBAL_SHORTCUTS_INTERFACE,
            "version",
            "2",
        );
        let c = global_shortcuts(&bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("v2"));
    }

    #[tokio::test]
    async fn global_shortcuts_absent_fails_and_explains_the_backend_situation() {
        // A running portal proves nothing: this is the wlr case, where the
        // portal exists but ships no GlobalShortcuts backend.
        let bus = FakeBus::new().with_name(PORTAL_BUS_NAME);
        let c = global_shortcuts(&bus).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        let d = detail(&c);
        assert!(d.contains("cannot be bound"));
        assert!(d.contains("xdg-desktop-portal-wlr"));
        assert!(d.contains("48.rc"));
    }

    #[tokio::test]
    async fn global_shortcuts_unqueryable_fails_without_claiming_absence() {
        let c = global_shortcuts(&FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("could not query"));
        assert!(!detail(&c).contains("not implemented"));
    }

    // ----- desktop.environment --------------------------------------------

    fn gnome_env() -> Env {
        Env::from_pairs([("XDG_CURRENT_DESKTOP", "GNOME")])
    }

    fn shell_bus(version: &str) -> FakeBus {
        FakeBus::new()
            .with_name(GNOME_SHELL_BUS_NAME)
            .with_property(
                GNOME_SHELL_BUS_NAME,
                GNOME_SHELL_OBJECT_PATH,
                GNOME_SHELL_BUS_NAME,
                "ShellVersion",
                version,
            )
    }

    #[tokio::test]
    async fn desktop_gnome_with_a_shell_version_passes() {
        let c = desktop_environment(&gnome_env(), &shell_bus("51.0")).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("GNOME Shell 51.0"));
    }

    #[tokio::test]
    async fn desktop_recognises_gnome_inside_a_colon_list_case_insensitively() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "ubuntu:gnome")]);
        assert!(is_gnome(&env));
        let c = desktop_environment(&env, &shell_bus("50.1")).await;
        assert_eq!(c.status, DoctorStatus::Ok);
    }

    #[tokio::test]
    async fn desktop_gnome_without_a_reachable_shell_warns() {
        let c = desktop_environment(&gnome_env(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("did not answer ShellVersion"));
    }

    #[tokio::test]
    async fn desktop_non_gnome_warns_about_the_target() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "sway:wlroots")]);
        assert!(!is_gnome(&env));
        let c = desktop_environment(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("not a GNOME session"));
        assert!(detail(&c).contains("Phase 5"));
    }

    #[tokio::test]
    async fn desktop_unset_warns_and_mentions_the_session_variable() {
        let env = Env::from_pairs([("XDG_SESSION_DESKTOP", "gnome")]);
        let c = desktop_environment(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("XDG_CURRENT_DESKTOP is unset"));
        assert!(detail(&c).contains("gnome"));
    }

    #[tokio::test]
    async fn desktop_gnome_with_an_unqueryable_bus_warns() {
        let c = desktop_environment(&gnome_env(), &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("could not be read"));
    }

    // ----- gnome.shell-extension ------------------------------------------

    fn extension_bus(version: &str) -> FakeBus {
        FakeBus::new()
            .with_name(GNOME_SHELL_BUS_NAME)
            .with_property(
                GNOME_SHELL_BUS_NAME,
                EXTENSION_OBJECT_PATH,
                EXTENSION_INTERFACE,
                "Version",
                version,
            )
    }

    #[tokio::test]
    async fn extension_at_the_expected_version_passes() {
        let bus = extension_bus(&EXTENSION_CONTRACT_VERSION.to_string());
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("extension present"));
    }

    #[tokio::test]
    async fn extension_version_mismatch_warns_and_names_the_degradation() {
        let bus = extension_bus(&(EXTENSION_CONTRACT_VERSION + 7).to_string());
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("version mismatch"));
        assert!(d.contains("window switching"));
        assert!(d.contains("clipboard history"));
        assert!(d.contains("paste"));
    }

    #[tokio::test]
    async fn extension_with_a_malformed_version_is_treated_as_incompatible() {
        let bus = extension_bus("v2-beta");
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("not a contract number"));
        assert!(detail(&c).contains("window switching"));
    }

    #[tokio::test]
    async fn extension_absent_on_a_running_shell_names_exactly_what_breaks() {
        let bus = FakeBus::new().with_name(GNOME_SHELL_BUS_NAME);
        let c = shell_extension(&gnome_env(), &bus).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        let d = detail(&c);
        assert!(d.contains("is not installed"));
        assert!(d.contains("window switching, clipboard history and paste are unavailable"));
        // The §3.5.1 promise: everything else is explicitly said to still work.
        assert!(d.contains("app search, launching"));
        assert!(!d.contains("some features"));
    }

    #[tokio::test]
    async fn extension_check_does_not_blame_the_extension_when_shell_is_absent() {
        let c = shell_extension(&gnome_env(), &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("nobody owns org.gnome.Shell"));
        assert!(!detail(&c).contains("is not installed"));
    }

    #[tokio::test]
    async fn extension_check_is_not_applicable_off_gnome() {
        let env = Env::from_pairs([("XDG_CURRENT_DESKTOP", "KDE")]);
        let c = shell_extension(&env, &FakeBus::new()).await;
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("not a GNOME session"));
    }

    #[tokio::test]
    async fn extension_check_reports_unknown_when_the_bus_cannot_be_queried() {
        let c = shell_extension(&gnome_env(), &FakeBus::failing_queries("Timeout")).await;
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("status is unknown"));
    }

    // ----- flatpak.sandbox ------------------------------------------------

    #[test]
    fn flatpak_inside_the_sandbox_mentions_flatpak_spawn() {
        let c = flatpak(&FakeFs::new().with_file(FLATPAK_INFO_PATH));
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("flatpak-spawn --host"));
        assert!(detail(&c).contains("host-os:ro"));
    }

    #[test]
    fn flatpak_outside_the_sandbox_says_so() {
        let c = flatpak(&FakeFs::new());
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("not sandboxed"));
    }

    // ----- xdg.application-dirs -------------------------------------------

    #[test]
    fn application_dirs_default_to_the_spec_paths() {
        let env = Env::from_pairs([("HOME", "/home/tester")]);
        let dirs = application_dir_paths(&env);
        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/tester/.local/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_prefer_xdg_data_home_over_home() {
        let env = Env::from_pairs([
            ("HOME", "/home/tester"),
            ("XDG_DATA_HOME", "/custom/data"),
            ("XDG_DATA_DIRS", "/usr/share"),
        ]);
        assert_eq!(
            application_dir_paths(&env),
            [
                PathBuf::from("/custom/data/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_are_deduplicated() {
        let env = Env::from_pairs([
            ("XDG_DATA_HOME", "/usr/share"),
            ("XDG_DATA_DIRS", "/usr/share:/opt/share:/usr/share"),
        ]);
        assert_eq!(
            application_dir_paths(&env),
            [
                PathBuf::from("/usr/share/applications"),
                PathBuf::from("/opt/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_with_no_home_and_no_data_dirs_still_have_the_system_defaults() {
        assert_eq!(
            application_dir_paths(&Env::empty()),
            [
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
    }

    #[test]
    fn application_dirs_counts_only_desktop_files() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new()
            .with_dir(
                "/d/applications",
                ["a.desktop", "notes.txt", "mimeinfo.cache"],
            )
            .with_dir("/usr/share/applications", ["b.desktop", "c.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("3 .desktop files"));
        assert!(detail(&c).contains("/d/applications — 1 .desktop files"));
    }

    #[test]
    fn application_dirs_absent_directories_are_not_a_problem_on_their_own() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new().with_dir("/usr/share/applications", ["b.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Ok);
        assert!(detail(&c).contains("/d/applications — absent"));
    }

    #[test]
    fn application_dirs_unreadable_directory_warns() {
        let env = Env::from_pairs([("XDG_DATA_HOME", "/d"), ("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new()
            .with_unreadable_dir("/d/applications", "Permission denied (os error 13)")
            .with_dir("/usr/share/applications", ["b.desktop"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("Permission denied"));
        assert!(detail(&c).contains("some applications will be missing"));
    }

    #[test]
    fn application_dirs_with_nothing_to_index_fails() {
        let env = Env::from_pairs([("XDG_DATA_DIRS", "/usr/share")]);
        let fs = FakeFs::new().with_dir("/usr/share/applications", ["mimeinfo.cache"]);
        let c = application_dirs(&env, &fs);
        assert_eq!(c.status, DoctorStatus::Fail);
        assert!(detail(&c).contains("app search will return nothing"));
    }

    #[test]
    fn application_dirs_completely_missing_fails() {
        let c = application_dirs(&Env::empty(), &FakeFs::new());
        assert_eq!(c.status, DoctorStatus::Fail);
    }
}
