//! `vicinae doctor`: what works here, what does not, and what that costs.
//!
//! The design constraint is `PLAN.md` §8.6: *a doctor that always says "fine"
//! is worse than no doctor*. Two things follow from taking that seriously.
//!
//! **Absence has to be as testable as presence.** Every input the checks read —
//! environment variables, the filesystem, the session bus — arrives through an
//! injectable seam ([`Env`], [`FsProbe`], [`BusProbe`]), so "GNOME with no
//! Shell extension, a portal without GlobalShortcuts and no
//! `XDG_RUNTIME_DIR`" is a value a unit test constructs, not a machine someone
//! has to go and find.
//!
//! **A missing capability has to be reported as the specific thing it costs.**
//! "Some features may not work" is what a doctor says when it does not know.
//! Checks here name the feature: no Shell extension means window switching,
//! clipboard history and paste, and nothing else.
//!
//! # Status meanings
//!
//! * [`DoctorStatus::Ok`] — works.
//! * [`DoctorStatus::Warn`] — degraded in a way this build is designed to
//!   survive (PLAN §3.5.1: nothing critical-path needs the Shell extension), or
//!   a fact worth stating in a bug report.
//! * [`DoctorStatus::Fail`] — something the launcher needs is broken.
//!
//! Only [`DoctorStatus::Fail`] makes `--check-only` exit non-zero.

pub mod bus;
pub mod checks;
pub mod env;
pub mod fs;
pub mod report;

pub use bus::{BusError, BusProbe, BusResult, FakeBus, ZbusProbe};
pub use env::Env;
pub use fs::{FakeFs, FsProbe, RealFs};
pub use report::{JSON_SCHEMA_VERSION, Report, Summary, Verdict};

use compass_ipc::SocketPath;

use crate::engine::Engine;

/// Everything the checks need, gathered once.
///
/// Facts that require I/O the checks themselves should not do — whether a
/// daemon accepts connections on the socket — are resolved by the caller and
/// passed in, which keeps [`checks`] synchronous where it can be and free of
/// ambient state where it cannot.
#[derive(Debug)]
pub struct Inputs<'a, B: BusProbe, F: FsProbe> {
    /// Environment snapshot.
    pub env: &'a Env,
    /// Filesystem probe.
    pub fs: &'a F,
    /// Session-bus probe.
    pub bus: &'a B,
    /// Socket path in effect, after any `--socket` override.
    pub socket: &'a SocketPath,
    /// Whether the socket path exists on disk.
    pub socket_exists: bool,
    /// Whether something accepted a connection on it.
    pub daemon_listening: bool,
    /// Engine selected for this invocation.
    pub engine: Engine,
}

/// Runs every check, in report order.
pub async fn run<B: BusProbe, F: FsProbe>(inputs: &Inputs<'_, B, F>) -> Report {
    let Inputs {
        env,
        fs,
        bus,
        socket,
        socket_exists,
        daemon_listening,
        engine,
    } = *inputs;

    let checks = vec![
        checks::engine(engine),
        checks::session_type(env),
        checks::desktop_environment(env, bus).await,
        checks::runtime_dir(env, socket),
        checks::ipc_socket(socket, socket_exists, daemon_listening),
        checks::session_bus(env, bus).await,
        checks::desktop_portal(bus).await,
        checks::global_shortcuts(bus).await,
        checks::shell_extension(env, bus).await,
        checks::flatpak(fs),
        checks::application_dirs(env, fs),
    ];

    Report {
        engine,
        socket: socket.to_string(),
        checks,
    }
}

/// Gathers the real inputs and runs the checks.
///
/// The only function in this module that touches process-global state; it is
/// deliberately trivial so that everything worth testing lives below it.
pub async fn run_on_this_machine(socket: &SocketPath, engine: Engine) -> Report {
    let env = Env::from_process();
    let fs = RealFs;
    let bus = ZbusProbe::new();

    let inputs = Inputs {
        env: &env,
        fs: &fs,
        bus: &bus,
        socket,
        socket_exists: socket.as_path().exists(),
        daemon_listening: compass_ipc::is_listening(socket.as_path()).await,
        engine,
    };

    run(&inputs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_ipc::DoctorStatus;

    /// A machine where absolutely nothing is available.
    fn barren() -> (Env, FakeFs, FakeBus, SocketPath) {
        (
            Env::empty(),
            FakeFs::new(),
            FakeBus::unreachable("No such file or directory (os error 2)"),
            SocketPath::exact("/tmp/nowhere/ipc.sock"),
        )
    }

    /// A healthy GNOME 51 Wayland session with everything installed.
    fn healthy() -> (Env, FakeFs, FakeBus, SocketPath) {
        let env = Env::from_pairs([
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_CURRENT_DESKTOP", "GNOME"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_DATA_DIRS", "/usr/share"),
            ("HOME", "/home/tester"),
        ]);
        let fs = FakeFs::new()
            .with_dir("/home/tester/.local/share/applications", ["mine.desktop"])
            .with_dir("/usr/share/applications", ["a.desktop", "b.desktop"]);
        let bus = FakeBus::new()
            .with_name(checks::PORTAL_BUS_NAME)
            .with_name(checks::GNOME_SHELL_BUS_NAME)
            .with_property(
                checks::PORTAL_BUS_NAME,
                checks::PORTAL_OBJECT_PATH,
                checks::GLOBAL_SHORTCUTS_INTERFACE,
                "version",
                "2",
            )
            .with_property(
                checks::GNOME_SHELL_BUS_NAME,
                checks::GNOME_SHELL_OBJECT_PATH,
                checks::GNOME_SHELL_BUS_NAME,
                "ShellVersion",
                "51.0",
            )
            .with_property(
                checks::GNOME_SHELL_BUS_NAME,
                checks::EXTENSION_OBJECT_PATH,
                checks::EXTENSION_INTERFACE,
                "Version",
                checks::EXTENSION_CONTRACT_VERSION.to_string(),
            );
        (env, fs, bus, SocketPath::in_dir("/run/user/1000"))
    }

    fn inputs<'a>(
        env: &'a Env,
        fs: &'a FakeFs,
        bus: &'a FakeBus,
        socket: &'a SocketPath,
        listening: bool,
    ) -> Inputs<'a, FakeBus, FakeFs> {
        Inputs {
            env,
            fs,
            bus,
            socket,
            socket_exists: listening,
            daemon_listening: listening,
            engine: Engine::Rust,
        }
    }

    #[tokio::test]
    async fn a_healthy_machine_reports_no_failures() {
        let (env, fs, bus, socket) = healthy();
        let report = run(&inputs(&env, &fs, &bus, &socket, true)).await;
        let problems: Vec<_> = report
            .checks
            .iter()
            .filter(|c| c.status != DoctorStatus::Ok)
            .collect();
        assert!(problems.is_empty(), "unexpected problems: {problems:#?}");
        assert_eq!(report.verdict(), Verdict::Ok);
    }

    #[tokio::test]
    async fn a_barren_machine_reports_failures_not_a_clean_bill() {
        let (env, fs, bus, socket) = barren();
        let report = run(&inputs(&env, &fs, &bus, &socket, false)).await;
        assert_eq!(report.verdict(), Verdict::Fail);
        assert!(report.has_failures());
        let summary = report.summary();
        assert!(summary.fail >= 4, "expected several failures: {summary:?}");
        assert_eq!(summary.total, report.checks.len());
    }

    #[tokio::test]
    async fn every_check_runs_exactly_once_and_has_a_detail() {
        let (env, fs, bus, socket) = healthy();
        let report = run(&inputs(&env, &fs, &bus, &socket, true)).await;
        let mut names: Vec<&str> = report.checks.iter().map(|c| c.name.as_str()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate check names");
        assert_eq!(count, 11);
        assert!(
            report
                .checks
                .iter()
                .all(|c| c.detail.as_deref().is_some_and(|d| !d.trim().is_empty()))
        );
    }

    #[tokio::test]
    async fn the_report_order_is_stable() {
        let (env, fs, bus, socket) = healthy();
        let a = run(&inputs(&env, &fs, &bus, &socket, true)).await;
        let b = run(&inputs(&env, &fs, &bus, &socket, true)).await;
        assert_eq!(a, b);
    }
}
