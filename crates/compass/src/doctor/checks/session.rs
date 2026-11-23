//! Session environment, runtime directory and IPC socket checks.
//!
//! Part of [`super`]; see that module for the purity rule every function here
//! keeps to.

use compass_ipc::{DoctorCheck, DoctorStatus, SocketPath};

use super::check;
use crate::doctor::env::Env;

/// Wayland, X11, or no graphical session at all.
///
/// The Wayland arm uses [`has_wayland_in`](crate::doctor::has_wayland_in)'s two
/// variables, so this check and the cold-start path's
/// [`has_display_in`](crate::doctor::has_display_in) answer the same question
/// for the same environment.
#[must_use]
pub fn session_type(env: &Env) -> DoctorCheck {
    const NAME: &str = "session.type";
    let wayland = env
        .get("WAYLAND_DISPLAY")
        .map(|display| ("WAYLAND_DISPLAY", display))
        .or_else(|| {
            env.get("WAYLAND_SOCKET")
                .map(|sock| ("WAYLAND_SOCKET", sock))
        });
    match (wayland, env.get("DISPLAY")) {
        (Some((key, wayland)), Some(x11)) => check(
            NAME,
            DoctorStatus::Ok,
            format!("Wayland ({key}={wayland}), with XWayland available at DISPLAY={x11}"),
        ),
        (Some((key, wayland)), None) => {
            check(NAME, DoctorStatus::Ok, format!("Wayland ({key}={wayland})"))
        }
        (None, Some(x11)) => check(
            NAME,
            DoctorStatus::Warn,
            format!(
                "X11 only (DISPLAY={x11}, WAYLAND_DISPLAY unset). Compass is made for \
                 Wayland and does not bind its hotkey on X11: bind a key to `compass toggle` \
                 in your window manager instead"
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
                 `compass toggle`, `show`, `hide` and `ping` all need it"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::checks::tests::detail;

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

    #[test]
    fn session_type_and_has_display_in_agree() {
        use crate::doctor::has_display_in;

        let environments = [
            Env::empty(),
            Env::from_pairs([("WAYLAND_DISPLAY", ""), ("DISPLAY", "")]),
            Env::from_pairs([("WAYLAND_DISPLAY", "wayland-0")]),
            Env::from_pairs([("WAYLAND_SOCKET", "/run/user/1000/wayland-0")]),
            Env::from_pairs([("DISPLAY", ":0")]),
            Env::from_pairs([("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]),
        ];
        for env in environments {
            assert_eq!(
                session_type(&env).status != DoctorStatus::Fail,
                has_display_in(&env),
                "the doctor and the cold-start path must agree on {env:?}"
            );
        }
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
}
