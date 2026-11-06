//! `compass doctor`: what works here, what does not, and what that costs.
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
//! * [`DoctorStatus::Ok`](compass_ipc::DoctorStatus::Ok) — works.
//! * [`DoctorStatus::Warn`](compass_ipc::DoctorStatus::Warn) — degraded in a way this build is designed to
//!   survive (PLAN §3.5.1: nothing critical-path needs the Shell extension), or
//!   a fact worth stating in a bug report.
//! * [`DoctorStatus::Fail`](compass_ipc::DoctorStatus::Fail) — something the launcher needs is broken.
//!
//! Only [`DoctorStatus::Fail`](compass_ipc::DoctorStatus::Fail) makes `--check-only` exit non-zero.

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
    /// The snippet keyword expander's helper, as far as it could be read.
    pub input_server: &'a checks::InputServerFacts,
    /// What probing the Wayland compositor found; `None` with no display or
    /// when the probe failed.
    pub wayland: Option<checks::WaylandFindings>,
    /// What reading `compass.json` found.
    pub config: &'a checks::ConfigFacts,
    /// What installing the extensions `compass.json` declares would do, or
    /// why the file could not be read.
    pub declared_extensions: &'a Result<Vec<compass_core::declared_extensions::Step>, String>,
    /// The file-search index's measurements; `None` without a cache
    /// directory to find it in.
    pub file_index: Option<&'a compass_db::index_stats::IndexStats>,
    /// Clipboard history on disk, its key, and what the engine says of it.
    pub clipboard: &'a checks::ClipboardFacts,
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
        input_server,
        ref wayland,
        config,
        declared_extensions,
        file_index,
        clipboard,
    } = *inputs;

    let checks = vec![
        checks::engine(engine),
        checks::config_file(config),
        checks::session_type(env),
        checks::desktop_environment(env, bus).await,
        checks::runtime_dir(env, socket),
        checks::ipc_socket(socket, socket_exists, daemon_listening),
        checks::session_bus(env, bus).await,
        checks::desktop_portal(bus).await,
        checks::global_shortcuts(
            env,
            fs.exists(std::path::Path::new(checks::FLATPAK_INFO_PATH)),
            bus,
        )
        .await,
        checks::shell_extension(env, bus).await,
        checks::kwin(env, bus).await,
        checks::wlroots(env, wayland.as_ref(), bus).await,
        checks::clipboard_history(env, wayland.as_ref(), clipboard.engine.as_ref()),
        checks::keyring(env, bus).await,
        checks::clipboard_storage(clipboard),
        checks::flatpak(fs),
        checks::input_server(fs, input_server),
        checks::declared_extensions(declared_extensions),
        checks::file_index(file_index),
        checks::application_dirs(env, fs),
        checks::screen_reader(bus).await,
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

    let daemon_listening = compass_ipc::is_listening(socket.as_path()).await;
    let status = if daemon_listening {
        match crate::ipc::send(socket, compass_ipc::Request::InputServerStatus).await {
            Ok(compass_ipc::Response::InputServerStatus(status)) => Some(status),
            _ => None,
        }
    } else {
        None
    };
    let input_server = tokio::task::spawn_blocking(move || gather_input_server(status))
        .await
        .unwrap_or_default();

    let config = tokio::task::spawn_blocking(gather_config)
        .await
        .unwrap_or_default();
    let declared_extensions = tokio::task::spawn_blocking(gather_declared_extensions)
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    let file_index = tokio::task::spawn_blocking(gather_file_index)
        .await
        .ok()
        .flatten();
    let clipboard = gather_clipboard(socket, daemon_listening).await;
    let inputs = Inputs {
        env: &env,
        fs: &fs,
        bus: &bus,
        socket,
        socket_exists: socket.as_path().exists(),
        daemon_listening,
        engine,
        input_server: &input_server,
        config: &config,
        declared_extensions: &declared_extensions,
        file_index: file_index.as_ref(),
        clipboard: &clipboard,
        wayland: tokio::task::spawn_blocking(probe_wayland)
            .await
            .ok()
            .flatten(),
    };

    run(&inputs).await
}

/// What reconciling `extensions.installed` would do now, worked out without
/// the network.
///
/// # Errors
///
/// When `compass.json` cannot be read, with why.
pub fn gather_declared_extensions() -> Result<Vec<compass_core::declared_extensions::Step>, String>
{
    let config = compass_core::Config::load().map_err(|error| error.to_string())?;
    let installed: std::collections::HashSet<String> =
        compass_core::manifest::registry::scan(&compass_core::manifest::registry::search_paths())
            .extensions
            .into_iter()
            .map(|manifest| manifest.id)
            .collect();
    Ok(compass_core::declared_extensions::plan(
        config.extensions().installed(),
        |id| installed.contains(id),
        compass_core::raycast_overrides::Manifest::shipped(),
    ))
}

/// The file-search index's measurements, read-only; `None` without a cache
/// directory.
#[must_use]
pub fn gather_file_index() -> Option<compass_db::index_stats::IndexStats> {
    let cache = compass_core::xdg_dirs::cache_home()?;
    Some(compass_db::index_stats::inspect(
        &crate::indexer_service::database_path(&cache),
    ))
}

/// Reads the clipboard history and its key without changing either, and asks
/// a running engine what it is doing with them.
pub async fn gather_clipboard(
    socket: &SocketPath,
    daemon_listening: bool,
) -> checks::ClipboardFacts {
    let engine = if daemon_listening {
        match crate::ipc::send(socket, compass_ipc::Request::ClipboardStatus).await {
            Ok(compass_ipc::Response::ClipboardStatus(status)) => Some(status),
            _ => None,
        }
    } else {
        None
    };
    checks::ClipboardFacts {
        inspection: crate::clipboard_service::inspect()
            .await
            .map_err(|err| err.to_string()),
        engine,
    }
}

/// Reads `compass.json` and checks it, as the engine does when it starts.
#[must_use]
pub fn gather_config() -> checks::ConfigFacts {
    let path = compass_core::config::default_config_path().ok();
    let exists = path.as_ref().is_some_and(|path| path.exists());
    let _ = compass_ui::theme::load_default_user_themes();
    let problems = match compass_core::Config::load_with_issues() {
        Ok((config, issues)) => Ok(crate::config_watch::check(&config, issues)
            .iter()
            .map(ToString::to_string)
            .collect()),
        Err(error) => Err(error.to_string()),
    };
    checks::ConfigFacts {
        path,
        exists,
        problems,
    }
}

/// Reads the facts on this machine. The engine's status is the caller's.
#[must_use]
pub fn gather_input_server(
    engine: Option<compass_ipc::InputServerStatus>,
) -> checks::InputServerFacts {
    let enabled = compass_core::Config::load()
        .map(|config| config.input_server().enabled())
        .unwrap_or(compass_core::config::DEFAULT_INPUT_SERVER_ENABLED);
    let helper = crate::input_server::find_helper();
    let capability = helper.as_deref().and_then(has_dac_override);
    let uinput_writable = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let input_readable = std::fs::read_dir("/dev/input").is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.file_name().to_string_lossy().starts_with("event")
                && std::fs::File::open(entry.path()).is_ok()
        })
    });
    checks::InputServerFacts {
        enabled,
        helper,
        capability,
        uinput_writable,
        input_readable,
        engine,
    }
}

/// Whether `getcap` lists `cap_dac_override` on `path`; `None` without
/// `getcap`.
fn has_dac_override(path: &std::path::Path) -> Option<bool> {
    let output = std::process::Command::new("getcap")
        .arg(path)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).contains("cap_dac_override"))
}

/// Probes the compositor in `WAYLAND_DISPLAY`: its family, the wlroots
/// protocols it advertises, and whether its own IPC (Hyprland, niri) answers.
/// Blocking.
fn probe_wayland() -> Option<checks::WaylandFindings> {
    let session = compass_wayland::compositor::Session::detect().ok()?;
    Some(checks::WaylandFindings {
        family: session.family,
        capabilities: compass_wayland::compositor::Capabilities::of(&session.globals),
        compositor_ipc: compass_platform_linux::compositor::Provider::detect()
            .map(|provider| (provider.display_name().to_owned(), provider.ping())),
    })
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
                checks::SECRET_SERVICE_BUS_NAME,
                checks::DEFAULT_COLLECTION_PATH,
                checks::COLLECTION_INTERFACE,
                "Locked",
                "false",
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

    static CLEAN_CONFIG: std::sync::LazyLock<checks::ConfigFacts> =
        std::sync::LazyLock::new(|| checks::ConfigFacts {
            path: Some("/home/tester/.config/compass/compass.json".into()),
            exists: true,
            problems: Ok(Vec::new()),
        });

    static HEALTHY_INPUT_SERVER: std::sync::LazyLock<checks::InputServerFacts> =
        std::sync::LazyLock::new(|| checks::InputServerFacts {
            enabled: true,
            helper: Some("/usr/libexec/compass/compass-input-server".into()),
            capability: Some(true),
            ..checks::InputServerFacts::default()
        });

    static NOTHING_DECLARED: Result<Vec<compass_core::declared_extensions::Step>, String> =
        Ok(Vec::new());

    /// An index not built yet, as `inspect` finds it.
    static NO_INDEX: std::sync::LazyLock<compass_db::index_stats::IndexStats> =
        std::sync::LazyLock::new(|| {
            use compass_db::index_stats::Measurement;
            let unavailable = Measurement::Unavailable("file index has not been created".into());
            compass_db::index_stats::IndexStats {
                database_bytes: Measurement::Missing,
                wal_bytes: Measurement::Missing,
                shm_bytes: Measurement::Missing,
                page_size: unavailable.clone(),
                page_count: unavailable.clone(),
                freelist_count: unavailable.clone(),
                indexed_rows: unavailable.clone(),
                fts_bytes: unavailable,
            }
        });

    /// No clipboard history yet, and no engine asked about it.
    static HEALTHY_CLIPBOARD: std::sync::LazyLock<checks::ClipboardFacts> =
        std::sync::LazyLock::new(|| {
            let dir = std::path::PathBuf::from("/home/tester/.local/share/compass");
            checks::ClipboardFacts {
                inspection: Ok(crate::clipboard_service::Inspection {
                    survey: Ok(compass_clipboard::files::Survey {
                        dir: dir.clone(),
                        parts: Vec::new(),
                    }),
                    dir,
                    key: crate::clipboard_service::KeyState::Missing,
                    read: None,
                }),
                engine: None,
            }
        });

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
            input_server: &HEALTHY_INPUT_SERVER,
            config: &CLEAN_CONFIG,
            declared_extensions: &NOTHING_DECLARED,
            file_index: Some(&NO_INDEX),
            clipboard: &HEALTHY_CLIPBOARD,
            // The healthy machine is GNOME; the barren one has no display.
            wayland: env.get("WAYLAND_DISPLAY").map(|_| checks::WaylandFindings {
                family: compass_wayland::compositor::Family::Gnome,
                capabilities: compass_wayland::compositor::Capabilities::default(),
                compositor_ipc: None,
            }),
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
    async fn a_correctly_set_up_sway_has_no_failures_so_check_only_exits_zero() {
        // Sway as it ships: a portal (xdg-desktop-portal-wlr) with no
        // GlobalShortcuts, the key bound in Sway's config, no Hyprland or niri
        // socket, every wlroots protocol, and a keyring.
        let env = Env::from_pairs([
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_CURRENT_DESKTOP", "sway"),
            ("SWAYSOCK", "/run/user/1000/sway-ipc.1000.42.sock"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("HOME", "/home/tester"),
        ]);
        let fs = FakeFs::new().with_dir("/usr/share/applications", ["a.desktop"]);
        let bus = FakeBus::new()
            .with_name(checks::PORTAL_BUS_NAME)
            .with_property(
                checks::SECRET_SERVICE_BUS_NAME,
                checks::DEFAULT_COLLECTION_PATH,
                checks::COLLECTION_INTERFACE,
                "Locked",
                "false",
            );
        let socket = SocketPath::in_dir("/run/user/1000");
        let mut inputs = inputs(&env, &fs, &bus, &socket, true);
        inputs.wayland = Some(checks::WaylandFindings {
            family: compass_wayland::compositor::Family::Wlroots,
            capabilities: compass_wayland::compositor::Capabilities {
                layer_shell: true,
                toplevel_management: true,
                toplevel_list: true,
                data_control: true,
                hotkey: false,
                virtual_keyboard: true,
                shortcuts_inhibit: true,
            },
            compositor_ipc: None,
        });
        let report = run(&inputs).await;
        let failures: Vec<_> = report
            .checks
            .iter()
            .filter(|c| c.status == DoctorStatus::Fail)
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(!report.has_failures(), "--check-only exits 0");
        let shortcuts = report
            .checks
            .iter()
            .find(|c| c.name == "portal.global-shortcuts")
            .unwrap();
        assert_eq!(shortcuts.status, DoctorStatus::Ok);
        let text = format!("{report:?}");
        for jargon in ["Phase 5", "C++", "PLAN.md", "#118", "#153", "--engine"] {
            assert!(!text.contains(jargon), "{jargon} in the report");
        }
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
        assert_eq!(count, 21);
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
