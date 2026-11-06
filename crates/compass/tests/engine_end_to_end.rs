//! The engine, end to end: a real process, a real socket, the real client.
//!
//! Every other test in this workspace exercises a library. These spawn the
//! `compass` binary as an actual daemon, talk to it over a Unix socket with the
//! same client code a user's shell runs, and assert on what comes back. That is
//! the only level at which "the port runs" is a claim rather than a hope.
//!
//! Each test gets its own socket under a `TempDir` and its own `XDG_DATA_DIRS`
//! pointing at a fixture tree, so nothing here can see — or be perturbed by —
//! the invoking user's real session or real applications.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// compass-shell's private `dbus-daemon` and mock GNOME Shell extension, for
/// the tests that need the extension's windows.
#[path = "../../compass-shell/tests/support/bus.rs"]
#[allow(dead_code)]
mod shell_bus;
#[path = "../../compass-shell/tests/support/mock.rs"]
#[allow(dead_code)]
mod shell_mock;

/// A session bus address with nothing behind it, for every engine this file
/// starts. The engine opens clipboard history through the login keyring on
/// the session bus; pointed at a real one, running these tests would create a
/// Compass key in the developer's own keyring.
const NO_SESSION_BUS: &str = "unix:path=/nonexistent/compass-test-no-session-bus";

/// How long to wait for the daemon to bind before calling it a failure.
///
/// Generous because CI runners are slow and a flaky timeout here would be
/// indistinguishable from a real bind failure, which is the thing worth
/// catching.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// A release feed nothing listens on (the discard port), so an engine a
/// test starts never asks GitHub.
const NO_UPDATE_FEED: &str = "http://127.0.0.1:9/releases/latest";

fn binary() -> PathBuf {
    // The integration-test binary lives next to the crate's binaries.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("compass")
}

#[test]
fn graphical_session_starts_an_engine_and_reaps_only_its_own_child() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let mut command = Command::new(binary());
    command
        .arg("--socket")
        .arg(socket.as_path())
        .args(["serve", "--no-hotkey"])
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_DATA_DIRS", dir.path().join("empty"))
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .env("HOME", dir.path())
        .env_remove("XDG_STATE_HOME")
        .env("COMPASS_UPDATE_FEED_URL", NO_UPDATE_FEED)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = compass::session::ensure(&socket, &mut command, STARTUP_TIMEOUT)
            .await
            .unwrap();
        assert!(compass::ipc::ping(&socket).await.is_ok());
        let mut must_not_spawn = Command::new("/does/not/exist");
        let guest = compass::session::ensure(&socket, &mut must_not_spawn, STARTUP_TIMEOUT)
            .await
            .unwrap();
        drop(guest);
        assert!(
            compass::ipc::ping(&socket).await.is_ok(),
            "reusing must not stop the engine"
        );
        drop(owner);
        assert!(
            compass::ipc::ping(&socket).await.is_err(),
            "owned engine must be reaped"
        );
    });
}

/// The Phase 7 rename: an engine starting over a pre-rename install moves
/// each `vicinae` base directory to `compass` (or into the `compass` one the
/// pre-cutover engine already kept), renames `vicinae.json`, and leaves
/// symlinks at the old paths so an older binary still finds its data.
#[test]
fn the_engine_carries_a_pre_rename_install_over_on_start() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (config, data, cache, state) = (
        root.join("config"),
        root.join("data"),
        root.join("cache"),
        root.join("state"),
    );
    std::fs::create_dir_all(config.join("vicinae")).unwrap();
    std::fs::write(config.join("vicinae/vicinae.json"), "{}").unwrap();
    std::fs::create_dir_all(data.join("vicinae/extensions/hello")).unwrap();
    // The pre-cutover engine already kept its Rhai scripts under `compass`.
    std::fs::create_dir_all(data.join("compass/scripts")).unwrap();
    std::fs::create_dir_all(cache.join("vicinae/file-indexer")).unwrap();
    std::fs::create_dir_all(state.join("vicinae")).unwrap();
    std::fs::write(state.join("vicinae/onboarding.json"), "{}").unwrap();

    let socket = compass_ipc::SocketPath::exact(root.join("ipc.sock"));
    let mut command = Command::new(binary());
    command
        .arg("--socket")
        .arg(socket.as_path())
        .args(["serve", "--no-hotkey"])
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", root.join("empty"))
        .env("XDG_CACHE_HOME", &cache)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", root)
        .env("COMPASS_UPDATE_FEED_URL", NO_UPDATE_FEED)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let owner = compass::session::ensure(&socket, &mut command, STARTUP_TIMEOUT)
            .await
            .unwrap();

        for base in [&config, &data, &cache, &state] {
            let legacy = base.join("vicinae");
            assert!(base.join("compass").is_dir(), "{}", base.display());
            assert_eq!(
                std::fs::read_link(&legacy).ok().as_deref(),
                Some(Path::new("compass")),
                "{} is left as a symlink",
                legacy.display()
            );
        }
        assert!(config.join("compass/compass.json").is_file());
        assert_eq!(
            std::fs::read_link(config.join("compass/vicinae.json"))
                .ok()
                .as_deref(),
            Some(Path::new("compass.json"))
        );
        assert!(data.join("compass/extensions/hello").is_dir());
        assert!(data.join("compass/scripts").is_dir());
        assert!(state.join("compass/onboarding.json").is_file());
        drop(owner);
    });
}

#[test]
fn graphical_session_reports_engine_startup_failure() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime
        .block_on(compass::session::ensure(
            &socket,
            &mut Command::new("/bin/false"),
            STARTUP_TIMEOUT,
        ))
        .expect_err("failed child must not count as ready");
    assert!(error.to_string().contains("exited before becoming ready"));
}

#[test]
fn graphical_session_does_not_replace_an_unresponsive_socket_owner() {
    let dir = TempDir::new().unwrap();
    let socket = compass_ipc::SocketPath::exact(dir.path().join("ipc.sock"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = tokio::net::UnixListener::bind(socket.as_path()).unwrap();
        let server = tokio::spawn(async move {
            let (_connection, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let error = compass::session::ensure(
            &socket,
            &mut Command::new("/does/not/exist"),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(
            !server.is_finished(),
            "existing socket owner must be left alone"
        );
        server.abort();
    });
}

/// A `.desktop` fixture tree, laid out the way XDG expects.
fn write_apps(root: &Path, entries: &[(&str, &str)]) {
    let dir = root.join("applications");
    std::fs::create_dir_all(&dir).expect("create applications dir");
    for (name, body) in entries {
        std::fs::write(dir.join(name), body).expect("write desktop entry");
    }
}

fn entry(name: &str, extra: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n{extra}")
}

/// A daemon running on its own socket, killed on drop.
struct Daemon {
    child: Child,
    socket: PathBuf,
    _dirs: TempDir,
}

impl Daemon {
    fn start(entries: &[(&str, &str)]) -> Daemon {
        Self::start_with_config(entries, "{}")
    }

    fn start_with_config(entries: &[(&str, &str)], config: &str) -> Daemon {
        Self::start_prepared(entries, config, |_| Vec::new())
    }

    /// As [`Self::start_with_config`], with `prepare` given the tempdir root
    /// before the engine starts, returning extra environment for it.
    fn start_prepared(
        entries: &[(&str, &str)],
        config: &str,
        prepare: impl FnOnce(&std::path::Path) -> Vec<(&'static str, std::ffi::OsString)>,
    ) -> Daemon {
        let dirs = TempDir::new().expect("tempdir");
        let extra_env = prepare(dirs.path());
        let data = dirs.path().join("data");
        write_apps(&data, entries);
        let config_dir = dirs.path().join("config/compass");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("compass.json"), config).unwrap();

        let socket = dirs.path().join("ipc.sock");
        let child = Command::new(binary())
            .arg("--socket")
            .arg(&socket)
            .arg("serve")
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .env("XDG_DATA_DIRS", &data)
            // Keep the daemon out of the invoking user's home entirely: its
            // config, its launch history and its data all land in the tempdir.
            .env("XDG_DATA_HOME", dirs.path().join("data-home"))
            .env("XDG_CONFIG_HOME", dirs.path().join("config"))
            // The file indexer's database, which a real cache home must never
            // receive from a test.
            .env("XDG_CACHE_HOME", dirs.path().join(".cache"))
            .env("HOME", dirs.path())
            // Nor its log, which goes under the home the line above set.
            .env_remove("XDG_STATE_HOME")
            // Nor its compositor: on a wlroots session the engine would answer
            // window requests over Wayland (`tests/wlroots_engine.rs`).
            .env_remove("WAYLAND_DISPLAY")
            // Nor its compositor's own socket (Hyprland, niri), nor its
            // desktop: the wallpaper backends are chosen by name.
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("NIRI_SOCKET")
            .env_remove("XDG_CURRENT_DESKTOP")
            .env_remove("GDMSESSION")
            // Nor the network: a test's HTTP goes to its own local fake,
            // never through a proxy the invoking shell set.
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("ALL_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env("NO_PROXY", "127.0.0.1,localhost")
            // Nor the ECB: no exchange rates are fetched unless a test
            // serves its own and turns the refresh back on.
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env(
                "COMPASS_EXCHANGE_RATES_URL",
                "http://127.0.0.1:9/no-network",
            )
            // Nor the machine's keyboards: a helper that exits at once
            // stands in for compass-input-server unless a test brings its
            // own fake.
            .env("COMPASS_INPUT_SERVER_BIN", "/bin/true")
            // Nor GitHub: the update check asks a closed local port unless a
            // test brings its own feed.
            .env("COMPASS_UPDATE_FEED_URL", NO_UPDATE_FEED)
            // Nor its supervisor: under systemd the tray has no Quit.
            .env_remove("INVOCATION_ID")
            .envs(extra_env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the engine");

        let daemon = Daemon {
            child,
            socket,
            _dirs: dirs,
        };
        daemon.wait_until_listening();
        daemon
    }

    /// Blocks until the socket answers, or panics with why it never did.
    fn wait_until_listening(&self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if self.try_client(&["ping"]).status.success() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "the engine never started listening on {} within {STARTUP_TIMEOUT:?}",
            self.socket.display()
        );
    }

    fn try_client(&self, args: &[&str]) -> std::process::Output {
        Command::new(binary())
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .expect("run the client")
    }

    /// Runs a client command and asserts it succeeded, returning stdout.
    fn client(&self, args: &[&str]) -> String {
        let out = self.try_client(args);
        assert!(
            out.status.success(),
            "`compass {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    fn request(&self, request: compass_ipc::Request) -> compass_ipc::Response {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                compass_ipc::Client::connect(&self.socket)
                    .await
                    .unwrap()
                    .request(request)
                    .await
                    .unwrap()
            })
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------

/// `AppService`'s directory watch: an application installed while the engine
/// runs is found without a restart, one removed is gone, and the catalog
/// generation a window compares says so.
#[test]
fn an_application_installed_while_the_engine_runs_is_found_without_a_restart() {
    use compass_ipc::{Request, Response};
    let alpha = entry("Alpha Editor", "");
    let daemon = Daemon::start(&[("alpha.desktop", alpha.as_str())]);
    let ids = |query: &str| -> Vec<String> {
        let Response::QueryResults { hits } = daemon.request(Request::Query { text: query.into() })
        else {
            panic!("expected query results");
        };
        hits.into_iter()
            .map(|hit| hit.id)
            .filter(|id| id.starts_with("applications:"))
            .collect()
    };
    let generation = || match daemon.request(Request::CatalogGeneration) {
        Response::CatalogGeneration { generation } => generation,
        other => panic!("unexpected answer: {other:?}"),
    };
    assert_eq!(generation(), 0);
    assert!(ids("Zephyr").is_empty());

    let apps = daemon._dirs.path().join("data/applications");
    std::fs::write(apps.join("zephyr.desktop"), entry("Zephyr Mail", "")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while ids("Zephyr").is_empty() {
        assert!(
            Instant::now() < deadline,
            "the new application never appeared"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(ids("Zephyr"), ["applications:zephyr"]);
    assert!(generation() >= 1);

    std::fs::remove_file(apps.join("alpha.desktop")).unwrap();
    while !ids("Alpha").is_empty() {
        assert!(Instant::now() < deadline, "the removed application stayed");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `ExtensionRegistry`'s directory watch: an extension a developer builds
/// into place (the directory first, the manifest after, as `compass develop`
/// does) joins root search without a restart, and leaves it when removed.
#[test]
fn an_extension_built_into_place_while_the_engine_runs_joins_root_search() {
    use compass_ipc::{Request, Response};
    let daemon = Daemon::start_prepared(&[], "{}", |root| {
        std::fs::create_dir_all(root.join("data-home/compass/extensions")).unwrap();
        Vec::new()
    });
    let ids = |query: &str| -> Vec<String> {
        let Response::QueryResults { hits } = daemon.request(Request::Query { text: query.into() })
        else {
            panic!("expected query results");
        };
        hits.into_iter()
            .map(|hit| hit.id)
            .filter(|id| id.starts_with('@'))
            .collect()
    };
    let generation = || match daemon.request(Request::CatalogGeneration) {
        Response::CatalogGeneration { generation } => generation,
        other => panic!("unexpected answer: {other:?}"),
    };
    assert!(ids("Zulu Clock").is_empty());

    let extension = daemon
        ._dirs
        .path()
        .join("data-home/compass/extensions/zulu-clock");
    std::fs::create_dir_all(&extension).unwrap();
    // Long enough for the first rescan to find no manifest yet.
    std::thread::sleep(Duration::from_millis(400));
    std::fs::write(
        extension.join("package.json"),
        r#"{"name": "zulu-clock", "title": "Zulu Clock", "author": "dev",
            "commands": [{"name": "show", "title": "Zulu Clock", "mode": "view"}]}"#,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while ids("Zulu Clock").is_empty() {
        assert!(
            Instant::now() < deadline,
            "the built extension never appeared"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(ids("Zulu Clock"), ["@dev/zulu-clock:show"]);
    let seen = generation();
    assert!(seen >= 1);

    std::fs::remove_dir_all(&extension).unwrap();
    while !ids("Zulu Clock").is_empty() {
        assert!(Instant::now() < deadline, "the removed extension stayed");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(generation() > seen);
}

/// Set Default Browser and Set Default Terminal against a real engine: the
/// candidates with the current default first, and a choice written to the
/// user's `mimeapps.list` or `xdg-terminals.list` under the temp config home.
#[test]
fn the_default_browser_and_terminal_are_listed_and_set_in_the_users_files() {
    use compass_ipc::{DefaultAppKind, Request, Response};
    let web = entry("Web", "MimeType=x-scheme-handler/https;text/html;\n");
    let surf = entry(
        "Surf",
        "Comment=A small browser\nMimeType=x-scheme-handler/https;\n",
    );
    let editor = entry("Editor", "MimeType=text/plain;\n");
    let xterm = entry("XTerm", "Categories=System;TerminalEmulator;\n");
    let kitty = entry("Kitty", "Categories=System;TerminalEmulator;\n");
    let daemon = Daemon::start(&[
        ("web.desktop", web.as_str()),
        ("surf.desktop", surf.as_str()),
        ("editor.desktop", editor.as_str()),
        ("xterm.desktop", xterm.as_str()),
        ("kitty.desktop", kitty.as_str()),
    ]);
    let listed = |kind| match daemon.request(Request::ListDefaultApps { kind }) {
        Response::DefaultApps { apps } => apps
            .into_iter()
            .map(|app| (app.id, app.is_default))
            .collect::<Vec<_>>(),
        other => panic!("unexpected answer: {other:?}"),
    };
    let set = |kind, id: &str| {
        daemon.request(Request::SetDefaultApp {
            kind,
            id: id.to_owned(),
        })
    };
    let config = daemon._dirs.path().join("config");

    let browsers = listed(DefaultAppKind::Browser);
    assert_eq!(
        browsers.len(),
        2,
        "what opens https, not the editor: {browsers:?}"
    );
    assert_eq!(
        browsers[0],
        ("surf.desktop".to_owned(), true),
        "the first opener"
    );
    assert_eq!(set(DefaultAppKind::Browser, "web.desktop"), Response::Ack);
    let written = std::fs::read_to_string(config.join("mimeapps.list")).unwrap();
    assert!(
        written.contains("x-scheme-handler/http=web.desktop"),
        "{written}"
    );
    assert!(
        written.contains("application/xhtml+xml=web.desktop"),
        "{written}"
    );
    assert_eq!(
        listed(DefaultAppKind::Browser)[0],
        ("web.desktop".to_owned(), true)
    );

    let terminals = listed(DefaultAppKind::Terminal);
    let mut ids: Vec<&str> = terminals.iter().map(|(id, _)| id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["kitty.desktop", "xterm.desktop"]);
    assert_eq!(
        set(DefaultAppKind::Terminal, "xterm.desktop"),
        Response::Ack
    );
    let written = std::fs::read_to_string(config.join("xdg-terminals.list")).unwrap();
    assert!(
        written.lines().any(|line| line == "xterm.desktop"),
        "{written}"
    );
    assert_eq!(
        listed(DefaultAppKind::Terminal)[0],
        ("xterm.desktop".to_owned(), true)
    );
}

#[test]
fn the_fallback_manager_writes_the_users_fallbacks() {
    use compass_ipc::{Request, Response, RootItemEdit};
    let daemon = Daemon::start_with_config(&[("a.desktop", &entry("Alpha", ""))], "{}");
    let edit = |id: &str, edit| {
        daemon.request(Request::RootItemEdit {
            id: id.to_owned(),
            edit,
        })
    };
    let written = || -> serde_json::Value {
        let path = daemon._dirs.path().join("config/compass/compass.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    assert_eq!(
        edit("applications:a", RootItemEdit::Fallback(true)),
        Response::Ack
    );
    assert_eq!(
        written()["fallbacks"],
        serde_json::json!(["applications:a", "files:search"]),
        "first, before the default"
    );
    assert_eq!(
        edit("files:search", RootItemEdit::Fallback(false)),
        Response::Ack,
        "Search Files by the C++'s id, which names no root item here"
    );
    assert_eq!(
        written()["fallbacks"],
        serde_json::json!(["applications:a"])
    );
    assert!(matches!(
        edit("nothing:here", RootItemEdit::Fallback(true)),
        Response::Error(_)
    ));
}

#[test]
fn daemon_search_reads_application_aliases_and_enabled_precedence_from_config() {
    use compass_ipc::{Request, Response};
    let entries = [
        ("alpha.desktop", entry("Alpha Editor", "")),
        ("beta.desktop", entry("Beta Editor", "")),
    ];
    let entries = entries
        .iter()
        .map(|(id, body)| (*id, body.as_str()))
        .collect::<Vec<_>>();
    // `applications:beta`, not `beta.desktop` — the same `provider:entrypoint`
    // form the config below addresses these entries by.
    for (enabled, expected) in [(true, vec!["applications:beta"]), (false, vec![])] {
        let config = serde_json::json!({"providers": {"applications": {
            "enabled": enabled,
            "entrypoints": {
                "alpha": {"enabled": false},
                "beta": {"enabled": true, "alias": "uniquealias"}
            }
        }}});
        let daemon = Daemon::start_with_config(&entries, &config.to_string());
        for query in ["", "Editor", "uniquealias"] {
            let Response::QueryResults { hits } =
                daemon.request(Request::Query { text: query.into() })
            else {
                panic!("expected query result");
            };
            // Applications only: this is about their config. Builtin
            // commands rank in the same list and have their own test.
            assert_eq!(
                hits.iter()
                    .map(|hit| hit.id.as_str())
                    .filter(|id| id.starts_with("applications:"))
                    .collect::<Vec<_>>(),
                expected,
                "{query}"
            );
        }
    }
}

#[test]
fn the_engine_starts_and_answers_a_ping() {
    let daemon = Daemon::start(&[("true.desktop", &entry("True", ""))]);
    let out = daemon.client(&["ping"]);
    assert!(out.contains("engine alive"), "{out}");
    assert!(out.contains("protocol v"), "{out}");
}

#[test]
fn a_query_finds_an_application_from_the_indexed_directory() {
    let daemon = Daemon::start(&[
        ("editor.desktop", &entry("Text Editor", "Comment=Edit text")),
        ("browser.desktop", &entry("Web Browser", "")),
        ("calc.desktop", &entry("Calculator", "")),
    ]);

    let out = daemon.client(&["query", "text"]);
    assert!(out.contains("Text Editor"), "{out}");
    assert!(
        !out.contains("Calculator"),
        "a non-match was returned: {out}"
    );
}

#[test]
fn a_multi_word_query_is_joined_before_it_reaches_the_engine() {
    let daemon = Daemon::start(&[("editor.desktop", &entry("Text Editor", ""))]);
    let joined = daemon.client(&["query", "text", "editor"]);
    let quoted = daemon.client(&["query", "text editor"]);
    assert_eq!(joined, quoted);
    assert!(joined.contains("Text Editor"), "{joined}");
}

#[test]
fn reporting_a_launch_persists_history_and_changes_root_order() {
    use compass_core::FrecencyStore;
    use compass_ipc::{Request, Response};
    let daemon = Daemon::start(&[
        ("alpha.desktop", &entry("Alpha Editor", "")),
        ("beta.desktop", &entry("Beta Editor", "")),
    ]);
    let query = || {
        let Response::QueryResults { hits } = daemon.request(Request::Query {
            text: "Editor".to_owned(),
        }) else {
            panic!("query failed")
        };
        hits
    };
    let before = query();
    assert_eq!(before[0].id, "applications:alpha");
    assert_eq!(
        daemon.request(Request::RecordLaunch {
            key: "beta.desktop".to_owned()
        }),
        Response::Ack
    );
    let after = query();
    assert_eq!(after[0].id, "applications:beta");
    assert_eq!(
        after[0].score, before[1].score,
        "wire score excludes history"
    );
    let reopened = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/compass/frecency.json"),
    )
    .unwrap();
    let record = reopened.record("beta.desktop").unwrap();
    assert_eq!(record.launch_count, 1);
    assert!(record.last_launched_at.is_some());
    assert!(reopened.record("alpha.desktop").is_none());
}

#[test]
fn unknown_launch_keys_do_not_create_history_and_persistence_errors_are_reported() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let path = daemon._dirs.path().join("data-home/compass/frecency.json");
    for key in ["", "unknown.desktop", "../alpha.desktop"] {
        assert!(matches!(
            daemon.request(Request::RecordLaunch { key: key.to_owned() }),
            Response::Error(error) if error.kind == ErrorKind::BadRequest
        ));
    }
    assert!(!path.exists());
    std::fs::create_dir_all(&path).unwrap();
    assert!(matches!(
        daemon.request(Request::RecordLaunch { key: "alpha.desktop".to_owned() }),
        Response::Error(error) if error.kind == ErrorKind::Internal
    ));
    assert!(matches!(
        daemon.request(Request::Ping),
        Response::Pong { .. }
    ));
}

#[test]
fn concurrent_launch_reports_do_not_lose_visits() {
    use compass_core::FrecencyStore;
    use compass_ipc::{Client, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut requests = tokio::task::JoinSet::new();
            for _ in 0..16 {
                let socket = daemon.socket.clone();
                requests.spawn(async move {
                    Client::connect(socket)
                        .await
                        .unwrap()
                        .request(Request::RecordLaunch {
                            key: "alpha.desktop".to_owned(),
                        })
                        .await
                        .unwrap()
                });
            }
            while let Some(result) = requests.join_next().await {
                assert_eq!(result.unwrap(), Response::Ack);
            }
        });
    let reopened = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/compass/frecency.json"),
    )
    .unwrap();
    assert_eq!(reopened.record("alpha.desktop").unwrap().launch_count, 16);
}

#[test]
fn ui_search_and_successful_launch_share_the_real_daemons_history() {
    use compass_core::FrecencyStore;
    use compass_platform::{AppLauncher, LaunchFuture, LaunchMethod};
    use compass_ui::{LauncherApp, Message};
    use futures_util::StreamExt;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct Launcher(Mutex<Vec<String>>);
    impl AppLauncher for Launcher {
        fn launch<'a>(
            &'a self,
            entry: &'a compass_xdg::DesktopEntry,
            _uris: &'a [&'a str],
        ) -> LaunchFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(entry.name().to_owned());
                Ok(LaunchMethod::Direct)
            })
        }
    }

    let daemon = Daemon::start(&[
        ("alpha.desktop", &entry("Alpha Editor", "")),
        ("beta.desktop", &entry("Beta Editor", "")),
    ]);
    let launcher = Arc::new(Launcher::default());
    let build_ui = || {
        LauncherApp::with_index(
            compass_core::AppIndex::builder()
                .dir(daemon._dirs.path().join("data/applications"))
                .build(),
        )
        .with_launcher(launcher.clone())
        .with_backend(Arc::new(compass::ui_backend::DaemonBackend::new(
            compass_ipc::SocketPath::exact(&daemon.socket),
        )))
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let drive = |app: &mut LauncherApp, message| {
        let task = app.update(message);
        runtime.block_on(async {
            if let Some(mut stream) = iced_winit::runtime::task::into_stream(task) {
                while let Some(action) = stream.next().await {
                    if let iced_winit::runtime::Action::Output(message) = action {
                        let _ = app.update(message);
                    }
                }
            }
        });
    };
    let mut app = build_ui();
    drive(&mut app, Message::QueryChanged("Editor".to_owned()));
    assert_eq!(app.selected_item().unwrap().key(), "alpha.desktop");
    drive(&mut app, Message::ResultSelected(1));
    drive(&mut app, Message::LaunchSelected);
    assert_eq!(*launcher.0.lock().unwrap(), ["Beta Editor"]);
    let mut reopened_ui = build_ui();
    drive(
        &mut reopened_ui,
        Message::Opened(iced::window::Id::unique()),
    );
    assert_eq!(reopened_ui.selected_item().unwrap().key(), "beta.desktop");
    let history = compass_core::JsonFrecencyStore::open(
        daemon._dirs.path().join("data-home/compass/frecency.json"),
    )
    .unwrap();
    assert_eq!(history.record("beta.desktop").unwrap().launch_count, 1);
}

#[test]
fn a_desktop_link_without_exec_is_returned_by_root_search() {
    let daemon = Daemon::start(&[(
        "manual.desktop",
        "[Desktop Entry]\nType=Link\nName=Reference Manual\nURL=file:///usr/share/doc/manual.html\n",
    )]);
    for query in ["", "Reference"] {
        let out = daemon.client(&["query", query, "--json", "--provider", "applications"]);
        let rows: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        // The ENTRYPOINT id, which is what the protocol documents this field
        // as and what the C++ engine answers. It read `manual.desktop` until
        // Suite 0's first differential put the two engines side by side.
        assert_eq!(rows[0]["id"], "applications:manual");
        assert_eq!(rows[0]["title"], "Reference Manual");
    }
}

#[test]
fn an_empty_query_lists_everything_rather_than_nothing() {
    // The pre-typing state of a launcher. Returning nothing here would make the
    // window open blank, which reads as broken.
    let daemon = Daemon::start(&[
        ("a.desktop", &entry("Alpha", "")),
        ("b.desktop", &entry("Beta", "")),
    ]);
    let out = daemon.client(&["query", ""]);
    assert!(out.contains("Alpha"), "{out}");
    assert!(out.contains("Beta"), "{out}");
}

#[test]
fn a_query_that_matches_nothing_says_so() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.client(&["query", "zzzzzzqqq"]);
    assert_eq!(out.trim(), "no matches");
}

#[test]
fn query_json_is_machine_readable_and_carries_the_documented_fields() {
    let daemon = Daemon::start(&[(
        "editor.desktop",
        &entry("Text Editor", "GenericName=Editor"),
    )]);
    let out = daemon.client(&["query", "--json", "text"]);
    let hits: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let first = &hits[0];

    assert_eq!(first["title"], "Text Editor");
    assert!(
        first["subtitle"].is_null(),
        "application descriptions are not root subtitles"
    );
    assert!(first["id"].is_string(), "{first}");

    let score = first["score"].as_u64().expect("a numeric score");
    assert!(
        score <= 100,
        "score {score} is outside the documented 0..=100"
    );
}

#[test]
fn queries_use_root_provider_fields_and_do_not_return_desktop_actions() {
    let daemon = Daemon::start(&[(
        "browser.desktop",
        &entry(
            "Browser",
            "TryExec=compass-unresolved-sentinel\nComment=DescriptionSentinel\nActions=private;\n[Desktop Action private]\nName=Private Window\nExec=browser --private\n",
        ),
    )]);
    let all: serde_json::Value = serde_json::from_str(&daemon.client(&[
        "query",
        "--json",
        "--provider",
        "applications",
        "",
    ]))
    .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["id"], "applications:browser");
    for query in ["DescriptionSentinel", "Private"] {
        let hits: serde_json::Value =
            serde_json::from_str(&daemon.client(&["query", "--json", query])).unwrap();
        assert!(hits.as_array().unwrap().is_empty(), "{query}: {hits}");
    }
}

#[test]
fn the_window_commands_refuse_rather_than_pretend() {
    // The point of the design: a client must be able to tell "there is no
    // window" from "the window was shown". If these ever start answering Ack
    // without a UI behind them, first bring-up debugs a lie.
    //
    // Asserted on the *properties* of the refusal rather than on its wording.
    // An earlier version checked `stderr.contains("headless")`, which broke the
    // moment ADR-0015 made the refusal name a missing connection instead of a
    // headless build -- a more accurate message failing a test is the test
    // being wrong, not the message.
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    for command in ["show", "hide", "toggle"] {
        let out = daemon.try_client(&[command]);
        assert!(
            !out.status.success(),
            "`{command}` reported success with no window"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        assert!(
            stderr.contains("window"),
            "`{command}` refused without naming what is missing: {stderr}"
        );
        // Actionable: a refusal that does not say what to do about it sends
        // the reader to the source.
        assert!(
            stderr.contains("compass ui"),
            "`{command}` refused without saying how to fix it: {stderr}"
        );
    }
}

#[test]
fn doctor_runs_against_the_live_engine() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.client(&["doctor", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(report["checks"].as_array().is_some_and(|c| !c.is_empty()));
}

#[test]
fn shutdown_is_answered_before_the_engine_goes_away() {
    // The response has to be written before the serve loop stops, or the client
    // sees its connection drop and reports a crash instead of a clean stop.
    let mut daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = daemon.try_client(&["shutdown"]);
    assert!(
        out.status.success(),
        "shutdown did not report success: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match daemon.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "the engine exited with {status}");
                break;
            }
            None if Instant::now() >= deadline => panic!("the engine did not exit after shutdown"),
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }

    // And the socket file is gone, so the next start does not have to reclaim it.
    assert!(
        !daemon.socket.exists(),
        "the socket file outlived the engine"
    );
}

#[test]
fn a_second_engine_on_the_same_socket_refuses_to_start() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let out = Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("serve")
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
        .env("HOME", daemon._dirs.path())
        .env_remove("XDG_STATE_HOME")
        .output()
        .expect("run a second engine");

    assert!(!out.status.success(), "two engines bound the same socket");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("already running"),
        "the second engine did not say why: {stderr}"
    );
}

#[test]
fn a_client_with_no_engine_explains_itself() {
    let dir = TempDir::new().expect("tempdir");
    let socket = dir.path().join("absent.sock");
    let out = Command::new(binary())
        .arg("--socket")
        .arg(&socket)
        .arg("ping")
        .output()
        .expect("run the client");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&socket.display().to_string()),
        "the error did not name the path it tried: {stderr}"
    );
}

// --- the resident window, against a real daemon -----------------------------
//
// ADR-0015: `serve` forwards `show`/`hide`/`toggle` to a window that attached
// itself over the same socket. These tests *are* that window: the test process
// attaches a `WindowClient` to a real spawned daemon and answers what it is
// asked, which exercises the handover, the push and the reply across a process
// boundary rather than inside one.

/// A fake launcher window attached to a running daemon, on its own thread.
///
/// Answers every command with `reports`, and records what it was asked.
struct FakeWindow {
    seen: std::sync::Arc<std::sync::Mutex<Vec<compass_ipc::WindowCommand>>>,
    /// Told on drop, so the loop returns and the socket closes.
    ///
    /// Without it, dropping the window while the daemon still holds the link
    /// joins a thread parked forever in `next_command`. A test that wants to
    /// kill the window has to be able to do it while the engine is still
    /// alive, which is exactly what the death test needs.
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeWindow {
    fn attach(socket: &Path, reports: compass_ipc::WindowOutcome) -> Self {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

        let thread = {
            let seen = std::sync::Arc::clone(&seen);
            let socket = socket.to_path_buf();
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let mut window = compass_ipc::WindowClient::attach(&socket)
                        .await
                        .expect("attach to the engine");
                    ready_tx.send(()).expect("signal attached");

                    loop {
                        tokio::select! {
                            // `biased` so a pending stop wins over a command
                            // that arrived in the same poll: a window asked to
                            // die should die rather than answer once more.
                            biased;
                            _ = &mut stop_rx => break,
                            command = window.next_command() => {
                                match command.expect("read a command") {
                                    Some(command) => {
                                        seen.lock().expect("lock").push(command);
                                        window.reply(reports.clone()).await.expect("reply");
                                    }
                                    // The engine closed the link.
                                    None => break,
                                }
                            }
                        }
                    }
                    drop(window);
                });
            })
        };

        // Attach before returning: a test that raced the attach would exercise
        // the refusal path and still look like it passed.
        ready_rx
            .recv_timeout(STARTUP_TIMEOUT)
            .expect("the fake window attached");

        Self {
            seen,
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    fn seen(&self) -> Vec<compass_ipc::WindowCommand> {
        self.seen.lock().expect("lock").clone()
    }
}

impl Drop for FakeWindow {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            // Joining rather than detaching: the death test needs the socket
            // closed by the time this returns, and a detached runtime would
            // close it whenever it got around to it.
            let _ = thread.join();
        }
    }
}

#[test]
fn an_attached_window_turns_the_refusal_into_a_real_show() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    // Control: the same command against the same daemon is refused a moment
    // before the window attaches. Without this the test could pass against an
    // engine that answered `Ack` unconditionally.
    let before = daemon.try_client(&["show"]);
    assert!(
        !before.status.success(),
        "`show` must be refused before a window attaches"
    );

    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);

    let after = daemon.try_client(&["show"]);
    assert!(
        after.status.success(),
        "`show` must succeed with a window attached: {}",
        String::from_utf8_lossy(&after.stderr)
    );
    assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
}

#[test]
fn a_second_ui_invocation_shows_the_existing_window_without_starting_a_renderer() {
    let daemon = Daemon::start(&[]);
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(daemon.socket.with_extension("ui.lock"))
        .unwrap();
    lease.try_lock().unwrap();
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    let output = Command::new(binary())
        .args(["--socket", daemon.socket.to_str().unwrap(), "ui"])
        .env("DISPLAY", ":65534")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
    let hidden = Command::new(binary())
        .args([
            "--socket",
            daemon.socket.to_str().unwrap(),
            "start",
            "--hidden",
        ])
        .env("DISPLAY", ":65534")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();
    assert!(
        hidden.status.success(),
        "{}",
        String::from_utf8_lossy(&hidden.stderr)
    );
    assert_eq!(
        window.seen(),
        vec![compass_ipc::WindowCommand::Show],
        "a duplicate hidden start must not change visibility"
    );
}

#[test]
fn each_command_reaches_the_window_as_itself() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);

    // Three different commands, so a daemon that forwarded a constant would
    // produce a different vector rather than an identical-looking one.
    for command in ["show", "hide", "toggle"] {
        let out = daemon.try_client(&[command]);
        assert!(
            out.status.success(),
            "`{command}` failed with a window attached: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    assert_eq!(
        window.seen(),
        vec![
            compass_ipc::WindowCommand::Show,
            compass_ipc::WindowCommand::Hide,
            compass_ipc::WindowCommand::Toggle,
        ]
    );
}

#[test]
fn the_engines_hud_reaches_the_launchers_hud() {
    use compass_ipc::{DefaultAppKind, Request, Response, WindowCommand};
    let web = entry("Web", "MimeType=x-scheme-handler/https;\n");
    let daemon = Daemon::start(&[("web.desktop", web.as_str())]);
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Hidden);
    assert_eq!(
        daemon.request(Request::SetDefaultApp {
            kind: DefaultAppKind::Browser,
            id: "web.desktop".to_owned(),
        }),
        Response::Ack
    );
    let expected = WindowCommand::Hud {
        text: "Default browser changed".to_owned(),
        icon: Some("globe-01".to_owned()),
    };
    let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
    while !window.seen().contains(&expected) {
        assert!(
            std::time::Instant::now() < deadline,
            "no HUD reached the window: {:?}",
            window.seen()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_window_that_refuses_is_reported_rather_than_acked() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let reason = "no compositor to present on";
    let _window = FakeWindow::attach(
        &daemon.socket,
        compass_ipc::WindowOutcome::Failed(reason.to_owned()),
    );

    let out = daemon.try_client(&["show"]);
    assert!(
        !out.status.success(),
        "a window that refused must not be reported as a success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(reason),
        "the window's own reason should reach the user: {stderr}"
    );
}

#[test]
fn a_window_that_dies_puts_the_engine_back_to_refusing() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    {
        let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
        let out = daemon.try_client(&["show"]);
        assert!(out.status.success(), "control: `show` works while attached");
        assert_eq!(window.seen(), vec![compass_ipc::WindowCommand::Show]);
    }
    // The fake window's runtime is gone here, so its socket is closed.

    // The first request after the death may be the one that discovers it --
    // the engine notices on a push, not before -- so a single failed attempt
    // proves nothing on its own. What must hold is that it *settles* on
    // refusing rather than alternating.
    //
    // What this does *not* pin is the engine clearing its slot: a push to a
    // dead socket fails whether or not the link was dropped, so the refusal
    // looks identical. Confirmed by a control -- these tests still pass with
    // the clearing removed. See `forward` in `serve/shell.rs`.
    for attempt in 0..3 {
        let out = daemon.try_client(&["show"]);
        assert!(
            !out.status.success(),
            "attempt {attempt}: `show` reported success after the window died"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        assert!(
            stderr.contains("compass ui"),
            "attempt {attempt}: the refusal should say how to fix it: {stderr}"
        );
    }
}

#[test]
fn clipboard_history_without_a_keyring_is_refused_by_name() {
    // Every engine here runs with no session bus, so no keyring: the history
    // cannot be opened, and the request must say so rather than answer with
    // an empty list a client would show as "nothing copied yet".
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    for request in [
        compass_ipc::Request::ClipboardHistory {
            query: String::new(),
            limit: 10,
        },
        compass_ipc::Request::ClipboardSetPinned {
            id: "1".into(),
            pinned: true,
        },
        compass_ipc::Request::ClipboardRemove { id: "1".into() },
        compass_ipc::Request::ClipboardHistoryOfKind {
            query: String::new(),
            limit: 10,
            kind: Some(compass_ipc::ClipboardKind::Image),
        },
        compass_ipc::Request::ClipboardDetail { id: "1".into() },
        compass_ipc::Request::ClipboardSetKeywords {
            id: "1".into(),
            keywords: "x".into(),
        },
        compass_ipc::Request::ClipboardRemoveAll,
    ] {
        let response = daemon.request(request.clone());
        let compass_ipc::Response::Error(err) = response else {
            panic!("expected {request:?} to be refused, got {response:?}");
        };
        assert_eq!(err.kind, compass_ipc::ErrorKind::Unsupported, "{request:?}");
        assert!(err.message.contains("keyring"), "{}", err.message);
    }
}

#[test]
fn pausing_the_clipboard_is_answered_and_kept_as_the_monitoring_preference() {
    use compass_ipc::{Request, Response};
    let config = serde_json::json!({
        "launcher": {"max_results": 7},
        "providers": {"clipboard": {"preferences": {"evictionThreshold": "3600"}}}
    });
    let daemon =
        Daemon::start_with_config(&[("a.desktop", &entry("Alpha", ""))], &config.to_string());
    let asked = daemon.request(Request::ClipboardMonitoring { enabled: None });
    assert_eq!(
        asked,
        Response::ClipboardMonitoring {
            // No session bus: nothing watches the selection here.
            supported: false,
            enabled: true,
        }
    );
    let paused = daemon.request(Request::ClipboardMonitoring {
        enabled: Some(false),
    });
    assert_eq!(
        paused,
        Response::ClipboardMonitoring {
            supported: false,
            enabled: false,
        }
    );
    let written: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(daemon._dirs.path().join("config/compass/compass.json"))
            .expect("the config"),
    )
    .expect("json");
    assert_eq!(
        written["providers"]["clipboard"]["preferences"],
        serde_json::json!({"evictionThreshold": "3600", "monitoring": false})
    );
    assert_eq!(written["launcher"]["max_results"], 7, "the rest is kept");
}

#[test]
fn a_clipboard_request_for_no_entries_is_a_bad_request() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let response = daemon.request(compass_ipc::Request::ClipboardHistory {
        query: String::new(),
        limit: 0,
    });
    let compass_ipc::Response::Error(err) = response else {
        panic!("expected a refusal, got {response:?}");
    };
    assert_eq!(err.kind, compass_ipc::ErrorKind::BadRequest);
}

#[test]
fn builtin_commands_rank_in_the_root_and_their_use_is_remembered() {
    use compass_ipc::{Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "clipboard".into(),
    }) else {
        panic!("expected query results");
    };
    assert_eq!(
        hits.first().map(|h| h.id.as_str()),
        Some("commands:clipboard-history")
    );
    assert_eq!(hits[0].title, "Clipboard History");

    // The provider flag narrows either way.
    let commands: serde_json::Value =
        serde_json::from_str(&daemon.client(&["query", "--json", "--provider", "commands", ""]))
            .unwrap();
    assert!(commands.as_array().unwrap().iter().all(|row| {
        row["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("commands:"))
    }));

    // Opening it counts, like launching an application.
    assert!(matches!(
        daemon.request(Request::RecordLaunch {
            key: "commands:clipboard-history".into()
        }),
        Response::Ack
    ));
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: String::new(),
    }) else {
        panic!("expected query results");
    };
    assert_eq!(
        hits.first().map(|h| h.id.as_str()),
        Some("commands:clipboard-history"),
        "the most-used row leads the empty query"
    );
}

#[test]
fn window_requests_without_a_session_bus_are_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    for request in [
        Request::ListWindows,
        Request::ActivateWindow { id: 1 },
        Request::CloseWindow { id: 1 },
        // Before the store is consulted: without a Shell there is nothing to
        // paste into, whether or not the id names an entry.
        Request::ClipboardPaste { id: "1".into() },
        // The emoji picker's paste: the window copies on this refusal.
        Request::PasteText {
            text: "👋".into()
        },
    ] {
        let Response::Error(err) = daemon.request(request.clone()) else {
            panic!("{request:?} was not refused");
        };
        assert_eq!(err.kind, ErrorKind::Unsupported, "{request:?}");
        assert!(err.message.contains("session bus"), "{}", err.message);
    }
}

/// An installed extension with one no-view command that writes `out`, and one
/// view command.
fn install_extension(root: &std::path::Path) -> std::path::PathBuf {
    let ext = root.join("data-home/compass/extensions/hello");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "hello", "title": "Hello", "author": "someone",
            "commands": [
              {"name": "write", "title": "Write Greeting", "mode": "no-view"},
              {"name": "show", "title": "Show Greeting", "mode": "view"},
              {"name": "nav", "title": "Navigate", "mode": "view"},
              {"name": "ask", "title": "Ask First", "mode": "view"},
              {"name": "auth", "title": "Sign In", "mode": "view"},
              {"name": "link", "title": "Open Link", "mode": "no-view"},
              {"name": "term", "title": "In Terminal", "mode": "no-view"},
              {"name": "tiles", "title": "Tiles", "mode": "view"},
              {"name": "toaster", "title": "Toaster", "mode": "view"},
              {"name": "heap", "title": "Heap", "mode": "no-view"},
              {"name": "machine", "title": "Machine", "mode": "view"},
              {"name": "probe", "title": "Probe", "mode": "no-view",
               "preferences": [{"name": "limit", "title": "Limit", "type": "textfield",
                                "required": false, "default": "20"}]},
              {"name": "issue", "title": "New Issue", "mode": "view"},
              {"name": "greet", "title": "Greet Someone", "mode": "no-view",
               "arguments": [{"name": "name", "type": "text", "placeholder": "Name",
                              "required": true}]},
              {"name": "needs", "title": "Needs Token", "mode": "no-view",
               "preferences": [{"name": "token", "title": "API Token", "type": "password",
                                "required": true}]}
            ]}"#,
    )
    .unwrap();
    // In the extension's support directory, one of the two places the sandbox
    // lets it write. Not the tempdir: that is under /tmp, which it may not.
    let out = root.join("data-home/compass/support/hello/greeting.txt");
    let answered = root.join("data-home/compass/support/hello/answered.txt");
    let replaced = root.join("data-home/compass/support/hello/replaced.txt");
    std::fs::write(
        ext.join("ask.js"),
        format!(
            "const React = require('react');
             const {{ List, ActionPanel, Action, confirmAlert }} = require('@vicinae/api');
             module.exports.default = () => React.createElement(List, null,
               React.createElement(List.Item, {{ title: 'risky', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Delete', onAction: async () => {{
                     const ok = await confirmAlert({{ title: 'Delete it?' }});
                     require('node:fs').writeFileSync({answered:?}, String(ok));
                   }} }}))
               }}),
               React.createElement(List.Item, {{ title: 'twice', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Ask twice', onAction: async () => {{
                     const first = confirmAlert({{ title: 'First?' }});
                     confirmAlert({{ title: 'Second?' }});
                     require('node:fs').writeFileSync({replaced:?}, String(await first));
                   }} }}))
               }}));",
            answered = answered.to_string_lossy(),
            replaced = replaced.to_string_lossy()
        ),
    )
    .unwrap();
    let authorized = root.join("data-home/compass/support/hello/authorized.txt");
    std::fs::write(
        ext.join("auth.js"),
        format!(
            "const React = require('react');
             const {{ List, OAuth }} = require('@vicinae/api');
             module.exports.default = function Auth() {{
               React.useEffect(() => {{
                 const client = new OAuth.PKCEClient({{
                   redirectMethod: OAuth.RedirectMethod.Web,
                   providerName: 'Example', description: 'Connect your Example account' }});
                 client.authorizationRequest({{
                   endpoint: 'https://example.com/authorize', clientId: 'id', scope: 'read' }})
                   .then((request) => client.authorize(request))
                   .then(({{ authorizationCode }}) => authorizationCode, (e) => 'refused:' + e.message)
                   .then((out) => require('node:fs').writeFileSync({authorized:?}, out));
               }}, []);
               return React.createElement(List, null, React.createElement(List.Item, {{ title: 'waiting' }}));
             }};",
            authorized = authorized.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("nav.js"),
        "const React = require('react');
         const { List, Detail, ActionPanel, Action, useNavigation } = require('@vicinae/api');
         module.exports.default = function Root() {
           const { push } = useNavigation();
           return React.createElement(List, null,
             React.createElement(List.Item, { title: 'go', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Push', onAction: () =>
                   push(React.createElement(Detail, { markdown: 'pushed' })) }))
             }));
         };",
    )
    .unwrap();
    let acted = root.join("data-home/compass/support/hello/acted.txt");
    std::fs::write(
        ext.join("show.js"),
        format!(
            "const React = require('react');
             const {{ List, ActionPanel, Action }} = require('@vicinae/api');
             module.exports.default = () => React.createElement(List, {{ navigationTitle: 'Greetings' }},
               React.createElement(List.Item, {{ title: 'hello', id: 'h', actions:
                 React.createElement(ActionPanel, null,
                   React.createElement(Action, {{ title: 'Act', onAction: () =>
                     require('node:fs').writeFileSync({acted:?}, 'acted') }}))
               }}));",
            acted = acted.to_string_lossy()
        ),
    )
    .unwrap();
    let submitted = root.join("data-home/compass/support/hello/submitted.json");
    std::fs::write(
        ext.join("issue.js"),
        format!(
            "const React = require('react');
             const {{ Form, ActionPanel, Action }} = require('@vicinae/api');
             module.exports.default = function Issue() {{
               const [title, setTitle] = React.useState('');
               return React.createElement(Form, {{ actions:
                   React.createElement(ActionPanel, null,
                     React.createElement(Action.SubmitForm, {{ title: 'Create', onSubmit: (values) =>
                       require('node:fs').writeFileSync({submitted:?},
                         JSON.stringify({{ values, title }})) }}))
                 }},
                 React.createElement(Form.TextField, {{ id: 'title', title: 'Title',
                   value: title, onChange: setTitle }}),
                 React.createElement(Form.Checkbox, {{ id: 'urgent', label: 'Urgent',
                   defaultValue: false }}));
             }};",
            submitted = submitted.to_string_lossy()
        ),
    )
    .unwrap();
    let probed = root.join("data-home/compass/support/hello/probe.json");
    std::fs::write(
        ext.join("probe.js"),
        format!(
            "module.exports.default = async () => {{
               const {{ environment, getPreferenceValues }} = require('@vicinae/api');
               const {{ spawnSync }} = require('node:child_process');
               const fs = require('node:fs');
               const status = (cmd, args) => {{
                 const r = spawnSync(cmd, args);
                 return r.error ? String(r.error.code) : r.status;
               }};
               // A program the command writes itself, into the one place it
               // may write, and then tries to run.
               const dropped = environment.supportPath + '/dropped';
               const original = ['/usr/bin/true', '/bin/true'].find((p) => fs.existsSync(p));
               fs.copyFileSync(original, dropped);
               fs.chmodSync(dropped, 0o755);
               // Outside the JavaScript heap, which the heap cap cannot see.
               let bigBuffer;
               try {{ bigBuffer = Buffer.alloc(512 * 1024 * 1024).length; }}
               catch (e) {{ bigBuffer = e.name; }}
               fs.writeFileSync({probed:?}, JSON.stringify({{
                 execDropped: status(dropped, []),
                 bigBuffer,
                 assetsPath: environment.assetsPath,
                 supportPath: environment.supportPath,
                 isDevelopment: environment.isDevelopment,
                 extensionName: environment.extensionName,
                 commandName: environment.commandName,
                 preferences: getPreferenceValues(),
                 execTrue: status('true', []),
                 execUnshare: status('unshare', ['-U', 'true']),
               }}));
             }};",
            probed = probed.to_string_lossy()
        ),
    )
    .unwrap();
    // `heap_size_limit` would be the obvious probe, and it lies in a worker
    // thread (it reports the process figure), so this allocates instead.
    let heap = root.join("data-home/compass/support/hello/heap");
    std::fs::write(
        ext.join("heap.js"),
        format!(
            "module.exports.default = async () => {{
               const fs = require('node:fs');
               fs.writeFileSync({started:?}, 'started');
               const keep = [];
               for (let i = 0; i < 400; i++) keep.push(new Array(128 * 1024).fill(i + 0.5));
               fs.writeFileSync({survived:?}, String(keep.length));
             }};",
            started = heap.with_extension("started").to_string_lossy(),
            survived = heap.with_extension("survived").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("toaster.js"),
        "const React = require('react');
         const { List, ActionPanel, Action, showToast, Toast } = require('@vicinae/api');
         let toast;
         module.exports.default = function Toaster() {
           React.useEffect(() => {
             showToast({ style: Toast.Style.Failure, title: 'Offline', message: 'retrying' })
               .then((t) => { toast = t; });
           }, []);
           return React.createElement(List, null,
             React.createElement(List.Item, { title: 'hide', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Hide', onAction: () => toast.hide() }))
             }));
         };",
    )
    .unwrap();
    // Asks the host what a command can learn about the desktop, and shows the
    // answers (or the rejections) in one line.
    std::fs::write(
        ext.join("machine.js"),
        "const React = require('react');
         const { Detail, getSelectedText, WindowManagement } = require('@vicinae/api');
         const said = (p) => p.then((v) => JSON.stringify(v),
               (e) => e instanceof Error ? 'error: ' + e.message : 'not an Error: ' + String(e));
         module.exports.default = () => {
           const [text, setText] = React.useState('');
           React.useEffect(() => {
             Promise.all([
               said(getSelectedText()),
               said(WindowManagement.getActiveWindow()),
               said(WindowManagement.getWindows()),
               said(WindowManagement.getScreens()),
               said(WindowManagement.getActiveWorkspace()),
             ]).then((answers) => setText(answers.join(' | ')));
           }, []);
           return React.createElement(Detail, { markdown: text || 'asking' });
         };",
    )
    .unwrap();
    std::fs::write(
        ext.join("tiles.js"),
        "const React = require('react');
         const { Grid, ActionPanel, Action } = require('@vicinae/api');
         module.exports.default = () => React.createElement(Grid, { columns: 4 },
           React.createElement(Grid.Section, { title: 'Weather' },
             React.createElement(Grid.Item, { title: 'sun', content: 'sun-16', actions:
               React.createElement(ActionPanel, null,
                 React.createElement(Action, { title: 'Pick', onAction: () => {} }))
             }),
             React.createElement(Grid.Item, { title: 'red', content: { color: '#ff0000' } })));",
    )
    .unwrap();
    std::fs::write(
        ext.join("term.js"),
        "const { runInTerminal } = require('@vicinae/api');
         module.exports.default = async () => {
           await runInTerminal(['htop', '-d', '5'], { title: 'Top' });
         };",
    )
    .unwrap();
    std::fs::write(
        ext.join("link.js"),
        "const { open } = require('@vicinae/api');
         module.exports.default = async () => { await open('https://example.com/a b'); };",
    )
    .unwrap();
    let greeted = root.join("data-home/compass/support/hello/greeted.txt");
    std::fs::write(
        ext.join("greet.js"),
        format!(
            "module.exports.default = async (props) => {{
               require('node:fs').writeFileSync({greeted:?}, 'hi ' + props.arguments.name);
             }};",
            greeted = greeted.to_string_lossy()
        ),
    )
    .unwrap();
    // Outside every path the sandbox grants: the data home itself, beside the
    // support directory the command may write.
    let escape = root.join("data-home/escape.txt");
    std::fs::write(
        ext.join("write.js"),
        format!(
            "module.exports.default = async () => {{
               const fs = require('node:fs');
               let escaped = 'wrote';
               try {{ fs.writeFileSync({escape:?}, 'x'); }} catch (e) {{ escaped = e.code; }}
               fs.writeFileSync({out:?}, 'hi:' + escaped);
             }};",
            escape = escape.to_string_lossy(),
            out = out.to_string_lossy()
        ),
    )
    .unwrap();
    out
}

/// Waits up to 20 s for `path` to exist with something in it: an extension's
/// `writeFileSync` creates the file before it writes, so existing is not enough.
fn wait_for_content(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !std::fs::metadata(path).is_ok_and(|m| m.len() > 0) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn extension_runtime() -> Option<std::path::PathBuf> {
    let built = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/typescript/extension-manager/dist/runtime.js");
    std::env::var_os("COMPASS_EXTENSION_RUNTIME")
        .map(Into::into)
        .or_else(|| built.is_file().then_some(built))
}

#[test]
fn an_installed_extension_command_is_found_and_a_no_view_one_runs() {
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut out = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        out = install_extension(root);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });

    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "write greeting".into(),
    }) else {
        panic!("no results");
    };
    let hit = hits.first().expect("a hit");
    assert_eq!(hit.id, "@someone/hello:write");
    assert_eq!(hit.subtitle.as_deref(), Some("Hello"));

    let started = daemon.request(Request::RunExtensionCommand {
        id: hit.id.clone(),
        arguments_json: None,
    });
    assert_eq!(started, Response::Ack, "{started:?}");
    wait_for_content(&out);
    assert_eq!(
        std::fs::read_to_string(&out).ok().as_deref(),
        Some("hi:EACCES"),
        "the command ran, confined: a write outside its directories is refused"
    );

    let Response::Error(err) = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:nothing".into(),
        arguments_json: None,
    }) else {
        panic!("an unknown id was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn a_view_command_renders_and_its_action_runs() {
    use compass_extension_api::View;
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });

    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:show".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("the view command did not start a session: {started:?}");
    };

    let mut after = 0;
    let view = loop {
        let answer = daemon.request(Request::ExtensionView { session, after });
        let Response::ExtensionView {
            version,
            view_json,
            problem,
            ended,
            ..
        } = answer
        else {
            panic!("no view answer: {answer:?}");
        };
        assert!(!ended, "the command ended: {problem:?}");
        assert_eq!(problem, None);
        if let Some(json) = view_json {
            break serde_json::from_str::<View>(&json).expect("a View");
        }
        after = version;
    };
    let View::List(list) = view else {
        panic!("not a list: {view:?}");
    };
    assert_eq!(list.navigation_title.as_deref(), Some("Greetings"));
    let item = &list.sections[0].items[0];
    assert_eq!(item.title, "hello");
    let action = item.actions.as_ref().expect("actions").actions()[0].clone();
    assert_eq!(action.title, "Act");

    let acted = root.join("data-home/compass/support/hello/acted.txt");
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: action.handler.0.clone(),
            args_json: "[]".into(),
        }),
        Response::Ack
    );
    wait_for_content(&acted);
    assert_eq!(
        std::fs::read_to_string(&acted).ok().as_deref(),
        Some("acted"),
        "the action's onAction ran in the extension"
    );

    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
    let Response::Error(err) = daemon.request(Request::ExtensionView { session, after: 0 }) else {
        panic!("a closed session still answers");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

/// Polls `session` until `done` accepts its view, returning it and the depth.
fn wait_for_view(
    daemon: &Daemon,
    session: u64,
    done: impl Fn(&compass_extension_api::View, u32) -> bool,
) -> (compass_extension_api::View, u32) {
    use compass_ipc::{Request, Response};
    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let answer = daemon.request(Request::ExtensionView { session, after });
        let Response::ExtensionView {
            version,
            view_json,
            problem,
            ended,
            depth,
            ..
        } = answer
        else {
            panic!("no view answer: {answer:?}");
        };
        assert!(!ended && problem.is_none(), "ended: {problem:?}");
        if let Some(view) = view_json.and_then(|json| serde_json::from_str(&json).ok())
            && done(&view, depth)
        {
            return (view, depth);
        }
        after = version;
    }
    panic!("the view never got there");
}

#[test]
fn without_a_desktop_the_selection_and_window_apis_answer_as_the_cpp_does() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:machine".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    // No session bus and no Wayland display: no Shell extension, no
    // compositor. Every call is answered, none of them "not implemented".
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Detail(detail) if detail.markdown.as_deref() != Some("asking")),
    );
    let View::Detail(detail) = view else {
        unreachable!()
    };
    assert_eq!(
        detail.markdown.as_deref(),
        Some(
            "error: Unable to get selected text | error: No active window | [] | [] \
             | error: No active workspace"
        )
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn a_pushed_view_shows_and_escape_pops_back_to_the_list() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:nav".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };

    let (root, depth) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    assert_eq!(depth, 1);
    let View::List(list) = root else {
        unreachable!()
    };
    let push = list.sections[0].items[0]
        .actions
        .as_ref()
        .expect("actions")
        .actions()[0]
        .handler
        .0
        .clone();

    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: push,
            args_json: "[]".into(),
        }),
        Response::Ack
    );
    let (pushed, depth) =
        wait_for_view(&daemon, session, |view, _| matches!(view, View::Detail(_)));
    assert_eq!(depth, 2, "the pushed view is on top of the list");
    let View::Detail(detail) = pushed else {
        unreachable!()
    };
    assert_eq!(detail.markdown.as_deref(), Some("pushed"));

    assert_eq!(
        daemon.request(Request::ExtensionPop { session }),
        Response::Ack
    );
    let (_, depth) = wait_for_view(&daemon, session, |view, depth| {
        matches!(view, View::List(_)) && depth == 1
    });
    assert_eq!(depth, 1, "popped back to the list");
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn an_alert_reaches_the_launcher_and_its_answer_reaches_the_extension() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:ask".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    let (view, _) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    let View::List(list) = view else {
        unreachable!()
    };
    let delete = list.sections[0].items[0]
        .actions
        .as_ref()
        .expect("actions")
        .actions()[0]
        .handler
        .0
        .clone();
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: delete,
            args_json: "[]".into(),
        }),
        Response::Ack
    );

    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    let alert = loop {
        assert!(Instant::now() < deadline, "no alert arrived");
        let Response::ExtensionView { version, alert, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        if let Some(alert) = alert {
            break alert;
        }
        after = version;
    };
    assert_eq!(alert.title, "Delete it?");

    assert_eq!(
        daemon.request(Request::ExtensionAlertAnswer {
            session,
            confirmed: true
        }),
        Response::Ack
    );
    let answered = root.join("data-home/compass/support/hello/answered.txt");
    wait_for_content(&answered);
    assert_eq!(
        std::fs::read_to_string(&answered).ok().as_deref(),
        Some("true"),
        "the extension's confirmAlert resolved with the person's answer"
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

/// Waits for `session` to show an alert titled `title`.
fn wait_for_alert(daemon: &Daemon, session: u64, title: &str) {
    use compass_ipc::{Request, Response};
    let mut after = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "no alert {title:?} arrived");
        let Response::ExtensionView { version, alert, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        if alert.is_some_and(|alert| alert.title == title) {
            return;
        }
        after = version;
    }
}

#[test]
fn a_replaced_alert_and_one_navigated_away_from_both_answer_no() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:ask".into(),
        arguments_json: None,
    }) else {
        panic!("no session");
    };
    let (view, _) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    let View::List(list) = view else {
        unreachable!()
    };
    let handler = |row: usize| {
        list.sections[0].items[row]
            .actions
            .as_ref()
            .expect("actions")
            .actions()[0]
            .handler
            .0
            .clone()
    };
    let support = root.join("data-home/compass/support/hello");

    // A second alert while the first is open: the first is cancelled, and
    // the extension is told so rather than left waiting.
    daemon.request(Request::ExtensionEvent {
        session,
        handler: handler(1),
        args_json: "[]".into(),
    });
    wait_for_alert(&daemon, session, "Second?");
    let replaced = support.join("replaced.txt");
    wait_for_content(&replaced);
    assert_eq!(
        std::fs::read_to_string(&replaced).ok().as_deref(),
        Some("false"),
        "the replaced alert's promise settled with no"
    );
    assert_eq!(
        daemon.request(Request::ExtensionAlertAnswer {
            session,
            confirmed: false
        }),
        Response::Ack
    );

    // Leaving the view while an alert is open answers it no.
    daemon.request(Request::ExtensionEvent {
        session,
        handler: handler(0),
        args_json: "[]".into(),
    });
    wait_for_alert(&daemon, session, "Delete it?");
    assert_eq!(
        daemon.request(Request::ExtensionPop { session }),
        Response::Ack
    );
    let answered = support.join("answered.txt");
    wait_for_content(&answered);
    assert_eq!(
        std::fs::read_to_string(&answered).ok().as_deref(),
        Some("false"),
        "navigating away is a way out of the dialog, and it is not the confirm button"
    );
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn a_required_preference_without_a_keyring_is_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // These engines run with no session bus, so no keyring: there is nowhere
    // safe to keep the token, and the engine says so rather than asking.
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let answer = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:needs".into(),
        arguments_json: None,
    });
    let Response::Error(err) = answer else {
        panic!("not refused: {answer:?}");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(
        err.message.contains("API Token") && err.message.contains("keyring"),
        "{}",
        err.message
    );
}

#[test]
fn a_command_with_arguments_is_asked_for_them_then_runs_with_them() {
    use compass_ipc::{PreferenceFieldKind, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut greeted = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        greeted = dir.join("data-home/compass/support/hello/greeted.txt");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let run = |arguments_json: Option<&str>| {
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:greet".into(),
            arguments_json: arguments_json.map(str::to_owned),
        })
    };

    let Response::ExtensionNeedsArguments { title, fields } = run(None) else {
        panic!("not asked for its arguments");
    };
    assert_eq!(title, "Greet Someone");
    assert_eq!(fields.len(), 1);
    assert_eq!(
        (fields[0].name.as_str(), fields[0].title.as_str()),
        ("name", "Name")
    );
    assert!(fields[0].required && fields[0].kind == PreferenceFieldKind::Text);

    assert!(
        matches!(
            run(Some(r#"{"name": ""}"#)),
            Response::ExtensionNeedsArguments { .. }
        ),
        "an empty required argument is asked for again"
    );
    assert!(!greeted.exists(), "and nothing ran");

    assert_eq!(run(Some(r#"{"name": "Ada"}"#)), Response::Ack);
    wait_for_content(&greeted);
    assert_eq!(
        std::fs::read_to_string(&greeted).expect("the command ran"),
        "hi Ada"
    );
}

#[test]
fn a_confined_command_opens_a_link_in_the_application_that_claims_its_scheme() {
    use compass_ipc::{Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // The opener is spawned by the engine, not the extension, so it may write
    // where the sandbox would never let the extension.
    let mut opened = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        opened = dir.join("opened.txt");
        let script = dir.join("browser.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > {:?}\n",
                opened.to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("browser.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Browser\nExec={} %u\n\
                 MimeType=x-scheme-handler/https;\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:link".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&opened);
    assert_eq!(
        std::fs::read_to_string(&opened).expect("the link was opened"),
        "https://example.com/a b"
    );
}

#[test]
fn a_grid_command_renders_typed_cells() {
    use compass_extension_api::View;
    use compass_extension_api::view::{Color, GridContent};
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:tiles".into(),
        arguments_json: None,
    }) else {
        panic!("the grid command did not start a view");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Grid(grid) if !grid.sections.is_empty()),
    );
    let View::Grid(grid) = view else {
        unreachable!()
    };
    assert_eq!(grid.columns, Some(4));
    let section = &grid.sections[0];
    assert_eq!(section.title.as_deref(), Some("Weather"));
    let titles: Vec<&str> = section.items.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["sun", "red"]);
    assert!(section.items[0].actions.is_some());
    assert_eq!(
        section.items[1].content,
        GridContent::Color(Color::Literal("#ff0000".into()))
    );
    daemon.request(Request::CloseExtension { session });
}

/// The launcher's own client against a form left alone for longer than the
/// two seconds it used to allow a view read. The engine holds a view read
/// until something changes, so an idle form is answered only when the hold
/// runs out; with a client deadline shorter than the hold, the launcher took
/// that silence for a failure, stopped polling, and every later keystroke
/// and action went nowhere.
///
/// The other view tests here never saw it: they ask over a bare
/// `compass_ipc::Client`, with no deadline at all, as Suite 1 and the
/// conformance runner do with theirs. This one goes through
/// `compass::ui_backend::DaemonBackend`, the client the window uses. The
/// engine's hold is shortened to three seconds so the test does not wait
/// twenty, and still outlasts the old deadline.
#[test]
fn a_form_left_idle_past_the_old_deadline_still_takes_typing_and_submit() {
    use compass_extension_api::View;
    use compass_extension_api::view::{FieldValue, FormItem};
    use compass_ui::backend::{ApplicationBackend, ExtensionStart};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    const HOLD: Duration = Duration::from_secs(3);
    let mut submitted = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        submitted = dir.join("data-home/compass/support/hello/submitted.json");
        vec![
            ("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string()),
            (
                compass::serve::VIEW_HOLD_ENV,
                HOLD.as_millis().to_string().into(),
            ),
        ]
    });
    let backend =
        compass::ui_backend::DaemonBackend::new(compass_ipc::SocketPath::exact(&daemon.socket));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let ExtensionStart::View(session) = backend
            .run_extension_command("@someone/hello:issue".into(), None)
            .await
            .expect("the form command starts")
        else {
            panic!("the form command did not start a view");
        };
        let mut state = backend.extension_view(session, 0).await.expect("a first read");
        while !matches!(state.view.as_deref(), Some(View::Form(form)) if !form.items.is_empty()) {
            state = backend
                .extension_view(session, state.version)
                .await
                .expect("the form renders");
        }
        let Some(View::Form(form)) = state.view.as_deref() else {
            unreachable!()
        };
        let FormItem::Field(title) = &form.items[0] else {
            panic!("no title field");
        };
        let on_change = title.on_change.clone().expect("a controlled field");
        let submit = form.actions.as_ref().expect("actions").actions()[0]
            .handler
            .clone();

        // Nothing happens for longer than the old deadline.
        let asked = Instant::now();
        let idle = backend
            .extension_view(session, state.version)
            .await
            .expect("an idle form is not a failure");
        assert!(
            asked.elapsed() >= Duration::from_millis(2500),
            "the engine held the read for its hold, {:?}",
            asked.elapsed()
        );
        assert_eq!(idle.version, state.version, "nothing changed");
        assert!(!idle.ended && idle.problem.is_none());

        // The person types, and the form takes it.
        backend
            .extension_event(session, on_change.0, vec!["Crash on paste".into(), 1.into()])
            .await
            .expect("typing reaches the extension");
        let mut after = idle.version;
        loop {
            let next = backend
                .extension_view(session, after)
                .await
                .expect("the edit is read back");
            if let Some(View::Form(form)) = next.view.as_deref()
                && matches!(&form.items[0],
                    FormItem::Field(f) if f.value == Some(FieldValue::Text("Crash on paste".into())))
            {
                break;
            }
            after = next.version;
        }

        backend
            .extension_event(
                session,
                submit.0,
                vec![serde_json::json!({"title": "Crash on paste", "urgent": false})],
            )
            .await
            .expect("submit reaches the extension");
        backend.close_extension(session).await.expect("closes");
    });
    wait_for_content(&submitted);
    let got: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&submitted).expect("submitted")).unwrap();
    assert_eq!(got["title"], "Crash on paste");
}

#[test]
fn a_form_command_takes_edits_and_its_submit_gets_the_values() {
    use compass_extension_api::View;
    use compass_extension_api::view::{FieldValue, FormItem};
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut submitted = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        submitted = dir.join("data-home/compass/support/hello/submitted.json");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:issue".into(),
        arguments_json: None,
    }) else {
        panic!("the form command did not start a view");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Form(form) if !form.items.is_empty()),
    );
    let View::Form(form) = view else {
        unreachable!()
    };
    let FormItem::Field(title) = &form.items[0] else {
        panic!("no title field");
    };
    let on_change = title.on_change.clone().expect("a controlled field");
    let submit = form.actions.as_ref().expect("actions").actions()[0]
        .handler
        .clone();

    // The person types; the extension's state takes it and echoes it back.
    let answer = daemon.request(Request::ExtensionEvent {
        session,
        handler: on_change.0,
        args_json: r#"["Crash on paste", 1]"#.into(),
    });
    assert_eq!(answer, Response::Ack);
    let (echoed, _) = wait_for_view(&daemon, session, |view, _| match view {
        View::Form(form) => matches!(&form.items[0],
            FormItem::Field(f) if f.value == Some(FieldValue::Text("Crash on paste".into()))),
        _ => false,
    });
    let View::Form(echoed) = echoed else {
        unreachable!()
    };
    let FormItem::Field(title) = &echoed.items[0] else {
        unreachable!()
    };
    assert_eq!(
        title.echo.map(|seq| seq.raw()),
        Some(1),
        "an echo of edit 1"
    );

    daemon.request(Request::ExtensionEvent {
        session,
        handler: submit.0,
        args_json: r#"[{"title": "Crash on paste", "urgent": true}]"#.into(),
    });
    wait_for_content(&submitted);
    let got: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&submitted).expect("submitted")).unwrap();
    assert_eq!(
        got,
        serde_json::json!({
            "values": {"title": "Crash on paste", "urgent": true},
            "title": "Crash on paste"
        })
    );
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn an_extension_that_allocates_past_the_heap_cap_is_stopped() {
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut heap = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        heap = dir.join("data-home/compass/support/hello/heap");
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:heap".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    let (started, survived) = (
        heap.with_extension("started"),
        heap.with_extension("survived"),
    );
    wait_for_content(&started);
    assert!(started.exists(), "the command never ran");
    // 400 MiB takes well under a second to allocate; give it five.
    std::thread::sleep(Duration::from_secs(5));
    assert!(
        !survived.exists(),
        "an extension allocated 400 MiB of heap under a 160 MiB cap"
    );
}

/// Suite 1: what a command sees of its environment, and what the sandbox
/// lets it run.
#[test]
fn a_command_sees_its_own_paths_and_preferences_and_may_exec_but_not_unshare() {
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_owned();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let probed = root.join("data-home/compass/support/hello/probe.json");
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:probe".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&probed);
    let seen: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&probed).expect("the command ran")).unwrap();
    let data = root.join("data-home/compass");
    assert_eq!(
        seen["supportPath"],
        data.join("support/hello").to_string_lossy().as_ref()
    );
    assert_eq!(
        seen["assetsPath"],
        data.join("extensions/hello/assets")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(seen["isDevelopment"], false);
    assert_eq!(
        (&seen["extensionName"], &seen["commandName"]),
        (&"hello".into(), &"probe".into())
    );
    assert_eq!(
        seen["preferences"],
        serde_json::json!({"limit": "20"}),
        "a preference's default is resolved"
    );
    // Shelling out is allowed on purpose: `useExec` and every extension that
    // wraps a CLI depend on it.
    assert_eq!(seen["execTrue"], 0);
    // `unshare` is on the denylist. The control: where this machine lets an
    // unconfined process make a user namespace, the sandbox must not.
    let outside = std::process::Command::new("unshare")
        .args(["-U", "true"])
        .status()
        .is_ok_and(|status| status.success());
    if outside {
        assert_ne!(
            seen["execUnshare"], 0,
            "unshare -U works outside the sandbox and must not inside it"
        );
    } else {
        eprintln!("unshare -U fails here even unconfined; the denial is not tested");
    }
    // Suite 1's `fork` case (§8.2): running what it wrote itself is the
    // escape, and it fails closed, while `true` from the system ran above.
    assert_eq!(
        seen["execDropped"], "EACCES",
        "a program the command wrote into its support directory ran"
    );
    // The 512 MB case: a Buffer lives outside the heap cap, and the data
    // limit refuses it as an error the command can catch rather than a crash.
    assert_eq!(
        seen["bigBuffer"], "RangeError",
        "a 512 MiB Buffer was allocated inside the sandbox"
    );
}

#[test]
fn a_confined_command_runs_a_command_in_the_terminal() {
    use compass_ipc::{Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut launched = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        launched = dir.join("terminal-argv.txt");
        let script = dir.join("term.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" > {:?}\n",
                launched.to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("term.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Term\nExec={}\n\
                 Categories=System;TerminalEmulator;\n\
                 X-TerminalArgExec=--\nX-TerminalArgTitle=--title=\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    assert_eq!(
        daemon.request(Request::RunExtensionCommand {
            id: "@someone/hello:term".into(),
            arguments_json: None,
        }),
        Response::Ack
    );
    wait_for_content(&launched);
    assert_eq!(
        std::fs::read_to_string(&launched).expect("the terminal was launched"),
        "--title=Top|--|htop|-d|5|"
    );
}

#[test]
fn a_views_toast_reaches_the_launcher_and_hiding_it_clears_it() {
    use compass_extension_api::View;
    use compass_ipc::{ExtensionToastStyle, Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:toaster".into(),
        arguments_json: None,
    }) else {
        panic!("the view did not start");
    };
    // Follow the raw answers: the toast is not part of the view.
    let mut after = 0;
    let mut hide = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    let toast = loop {
        assert!(Instant::now() < deadline, "no toast arrived");
        let Response::ExtensionView {
            version,
            view_json,
            toast,
            ..
        } = daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        after = version;
        if let Some(Ok(View::List(list))) =
            view_json.map(|json| serde_json::from_str::<View>(&json))
        {
            hide = list.sections[0].items[0]
                .actions
                .as_ref()
                .map(|panel| panel.actions()[0].handler.clone());
        }
        if let (Some(toast), Some(_)) = (toast, &hide) {
            break toast;
        }
    };
    assert_eq!(
        (toast.title.as_str(), toast.message.as_str(), toast.style),
        ("Offline", "retrying", ExtensionToastStyle::Failure)
    );

    let hide = hide.expect("the list's action");
    daemon.request(Request::ExtensionEvent {
        session,
        handler: hide.0,
        args_json: "[]".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "the toast was never hidden");
        let Response::ExtensionView { version, toast, .. } =
            daemon.request(Request::ExtensionView { session, after })
        else {
            panic!("no view answer");
        };
        after = version;
        if toast.is_none() {
            break;
        }
    }
    daemon.request(Request::CloseExtension { session });
}

#[test]
fn a_power_command_answers_with_its_own_sentences_and_never_touches_this_machine() {
    use compass_ipc::{ErrorKind, Request, Response};
    // A system bus that does not exist: whatever this engine tries, the
    // machine running the test cannot be rebooted by it.
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![(
            "DBUS_SYSTEM_BUS_ADDRESS",
            "unix:path=/nonexistent/compass-test-system-bus".into(),
        )]
    });
    let Response::Error(err) = daemon.request(Request::RunPowerCommand {
        id: "reboot".into(),
    }) else {
        panic!("a reboot with no logind was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to reboot")
    );
    let Response::Error(err) = daemon.request(Request::RunPowerCommand {
        id: "self-destruct".into(),
    }) else {
        panic!("an unknown power command was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn a_power_command_with_a_custom_program_runs_it_instead() {
    use compass_ipc::{ErrorKind, Request, Response};
    let config = r#"{"providers": {"power": {"entrypoints": {
        "reboot": {"preferences": {"customProgram": "touch \"$HOME/rebooted\""}},
        "suspend": {"preferences": {"customProgram": ""}}
    }}}}"#;
    let mut home = None;
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], config, |root| {
        home = Some(root.to_path_buf());
        vec![
            (
                "DBUS_SYSTEM_BUS_ADDRESS",
                "unix:path=/nonexistent/compass-test-system-bus".into(),
            ),
            ("SHELL", "/bin/sh".into()),
        ]
    });
    let home = home.expect("the daemon's home");
    assert_eq!(
        daemon.request(Request::RunPowerCommand {
            id: "reboot".into()
        }),
        Response::Ack
    );
    assert!(
        home.join("rebooted").exists(),
        "the program ran in place of logind"
    );
    // An empty program is no program: the built-in call is made, and with no
    // system bus it fails with the command's own sentence.
    let Response::Error(err) = daemon.request(Request::RunPowerCommand {
        id: "suspend".into(),
    }) else {
        panic!("a suspend with no logind was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Internal);
    assert_eq!(err.message, "Failed to suspend");
}

#[test]
fn a_media_command_says_why_it_did_nothing() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::io::{BufRead, BufReader};
    let play_pause = || Request::RunMediaCommand {
        id: "play-pause".into(),
    };

    // No session bus at all: the player could not be reached.
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let Response::Error(err) = daemon.request(play_pause()) else {
        panic!("play/pause with no session bus was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to toggle playback")
    );
    let Response::Error(err) = daemon.request(Request::RunMediaCommand {
        id: "rewind-time".into(),
    }) else {
        panic!("an unknown media command was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    drop(daemon);

    // A bus with no player on it: the C++'s own sentence.
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED the empty-bus half: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });
    let answer = daemon.request(play_pause());
    let _ = bus.kill();
    let _ = bus.wait();
    let Response::Error(err) = answer else {
        panic!("play/pause with no player was not refused: {answer:?}");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Unsupported, "No media player is running")
    );
}

#[test]
fn a_volume_command_runs_pactl_with_the_cpp_arguments() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let bin = TempDir::new().expect("tempdir");
    let log = bin.path().join("pactl.log");
    let fake = bin.path().join("pactl");
    std::fs::write(
        &fake,
        format!(
            r#"#!/bin/sh
echo "$*" >> '{log}'
[ -e '{fail}' ] && exit 1
case "$*" in
  get-default-sink) echo sink0 ;;
  "--format=json list sinks") echo '[{{"name":"sink0","mute":false,"volume":{{"mono":{{"value_percent":"45%"}}}}}}]' ;;
esac
exit 0
"#,
            log = log.display(),
            fail = bin.path().join("fail").display(),
        ),
    )
    .expect("fake pactl");
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH");
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("PATH", path)]
    });
    let run = |id: &str| daemon.request(Request::RunMediaCommand { id: id.to_owned() });

    assert!(matches!(run("volume-50"), Response::Ack));
    assert!(matches!(run("volume-up"), Response::Ack));
    assert!(matches!(run("toggle-mute"), Response::Ack));
    let calls = std::fs::read_to_string(&log).expect("pactl ran");
    let calls: Vec<&str> = calls.lines().collect();
    assert_eq!(
        calls,
        [
            "set-sink-volume @DEFAULT_SINK@ 50%",
            "set-sink-volume @DEFAULT_SINK@ +5%",
            "get-default-sink",
            "--format=json list sinks",
            "set-sink-mute @DEFAULT_SINK@ toggle",
            "get-default-sink",
            "--format=json list sinks",
            "get-default-sink",
            "--format=json list sinks",
        ]
    );

    std::fs::write(bin.path().join("fail"), "").expect("make pactl fail");
    let Response::Error(err) = run("volume-0") else {
        panic!("a failing pactl was not reported");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::Internal, "Failed to set volume")
    );

    // The step argument: a number moves by that much, anything else is
    // refused before pactl runs.
    std::fs::remove_file(bin.path().join("fail")).expect("let pactl succeed");
    std::fs::write(&log, "").expect("clear the log");
    let with = |id: &str, argument: &str| {
        daemon.request(Request::RunMediaCommandWith {
            id: id.to_owned(),
            argument: Some(argument.to_owned()),
        })
    };
    assert!(matches!(with("volume-down", "-12"), Response::Ack));
    let Response::Error(err) = with("volume-up", "five") else {
        panic!("a step that is not a number was not refused");
    };
    assert_eq!(err.message, "Invalid step value");
    let calls = std::fs::read_to_string(&log).expect("pactl ran");
    assert_eq!(
        calls.lines().next(),
        Some("set-sink-volume @DEFAULT_SINK@ -12%")
    );
}

#[test]
fn an_extensions_deeplink_goes_to_the_window_and_a_malformed_one_is_refused() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let open = |url: &str| {
        daemon.request(Request::OpenDeeplink {
            url: url.to_owned(),
        })
    };
    let Response::Error(err) = open("vicinae://extensions/only-one") else {
        panic!("a link with one segment was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (
            ErrorKind::BadRequest,
            compass_core::store_listing::EXTENSION_LINK_USAGE
        )
    );
    let Response::Error(err) = open("vicinae://theme/set/x") else {
        panic!("a deeplink the launcher does not handle was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    // A good link goes to the window, and there is none attached here.
    let Response::Error(err) = open("raycast://extensions/thomas/spotify-player") else {
        panic!("a deeplink with no window to show it was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(
        err.message.contains("no launcher window"),
        "{}",
        err.message
    );
}

/// A file manager that records what it was asked to show.
struct FakeFileManager {
    shown: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FakeFileManager {
    fn show_items(&self, uris: Vec<String>, _startup_id: String) {
        self.shown.lock().expect("log").extend(uris);
    }
}

#[test]
fn show_in_file_browser_asks_file_manager1_to_select_the_file() {
    use compass_ipc::{Request, Response};
    use std::io::{BufRead, BufReader};
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();
    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the file manager");
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _manager: zbus::Connection = runtime.block_on(async {
        zbus::connection::Builder::address(address.as_str())
            .expect("the private bus")
            .name("org.freedesktop.FileManager1")
            .expect("the name")
            .serve_at(
                "/org/freedesktop/FileManager1",
                FakeFileManager {
                    shown: std::sync::Arc::clone(&shown),
                },
            )
            .expect("the interface")
            .build()
            .await
            .expect("the file manager connects")
    });
    let file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let path = root.join("My Notes.txt");
        std::fs::write(&path, "x").unwrap();
        file.set(path).unwrap();
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });
    assert_eq!(
        daemon.request(Request::OpenFile {
            path: file.get().unwrap().to_string_lossy().into_owned(),
            reveal: true,
        }),
        Response::Ack
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while shown.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(daemon);
    let _ = bus.kill();
    let _ = bus.wait();
    let shown = shown.lock().unwrap();
    assert_eq!(shown.len(), 1, "ShowItems was called once");
    assert!(shown[0].starts_with("file:///"), "{shown:?}");
    assert!(shown[0].ends_with("/My%20Notes.txt"), "{shown:?}");
}

/// A mock MPRIS player for the engine's media tests.
struct FakePlayer {
    status: &'static str,
    title: &'static str,
    can_go_next: bool,
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl FakePlayer {
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status.to_owned()
    }

    #[zbus(property)]
    fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "xesam:title".to_owned(),
            zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from(self.title))
                .expect("a value"),
        );
        metadata
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(name = "PlayPause")]
    fn play_pause(&self) {
        self.calls.lock().expect("log").push("PlayPause".to_owned());
    }

    #[zbus(name = "Next")]
    fn next(&self) {
        self.calls.lock().expect("log").push("Next".to_owned());
    }

    #[zbus(name = "Previous")]
    fn previous(&self) {
        self.calls.lock().expect("log").push("Previous".to_owned());
    }
}

struct FakePlayerRoot {
    identity: &'static str,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl FakePlayerRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.identity.to_owned()
    }
}

#[test]
fn a_player_argument_picks_the_player_and_now_playing_lists_and_drives_them() {
    use compass_ipc::{ErrorKind, MediaPlayerAction, Request, Response};
    use std::io::{BufRead, BufReader};
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();

    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the players");
    let spotify = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let firefox = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _players: Vec<zbus::Connection> = runtime.block_on(async {
        let mut connections = Vec::new();
        for (name, identity, status, title, can_go_next, calls) in [
            (
                "org.mpris.MediaPlayer2.spotify",
                "Spotify",
                "Playing",
                "Blue Monday",
                true,
                &spotify,
            ),
            (
                "org.mpris.MediaPlayer2.firefox",
                "Firefox",
                "Paused",
                "A lecture",
                false,
                &firefox,
            ),
        ] {
            let connection = zbus::connection::Builder::address(address.as_str())
                .expect("the private bus")
                .name(name)
                .expect("a player name")
                .serve_at(
                    "/org/mpris/MediaPlayer2",
                    FakePlayer {
                        status,
                        title,
                        can_go_next,
                        calls: std::sync::Arc::clone(calls),
                    },
                )
                .expect("the player interface")
                .serve_at("/org/mpris/MediaPlayer2", FakePlayerRoot { identity })
                .expect("the root interface")
                .build()
                .await
                .expect("the player connects");
            connections.push(connection);
        }
        connections
    });

    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });
    let Response::MediaPlayers { mut players } = daemon.request(Request::ListMediaPlayers) else {
        panic!("the players were not listed");
    };
    players.sort_by(|a, b| a.identity.cmp(&b.identity));
    let summary: Vec<(&str, &str, bool, bool)> = players
        .iter()
        .map(|p| (p.identity.as_str(), p.title.as_str(), p.playing, p.paused))
        .collect();
    assert_eq!(
        summary,
        [
            ("Firefox", "A lecture", false, true),
            ("Spotify", "Blue Monday", true, false)
        ]
    );

    let with = |id: &str, argument: &str| {
        daemon.request(Request::RunMediaCommandWith {
            id: id.to_owned(),
            argument: Some(argument.to_owned()),
        })
    };
    assert!(matches!(with("play-pause", "lecture"), Response::Ack));
    let Response::Error(err) = with("next-track", "firefox") else {
        panic!("a skip the player cannot make was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (
            ErrorKind::Unsupported,
            "Firefox cannot skip to the next track"
        )
    );
    let Response::Error(err) = with("play-pause", "vlc") else {
        panic!("a player nothing matches was not refused");
    };
    assert_eq!(err.message, "No media player matches \"vlc\"");
    assert!(matches!(
        daemon.request(Request::ControlMediaPlayer {
            player: "org.mpris.MediaPlayer2.spotify".into(),
            action: MediaPlayerAction::Next,
        }),
        Response::Ack
    ));
    let Response::Error(err) = daemon.request(Request::ControlMediaPlayer {
        player: "com.example.nope".into(),
        action: MediaPlayerAction::Next,
    }) else {
        panic!("a name that is not a player's was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    drop(daemon);
    let _ = bus.kill();
    let _ = bus.wait();
    assert_eq!(firefox.lock().unwrap().as_slice(), ["PlayPause"]);
    assert_eq!(spotify.lock().unwrap().as_slice(), ["Next"]);
}

/// Asks Search Files until `found` accepts the answer, or panics with the
/// last one: the indexer scans in the background, so the first queries can
/// come before the file is in the index — or before the helper has started.
fn search_files_until(
    daemon: &Daemon,
    query: &str,
    category: Option<&str>,
    found: impl Fn(&str, &[compass_ipc::FileHit]) -> bool,
) -> (String, Vec<compass_ipc::FileHit>) {
    use compass_ipc::{Request, Response};
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut last = None;
    while Instant::now() < deadline {
        match daemon.request(Request::SearchFiles {
            query: query.into(),
            category: category.map(str::to_owned),
        }) {
            Response::Files { heading, files } => {
                if found(&heading, &files) {
                    return (heading, files);
                }
                last = Some(format!("{heading}: {files:?}"));
            }
            other => last = Some(format!("{other:?}")),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("Search Files never answered {query:?} as expected; last answer: {last:?}");
}

#[test]
fn search_files_indexes_the_home_directory_and_finds_a_file_by_a_misspelled_query() {
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let documents = root.join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join("quarterly-report.pdf"), "%PDF").unwrap();
        std::fs::write(documents.join("holiday.png"), "png").unwrap();
        Vec::new()
    });
    let is_report = |files: &[compass_ipc::FileHit]| {
        files
            .first()
            .is_some_and(|file| file.name == "quarterly-report.pdf")
    };

    let (heading, files) = search_files_until(&daemon, "quartely report", None, |_, files| {
        is_report(files)
    });
    assert_eq!(heading, "Results");
    assert!(files[0].path.ends_with("/Documents/quarterly-report.pdf"));
    assert_eq!(files[0].category, "Documents");

    // The category filter reaches the index: images only, so no report.
    let (_, images) = search_files_until(&daemon, "holiday", Some("Images"), |_, files| {
        files.iter().any(|file| file.name == "holiday.png")
    });
    assert!(images.iter().all(|file| file.category == "Images"));
    let (_, none) = search_files_until(&daemon, "quarterly report", Some("Images"), |_, _| true);
    assert!(
        none.iter().all(|file| file.name != "quarterly-report.pdf"),
        "{none:?}"
    );
}

#[test]
fn search_files_lists_recent_files_for_the_empty_query_and_a_typed_path_directly() {
    use compass_ipc::{ErrorKind, Request, Response};
    let root_dir = std::sync::OnceLock::new();
    let opener = entry("Plain Opener", "MimeType=text/plain;\n");
    let entries = [
        ("a.desktop", entry("Alpha", "")),
        ("plain-opener.desktop", opener),
    ];
    let entries: Vec<(&str, &str)> = entries.iter().map(|(n, e)| (*n, e.as_str())).collect();
    let daemon = Daemon::start_prepared(&entries, "{}", |root| {
        root_dir.set(root.to_path_buf()).unwrap();
        std::fs::write(root.join("plain.txt"), "hello").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2]).unwrap();
        let notes = root.join("notes.md");
        std::fs::write(&notes, "# notes").unwrap();
        let data_home = root.join("data-home");
        std::fs::create_dir_all(&data_home).unwrap();
        std::fs::write(
            data_home.join("recently-used.xbel"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file://{}" added="2026-01-01T00:00:00Z" modified="2026-01-02T00:00:00Z" visited="2026-01-01T00:00:00Z"/>
  <bookmark href="file:///nonexistent/gone.txt" added="2026-01-01T00:00:00Z" modified="2026-01-03T00:00:00Z" visited="2026-01-01T00:00:00Z"/>
</xbel>
"#,
                notes.display()
            ),
        )
        .unwrap();
        Vec::new()
    });

    let Response::Files { heading, files } = daemon.request(Request::SearchFiles {
        query: String::new(),
        category: None,
    }) else {
        panic!("the empty query was not answered with files");
    };
    assert_eq!(heading, "Recently Accessed");
    assert_eq!(
        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["notes.md"],
        "the missing file is skipped"
    );

    let Response::Files { heading, files } = daemon.request(Request::SearchFiles {
        query: "~/notes.md".into(),
        category: None,
    }) else {
        panic!("a typed path was not answered with files");
    };
    assert_eq!(heading, "Direct file path");
    assert_eq!(files.len(), 1);
    assert!(files[0].path.ends_with("/notes.md"), "{files:?}");

    // Opening refuses a path that is not there, and a file no installed
    // application opens -- the fixture tree has none for binary data --
    // rather than launching anything on the machine running the tests, and
    // says so rather than acknowledging an open that did not happen (#253).
    let Response::Error(err) = daemon.request(Request::OpenFile {
        path: "/nonexistent/gone.txt".into(),
        reveal: false,
    }) else {
        panic!("opening a missing file was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    let root = root_dir.get().unwrap();
    let Response::Error(err) = daemon.request(Request::OpenFile {
        path: root.join("blob.bin").to_string_lossy().into_owned(),
        reveal: false,
    }) else {
        panic!("opening a file nothing opens was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert_eq!(err.message, "no application opens this kind of file");
    // Markdown has no opener of its own here, but it is text: the text
    // editor opens it, as Open Config File's JSON is opened (#253).
    assert_eq!(
        daemon.request(Request::OpenFile {
            path: files[0].path.clone(),
            reveal: false,
        }),
        Response::Ack
    );

    // Opening one something does open (`/bin/true` here) records it in
    // `recently-used.xbel`, keeping what was there.
    let plain = root.join("plain.txt");
    assert_eq!(
        daemon.request(Request::OpenFile {
            path: plain.to_string_lossy().into_owned(),
            reveal: false,
        }),
        Response::Ack
    );
    let xbel = std::fs::read_to_string(root.join("data-home/recently-used.xbel")).unwrap();
    let bookmarks = compass_xdg::bookmarks::parse(&xbel).unwrap();
    assert_eq!(bookmarks.len(), 3, "{xbel}");
    let recorded = bookmarks
        .iter()
        .find(|b| b.path().as_deref() == Some(plain.as_path()))
        .expect("the opened file is recorded");
    assert_eq!(recorded.mime_type.as_deref(), Some("text/plain"));
    assert_eq!(recorded.applications[0].name, "compass");
}

#[test]
fn search_files_without_indexing_says_the_index_is_unavailable() {
    use compass_ipc::{ErrorKind, Request, Response};
    let config = r#"{"providers": {"files": {"preferences": {"autoIndexing": false}}}}"#;
    let daemon = Daemon::start_with_config(&[("a.desktop", &entry("Alpha", ""))], config);
    let Response::Error(err) = daemon.request(Request::SearchFiles {
        query: "report".into(),
        category: None,
    }) else {
        panic!("an index search with indexing off was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(err.message.contains("file indexer"), "{}", err.message);
}

#[test]
fn an_oauth_authorization_opens_the_browser_and_the_redirect_answers_it() {
    use compass_extension_api::View;
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    // The browser is a script that writes down the URL it was given, which
    // is how the test learns the state the extension generated.
    let mut root = std::path::PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        install_extension(dir);
        root = dir.to_path_buf();
        let script = dir.join("browser.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > {:?}\n",
                dir.join("opened.txt").to_string_lossy()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let applications = dir.join("data/applications");
        std::fs::create_dir_all(&applications).unwrap();
        std::fs::write(
            applications.join("browser.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nName=Browser\nExec={} %u\n\
                 MimeType=x-scheme-handler/https;\n",
                script.to_string_lossy()
            ),
        )
        .unwrap();
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hello:auth".into(),
        arguments_json: None,
    }) else {
        panic!("no session");
    };
    wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));

    let opened = root.join("opened.txt");
    wait_for_content(&opened);
    let url = std::fs::read_to_string(&opened).expect("the browser was opened");
    assert!(
        url.starts_with("https://example.com/authorize?"),
        "the extension's own authorization URL, as built: {url}"
    );
    let state = compass_worker_host::oauth_service::query_value(&url, "state").expect("a state");

    // A redirect for some other request is refused, and does not settle this one.
    let Response::Error(err) = daemon.request(Request::OAuthRedirect {
        url: "raycast://oauth?code=nope&state=someone-else".into(),
    }) else {
        panic!("a redirect nobody waits on was accepted");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    let redirect = format!(
        "raycast://oauth?package_name=Extension&code=the-code&state={}",
        percent_encode(&state)
    );
    assert_eq!(
        daemon.request(Request::OAuthRedirect { url: redirect }),
        Response::Ack
    );
    let authorized = root.join("data-home/compass/support/hello/authorized.txt");
    wait_for_content(&authorized);
    assert_eq!(
        std::fs::read_to_string(&authorized).ok().as_deref(),
        Some("the-code"),
        "the extension's authorize() resolved with the code from the redirect"
    );
    daemon.request(Request::CloseExtension { session });
}

/// Percent-encodes everything but the unreserved characters.
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A fake application in its own directory that appends each argument it is
/// launched with to `<dir>/<name>.log`, and the desktop entry that runs it
/// for `mime_types`.
fn recording_app(dir: &Path, name: &str, mime_types: &str) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join(format!("{name}.log"));
    let program = dir.join(name);
    std::fs::write(
        &program,
        format!(
            // All at once, then renamed into place: a reader polling for a
            // non-empty log must not see the first argument without the rest.
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{log}.part' && mv '{log}.part' '{log}'\n",
            log = log.display()
        ),
    )
    .expect("fake application");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={} %u\nMimeType={mime_types}\n",
        program.display()
    );
    (entry, log)
}

/// Reads `log` until it has a line, or panics: a launch is spawned, so the
/// answer can arrive before the program has run.
fn launched_with(log: &Path) -> Vec<String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(log)
            && !text.is_empty()
        {
            return text.lines().map(str::to_owned).collect();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{} was never launched", log.display());
}

#[test]
fn shortcuts_are_imported_created_searched_opened_edited_and_removed() {
    use compass_ipc::{ErrorKind, Request, Response, ShortcutEntry};
    let bin = TempDir::new().expect("tempdir");
    let (browser, log) = recording_app(bin.path(), "browser", "x-scheme-handler/https;");
    let vicinae_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("browser.desktop", &browser)], "{}", |root| {
        let file = root.join("data-home/compass/shortcuts/shortcuts.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[{"id":"sct-aaaaaaaaaaaa","name":"Crate Docs","icon":"icon://omnicast/link",
                "url":"https://docs.rs/{crate}","app":"default","openCount":4,
                "createdAt":1700000000,"updatedAt":1700000000}]"#,
        )
        .unwrap();
        vicinae_file.set(file).unwrap();
        Vec::new()
    });
    let list = |response: Response| -> Vec<ShortcutEntry> {
        match response {
            Response::Shortcuts { shortcuts } => shortcuts,
            other => panic!("not a shortcut list: {other:?}"),
        }
    };

    let imported = list(daemon.request(Request::ListShortcuts));
    assert_eq!(imported.len(), 1, "Vicinae's shortcut came across");
    assert_eq!(imported[0].name, "Crate Docs");
    assert_eq!(imported[0].open_count, 4);

    let saved = list(daemon.request(Request::SaveShortcut {
        id: None,
        name: "Search Rust".into(),
        icon: "default".into(),
        url: "https://docs.rs/releases/search?query={query}".into(),
        app: "default".into(),
    }));
    assert_eq!(saved.len(), 2);
    let created = saved.iter().find(|s| s.name == "Search Rust").unwrap();
    assert!(created.id.starts_with("sct-"), "{}", created.id);
    assert_eq!(
        created.icon, "icon://favicon/docs.rs?fallback=icon://omnicast/image",
        "the default icon is resolved to the site's favicon when saved"
    );

    // Root search ranks it with everything else, by its name.
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "search rust".into(),
    }) else {
        panic!("no query results");
    };
    assert_eq!(
        hits.first().map(|hit| hit.id.as_str()),
        Some(format!("shortcuts:{}", created.id).as_str()),
        "{hits:?}"
    );

    let Response::Text { text } = daemon.request(Request::ExpandShortcut {
        id: created.id.clone(),
        arguments: vec!["serde json".into()],
    }) else {
        panic!("not expanded");
    };
    assert_eq!(text, "https://docs.rs/releases/search?query=serde%20json");

    assert_eq!(
        daemon.request(Request::OpenShortcut {
            id: created.id.clone(),
            arguments: vec!["serde".into()],
        }),
        Response::Ack
    );
    assert_eq!(
        launched_with(&log),
        ["https://docs.rs/releases/search?query=serde"],
        "the browser, which claims https, opened the expanded link"
    );
    let after = list(daemon.request(Request::ListShortcuts));
    let opened = after.iter().find(|s| s.id == created.id).unwrap();
    assert_eq!(opened.open_count, 1);
    assert!(opened.last_used_at.is_some());

    let edited = list(daemon.request(Request::SaveShortcut {
        id: Some(created.id.clone()),
        name: "Rust Search".into(),
        icon: "icon://omnicast/bolt".into(),
        url: created.url.clone(),
        app: "default".into(),
    }));
    let renamed = edited.iter().find(|s| s.id == created.id).unwrap();
    assert_eq!(renamed.name, "Rust Search");
    assert_eq!(renamed.icon, "icon://omnicast/bolt");
    assert_eq!(renamed.open_count, 1, "editing is not opening");

    let remaining = list(daemon.request(Request::RemoveShortcut {
        id: "sct-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(
        remaining.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [created.id.as_str()]
    );
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "crate docs".into(),
    }) else {
        panic!("no query results");
    };
    assert!(
        hits.iter()
            .all(|hit| hit.id != "shortcuts:sct-aaaaaaaaaaaa"),
        "a removed shortcut leaves root search: {hits:?}"
    );

    // Compass wrote its own file; Vicinae's is as it was.
    let vicinae = std::fs::read_to_string(vicinae_file.get().unwrap()).unwrap();
    assert!(vicinae.contains("Crate Docs"));
    let compass = std::fs::read_to_string(
        vicinae_file
            .get()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("compass-shortcuts.json"),
    )
    .unwrap();
    assert!(compass.contains("Rust Search") && !compass.contains("Crate Docs"));

    let Response::Error(err) = daemon.request(Request::OpenShortcut {
        id: "sct-gone".into(),
        arguments: vec![],
    }) else {
        panic!("opening a missing shortcut was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    let Response::Error(err) = daemon.request(Request::SaveShortcut {
        id: None,
        name: "No link".into(),
        icon: "default".into(),
        url: String::new(),
        app: "default".into(),
    }) else {
        panic!("a shortcut with no link was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn snippets_are_imported_created_expanded_edited_and_removed() {
    use compass_ipc::{ErrorKind, Request, Response, SnippetEntry};
    let vicinae_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let file = root.join("data-home/compass/snippets/snippets.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"[{"id":"snp-aaaaaaaaaaaa","name":"Signature","data":{"text":"Best,\nMe"},
                "createdAt":1700000000,"expansion":{"keyword":";sig","apps":[],"word":true}}]"#,
        )
        .unwrap();
        vicinae_file.set(file).unwrap();
        Vec::new()
    });
    let list = |response: Response| -> Vec<SnippetEntry> {
        match response {
            Response::Snippets { snippets } => snippets,
            other => panic!("not a snippet list: {other:?}"),
        }
    };
    let refused = |response: Response| -> (ErrorKind, String) {
        match response {
            Response::Error(err) => (err.kind, err.message),
            other => panic!("not refused: {other:?}"),
        }
    };

    let imported = list(daemon.request(Request::ListSnippets));
    assert_eq!(imported.len(), 1, "Vicinae's snippet came across");
    assert_eq!(imported[0].keyword.as_deref(), Some(";sig"));
    assert_eq!(imported[0].text.as_deref(), Some("Best,\nMe"));

    let saved =
        list(
            daemon.request(Request::SaveSnippet {
                id: None,
                name: "Greeting".into(),
                text:
                    "Hello {name}, {date format=\"yyyy\"} {shell code=\"echo shell-ran\"}{cursor}"
                        .into(),
                keyword: Some(";hi".into()),
                word: false,
                apps: vec![],
            }),
        );
    assert_eq!(saved.len(), 2);
    let greeting = saved.iter().find(|s| s.name == "Greeting").unwrap().clone();
    assert!(greeting.id.starts_with("snp-"));
    assert!(!greeting.word);

    let (kind, message) = refused(daemon.request(Request::SaveSnippet {
        id: None,
        name: "Other".into(),
        text: "x".into(),
        keyword: Some(";sig".into()),
        word: true,
        apps: vec![],
    }));
    assert_eq!(kind, ErrorKind::BadRequest);
    assert_eq!(message, "keyword already assigned to \"Signature\"");
    let (kind, message) = refused(daemon.request(Request::SaveSnippet {
        id: None,
        name: "x".into(),
        text: "{cursor}{cursor}".into(),
        keyword: Some("has space".into()),
        word: true,
        apps: vec![],
    }));
    assert_eq!(kind, ErrorKind::BadRequest);
    assert!(
        message.contains("2 chars min.")
            && message.contains("Only one {cursor}")
            && message.contains("printable ASCII"),
        "{message}"
    );

    let Response::Text { text } = daemon.request(Request::ExpandSnippet {
        id: greeting.id.clone(),
        arguments: vec![("name".into(), "Zoë".into())],
    }) else {
        panic!("not expanded");
    };
    let year: i32 = text
        .split(", ")
        .nth(1)
        .and_then(|rest| rest.get(..4))
        .and_then(|year| year.parse().ok())
        .unwrap_or_else(|| panic!("no year in {text:?}"));
    assert!(year >= 2024, "{text}");
    assert_eq!(text, format!("Hello Zoë, {year} shell-ran"));

    // Manage Snippets' pane shows the shell placeholder rather than running it.
    let Response::Text { text } = daemon.request(Request::PreviewSnippet {
        id: greeting.id.clone(),
        arguments: vec![("name".into(), "Zoë".into())],
    }) else {
        panic!("not previewed");
    };
    assert_eq!(text, format!("Hello Zoë, {year} $(echo shell-ran)"));

    // Pasting needs the Shell extension, and this engine has no session bus.
    let (kind, _) = refused(daemon.request(Request::PasteSnippet {
        id: greeting.id.clone(),
        arguments: vec![],
    }));
    assert_eq!(kind, ErrorKind::Unsupported);

    let edited = list(daemon.request(Request::SaveSnippet {
        id: Some(greeting.id.clone()),
        name: "Hello".into(),
        text: "Hi".into(),
        keyword: Some(";hi".into()),
        word: true,
        apps: vec![],
    }));
    let hello = edited.iter().find(|s| s.id == greeting.id).unwrap();
    assert_eq!(hello.name, "Hello");
    assert!(hello.updated_at.is_some());
    assert_eq!(hello.created_at, greeting.created_at);

    let remaining = list(daemon.request(Request::RemoveSnippet {
        id: "snp-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(remaining.len(), 1);
    let (kind, _) = refused(daemon.request(Request::RemoveSnippet {
        id: "snp-aaaaaaaaaaaa".into(),
    }));
    assert_eq!(kind, ErrorKind::BadRequest);

    let vicinae = std::fs::read_to_string(vicinae_file.get().unwrap()).unwrap();
    assert!(vicinae.contains("Signature"), "Vicinae's file is untouched");
    let compass = std::fs::read_to_string(
        vicinae_file
            .get()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("compass-snippets.json"),
    )
    .unwrap();
    assert!(compass.contains("\"name\":\"Hello\"") && !compass.contains("Signature"));
}

/// Asks for a script run's output until it has finished, or panics.
fn script_output_until_finished(daemon: &Daemon, session: u64) -> (String, Option<i32>) {
    use compass_ipc::{Request, Response};
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        match daemon.request(Request::ScriptOutput { session }) {
            Response::ScriptOutput {
                output,
                finished: true,
                exit_code,
                ..
            } => return (output, exit_code),
            Response::ScriptOutput { .. } => {}
            other => panic!("not a script output: {other:?}"),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("script run {session} never finished");
}

#[test]
fn script_commands_are_scanned_searched_and_run_in_their_modes() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let marker = std::sync::OnceLock::new();
    let custom = TempDir::new().expect("tempdir");
    let write = |dir: &Path, name: &str, mode: &str, title: &str, body: &str| {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title {title}\n\
                 # @raycast.mode {mode}\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write(
        custom.path(),
        "compact.sh",
        "compact",
        "Custom Compact",
        "echo custom; exit 1",
    );
    let config = format!(
        r#"{{"providers": {{"scripts": {{"preferences": {{"customDirs": ["{}"]}}}}}}}}"#,
        custom.path().display()
    );
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], &config, |root| {
        let dir = root.join("data-home/compass/scripts");
        write(
            &dir,
            "full.sh",
            "fullOutput",
            "Full Report",
            "# @raycast.argument1 {\"type\":\"text\",\"placeholder\":\"who\"}\n\
             printf '\\033[32mgreen\\033[0m %s https://x.test\\n' \"$1\"; echo err >&2",
        );
        write(
            &dir,
            "inline.sh",
            "inline",
            "Queue Size",
            "# @raycast.icon 🎉\necho '42 items'",
        );
        write(
            &dir,
            "compact.sh",
            "compact",
            "Packaged Compact",
            "echo packaged",
        );
        let flag = root.join("silent-ran");
        write(
            &dir.join("tools"),
            "silent.sh",
            "silent",
            "Touch Marker",
            &format!("touch '{}'", flag.display()),
        );
        marker.set(flag).unwrap();
        Vec::new()
    });

    let Response::Scripts { scripts } = daemon.request(Request::ListScripts) else {
        panic!("no script list");
    };
    let mut listed: Vec<(&str, &str, &str)> = scripts
        .iter()
        .map(|s| (s.id.as_str(), s.title.as_str(), s.mode.as_str()))
        .collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            ("compact.sh", "Custom Compact", "compact"),
            ("full.sh", "Full Report", "fullOutput"),
            ("inline.sh", "Queue Size", "inline"),
            ("tools.silent.sh", "Touch Marker", "silent"),
        ],
        "a custom directory's script shadows the packaged one with the same id"
    );
    let full = scripts.iter().find(|s| s.id == "full.sh").unwrap();
    assert_eq!(full.arguments.len(), 1);
    let Response::ScriptIcons { icons } = daemon.request(Request::ScriptIcons) else {
        panic!("no script icons");
    };
    assert_eq!(icons.len(), 4, "one per script: {icons:?}");
    for (id, icon) in &icons {
        let want = if id == "inline.sh" {
            "icon://emoji/🎉"
        } else {
            "icon://omnicast/code?bg_tint=accent"
        };
        assert_eq!(icon, want, "{id}: an emoji header, else the default");
    }
    assert_eq!(full.arguments[0].placeholder.as_deref(), Some("who"));

    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "full report".into(),
    }) else {
        panic!("no query results");
    };
    assert_eq!(hits.first().map(|h| h.id.as_str()), Some("scripts:full.sh"));

    let started = |id: &str, arguments: Vec<String>| match daemon.request(Request::RunScript {
        id: id.into(),
        arguments,
    }) {
        Response::ScriptStarted { session } => session,
        other => panic!("{id} did not start: {other:?}"),
    };

    let session = started("full.sh", vec!["Zoë".into()]).expect("full output is followed");
    let (output, exit) = script_output_until_finished(&daemon, session);
    assert_eq!(exit, Some(0));
    assert!(
        output.contains("\u{1b}[32mgreen\u{1b}[0m Zoë https://x.test"),
        "{output:?}"
    );
    assert!(output.contains("err"), "stderr is part of full output");

    let session = started("inline.sh", vec![]).expect("inline is followed");
    let (output, _) = script_output_until_finished(&daemon, session);
    assert_eq!(output.trim(), "42 items");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let Response::Scripts { scripts } = daemon.request(Request::ListScripts) else {
            panic!("no script list");
        };
        let inline = scripts.iter().find(|s| s.id == "inline.sh").unwrap();
        if inline.subtitle == "42 items" {
            break;
        }
        assert_eq!(inline.subtitle, "No data");
        assert!(
            Instant::now() < deadline,
            "the inline line never became the subtitle"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let session = started("compact.sh", vec![]).expect("compact is followed");
    assert_eq!(
        script_output_until_finished(&daemon, session),
        ("custom\n".to_owned(), Some(1))
    );

    assert_eq!(started("tools.silent.sh", vec![]), None);
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while !marker.get().unwrap().exists() {
        assert!(Instant::now() < deadline, "the silent script never ran");
        std::thread::sleep(Duration::from_millis(25));
    }

    let Response::Error(err) = daemon.request(Request::RunScript {
        id: "nothing.sh".into(),
        arguments: vec![],
    }) else {
        panic!("an unknown script was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn run_terminal_program_lists_path_and_runs_directly_or_refuses() {
    use compass_ipc::{ErrorKind, Request, Response};
    let bin = TempDir::new().expect("tempdir");
    let (_, log) = recording_app(bin.path(), "fake-tool", "");
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .expect("PATH");
    let config = r#"{"providers": {"commands": {"entrypoints": {"run-program":
        {"preferences": {"default-action": "run"}}}}}}"#;
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], config, |_| {
        vec![("PATH", path)]
    });

    let Response::Programs {
        programs,
        terminal,
        default_action,
    } = daemon.request(Request::ListPrograms)
    else {
        panic!("no program list");
    };
    let tool = bin.path().join("fake-tool").to_string_lossy().into_owned();
    assert!(programs.contains(&tool), "{programs:?}");
    assert_eq!(terminal, None, "the fixture installs no terminal");
    assert_eq!(default_action, "run", "read from the command's preferences");

    assert_eq!(
        daemon.request(Request::RunProgram {
            argv: vec!["fake-tool".into(), "--flag".into(), "two words".into()],
            terminal: false,
            hold: false,
        }),
        Response::Ack
    );
    assert_eq!(launched_with(&log), ["--flag", "two words"]);

    let Response::Error(err) = daemon.request(Request::RunProgram {
        argv: vec!["no-such-tool-anywhere".into()],
        terminal: false,
        hold: false,
    }) else {
        panic!("a missing program was not refused");
    };
    assert_eq!(
        (err.kind, err.message.as_str()),
        (ErrorKind::BadRequest, "Not a valid executable")
    );
    let Response::Error(err) = daemon.request(Request::RunProgram {
        argv: vec!["fake-tool".into()],
        terminal: true,
        hold: true,
    }) else {
        panic!("a terminal run with no terminal was not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

#[test]
fn create_extension_writes_the_boilerplate_under_home() {
    use compass_ipc::{ErrorKind, Request, Response};
    let home = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        std::fs::create_dir_all(root.join("code")).unwrap();
        home.set(root.to_path_buf()).unwrap();
        Vec::new()
    });
    let request = |location: &str, author: &str| Request::CreateExtension {
        author: author.into(),
        title: "Hello World".into(),
        description: "Says hello to the whole world".into(),
        location: location.into(),
        command_title: "Say Hello".into(),
        command_description: "Says hello".into(),
        template: ":boilerplate/tmpl-no-view".into(),
    };
    let Response::ExtensionCreated { path } = daemon.request(request("~/code", "zoe")) else {
        panic!("not created");
    };
    let root = std::path::Path::new(&path);
    assert!(root.starts_with(home.get().unwrap().join("code")), "{path}");
    let manifest = std::fs::read_to_string(root.join("package.json")).unwrap();
    assert!(manifest.contains("\"mode\": \"no-view\""), "{manifest}");

    let Response::Error(err) = daemon.request(request("~/nowhere", "z")) else {
        panic!("an invalid form was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(
        err.message.contains("location: Must exist"),
        "{}",
        err.message
    );
}

#[test]
fn the_settings_view_writes_each_setting_and_switch_into_the_configuration() {
    use compass_ipc::{ErrorKind, Request, Response};
    let entries = [
        ("alpha.desktop", entry("Alpha Editor", "")),
        ("beta.desktop", entry("Beta Editor", "")),
    ];
    let entries = entries
        .iter()
        .map(|(id, body)| (*id, body.as_str()))
        .collect::<Vec<_>>();
    let config_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&entries, r#"{"mystery": {"kept": true}}"#, |root| {
        config_file
            .set(root.join("config/compass/compass.json"))
            .unwrap();
        Vec::new()
    });
    let saved = || {
        compass_core::Config::load_from(config_file.get().unwrap())
            .expect("the file stays a configuration")
    };
    let set = |key: &str, value: serde_json::Value| {
        daemon.request(Request::SetSetting {
            key: key.into(),
            value_json: value.to_string(),
        })
    };

    assert_eq!(
        set("launcher.wrap_navigation", serde_json::json!(true)),
        Response::Ack
    );
    assert_eq!(
        set("launcher.keybinding", serde_json::json!("emacs")),
        Response::Ack
    );
    assert_eq!(
        set("launcher.hotkey", serde_json::json!("alt+space")),
        Response::Ack
    );
    assert_eq!(
        set("launcher.appearance.theme", serde_json::json!("Nord")),
        Response::Ack
    );
    assert_eq!(
        set(
            "providers.clipboard.preferences.evictionThreshold",
            serde_json::json!("86400")
        ),
        Response::Ack
    );
    let config = saved();
    assert!(config.launcher().wrap_navigation());
    assert_eq!(config.launcher().keybinding(), "emacs");
    assert_eq!(config.launcher().hotkey(), "alt+space");
    assert_eq!(config.launcher().appearance().theme(), "nord");
    assert_eq!(
        config.get_path("providers.clipboard.preferences.evictionThreshold"),
        Some(serde_json::json!("86400"))
    );
    assert_eq!(
        config.get_path("mystery.kept"),
        Some(serde_json::json!(true))
    );

    assert_eq!(
        set("launcher.wrap_navigation", serde_json::Value::Null),
        Response::Ack
    );
    assert!(!saved().launcher().wrap_navigation(), "null resets it");

    for (key, value) in [
        ("launcher.keybinding", serde_json::json!("qwerty")),
        (
            "launcher.appearance.theme",
            serde_json::json!("no-such-theme"),
        ),
        ("launcher.window_opacity", serde_json::json!(0.5)),
    ] {
        let Response::Error(err) = set(key, value) else {
            panic!("{key} should be refused");
        };
        assert_eq!(err.kind, ErrorKind::BadRequest, "{key}: {}", err.message);
    }
    assert_eq!(
        saved().launcher().keybinding(),
        "emacs",
        "a refusal writes nothing"
    );

    let ids = |query: &str| -> Vec<String> {
        let Response::QueryResults { hits } = daemon.request(Request::Query { text: query.into() })
        else {
            panic!("expected query results");
        };
        hits.into_iter()
            .map(|hit| hit.id)
            .filter(|id| id.starts_with("applications:"))
            .collect()
    };
    assert_eq!(
        daemon.request(Request::RootItemEdit {
            id: "applications:alpha".into(),
            edit: compass_ipc::RootItemEdit::Disable,
        }),
        Response::Ack
    );
    assert_eq!(ids("Editor"), ["applications:beta"]);
    assert_eq!(
        daemon.request(Request::RootItemEdit {
            id: "applications:alpha".into(),
            edit: compass_ipc::RootItemEdit::Enabled(true),
        }),
        Response::Ack
    );
    assert_eq!(ids("Editor").len(), 2, "the switch turns it back on");
    assert_eq!(
        daemon.request(Request::SetProviderEnabled {
            provider: "applications".into(),
            enabled: false,
        }),
        Response::Ack
    );
    assert!(ids("Editor").is_empty(), "the whole provider is off");
    assert_eq!(
        saved().root_config().providers["applications"].enabled,
        Some(false)
    );
    assert_eq!(
        daemon.request(Request::SetProviderEnabled {
            provider: "applications".into(),
            enabled: true,
        }),
        Response::Ack
    );
    assert_eq!(ids("Editor").len(), 2);
}

#[test]
fn set_theme_keeps_the_theme_in_the_configuration() {
    use compass_ipc::{ErrorKind, Request, Response};
    let config_file = std::sync::OnceLock::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        config_file
            .set(root.join("config/compass/compass.json"))
            .unwrap();
        let themes = root.join("data-home/compass/themes");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(
            themes.join("harbour.toml"),
            "[meta]\nname = \"Harbour\"\ndescription = \"\"\nvariant = \"dark\"\n",
        )
        .unwrap();
        Vec::new()
    });
    assert_eq!(
        daemon.request(Request::SetTheme {
            theme: "harbour".into()
        }),
        Response::Ack,
        "a theme file in the user's theme directory is a theme"
    );
    let saved = std::fs::read_to_string(config_file.get().unwrap()).unwrap();
    assert!(saved.contains("\"harbour\""), "{saved}");

    // "Set as Compass font" writes `font.normal.family` beside it.
    assert_eq!(
        daemon.request(Request::SetFont {
            family: "Fira Sans".into()
        }),
        Response::Ack
    );
    let saved = std::fs::read_to_string(config_file.get().unwrap()).unwrap();
    let parsed = compass_core::Config::parse(&saved, std::path::Path::new("compass.json")).unwrap();
    assert_eq!(parsed.font_family(), Some("Fira Sans"));
    assert!(saved.contains("\"harbour\""), "the theme is kept: {saved}");
    assert_eq!(
        daemon.request(Request::SetTheme {
            theme: "Tokyo-Night".into()
        }),
        Response::Ack
    );
    let saved = std::fs::read_to_string(config_file.get().unwrap()).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert!(
        saved.to_string().contains("\"tokyo-night\""),
        "the persisted spelling is written: {saved}"
    );
    let Response::Error(err) = daemon.request(Request::SetTheme {
        theme: "no-such-theme".into(),
    }) else {
        panic!("an unknown theme was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

#[test]
fn browse_fonts_lists_families_and_previews_one() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let Response::Fonts { fonts, categories } = daemon.request(Request::ListFonts) else {
        panic!("no font list");
    };
    assert!(categories.iter().any(|category| category == "Latin"));
    for font in &fonts {
        assert!(
            font.categories.contains(&font.primary),
            "{font:?} is listed under a category it cannot be filtered by"
        );
    }
    if let Some(font) = fonts.first() {
        let Response::Text { text } = daemon.request(Request::FontSpecimen {
            name: font.name.clone(),
        }) else {
            panic!("no specimen for {}", font.name);
        };
        assert!(!text.is_empty());
    }
    let Response::Error(err) = daemon.request(Request::FontSpecimen {
        name: "No Such Family 123".into(),
    }) else {
        panic!("an unknown family was not refused");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

/// Runs `compass dmenu` against `daemon` with `stdin`, returning its output.
fn run_dmenu(daemon: &Daemon, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("dmenu")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run compass dmenu");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("compass dmenu finished")
}

#[test]
fn dmenu_shows_stdin_in_the_attached_window_and_prints_the_choice() {
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);

    // No window yet: refused, and nothing printed.
    let refused = run_dmenu(&daemon, &[], "a\nb\n");
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());

    // A fake window that picks the second entry for an index list, and
    // dismisses anything else.
    let socket = daemon.socket.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<compass_ipc::DmenuSpec>::new()));
    let window = {
        let seen = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(async move {
                let mut window = compass_ipc::WindowClient::attach(&socket)
                    .await
                    .expect("attach");
                ready_tx.send(()).expect("ready");
                for _ in 0..2 {
                    let Some(compass_ipc::WindowCommand::Dmenu(token)) =
                        window.next_command().await.expect("a command")
                    else {
                        panic!("expected a dmenu command");
                    };
                    window
                        .reply(compass_ipc::WindowOutcome::Shown)
                        .await
                        .expect("reply");
                    let mut client = compass_ipc::Client::connect(&socket).await.expect("client");
                    let compass_ipc::Response::DmenuList { spec } = client
                        .request(compass_ipc::Request::DmenuFetch { token })
                        .await
                        .expect("fetch")
                    else {
                        panic!("no dmenu list");
                    };
                    let output = spec.output_index.then(|| "1".to_owned());
                    seen.lock().expect("lock").push(spec);
                    let answered = client
                        .request(compass_ipc::Request::DmenuChoose { token, output })
                        .await
                        .expect("choose");
                    assert_eq!(answered, compass_ipc::Response::Ack);
                }
            });
        })
    };
    ready_rx.recv_timeout(STARTUP_TIMEOUT).expect("attached");

    let chosen = run_dmenu(
        &daemon,
        &["--format", "index", "-p", "Pick one", "-W", "300"],
        "alpha\nbeta\n\ngamma\n",
    );
    assert!(
        chosen.status.success(),
        "{}",
        String::from_utf8_lossy(&chosen.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&chosen.stdout), "1\n");

    let dismissed = run_dmenu(&daemon, &[], "alpha\n");
    assert_eq!(
        dismissed.status.code(),
        Some(1),
        "dismissed: exit 1, as the C++"
    );
    assert!(dismissed.stdout.is_empty());

    window.join().expect("the fake window finished");
    let seen = seen.lock().expect("lock");
    assert_eq!(seen[0].content, "alpha\nbeta\n\ngamma\n");
    assert_eq!(seen[0].placeholder.as_deref(), Some("Pick one"));
    assert!(
        seen[0].no_quick_look && seen[0].no_footer,
        "a list narrower than 500 px drops quick look and the footer"
    );
    assert!(!seen[1].output_index);
}

// ---------------------------------------------------------------------------
// Rhai scripts. The in-process tests in `rhai_scripts.rs` cover actions,
// consent and effects against a fake clipboard; these hold the real process
// to the XDG layout: a packaged script under `$XDG_DATA_DIRS`, the user's
// directory created under `$XDG_DATA_HOME`, and hot reload.

#[test]
fn a_packaged_rhai_script_is_found_in_root_search_and_its_view_renders() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/rhai-examples");
    let mut root = PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        let target = dir.join("data/compass/scripts/web-search");
        std::fs::create_dir_all(&target).unwrap();
        for file in ["script.toml", "main.rhai"] {
            std::fs::copy(examples.join("web-search").join(file), target.join(file)).unwrap();
        }
        root = dir.to_path_buf();
        Vec::new()
    });
    let user_dir = root.join("data-home/compass/scripts");
    assert!(user_dir.is_dir(), "the engine creates the user's directory");

    let Response::RhaiScripts { scripts } = daemon.request(Request::ListRhaiScripts) else {
        panic!("no script list");
    };
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].title, "Web Search");
    assert_eq!(scripts[0].icon.as_deref(), Some("globe"));

    let json = daemon.client(&["query", "--json", "duckduckgo"]);
    assert!(json.contains("rhai:script.web-search"), "{json}");

    let started = daemon.request(Request::RunExtensionCommand {
        id: "rhai:script.web-search".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("the script did not open: {started:?}");
    };
    let (view, depth) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    assert_eq!(depth, 1);
    let View::List(list) = view else {
        unreachable!()
    };
    let titles: Vec<&str> = list.sections[0]
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(titles, ["DuckDuckGo", "Wikipedia", "GitHub", "crates.io"]);
    let handler = list.search.on_change.expect("the script takes the text");
    assert_eq!(
        daemon.request(Request::ExtensionEvent {
            session,
            handler: handler.0,
            args_json: r#"["rust", 1]"#.into(),
        }),
        Response::Ack
    );
    let (view, _) = wait_for_view(&daemon, session, |view, _| {
        matches!(view, View::List(list)
            if list.sections[0].items[0].title.contains("rust"))
    });
    let View::List(list) = view else {
        unreachable!()
    };
    assert_eq!(
        list.sections[0].items[0].subtitle.as_deref(),
        Some("https://duckduckgo.com/?q=rust")
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );

    // A script saved into the user's directory while the engine runs is in
    // root search without asking for a rescan.
    let mine = user_dir.join("mine");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("main.rhai"), "fn search(q) { [] }").unwrap();
    std::fs::write(mine.join("script.toml"), "title = \"Zebra Notes\"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let json = daemon.client(&["query", "--json", "zebra notes"]);
        if json.contains("rhai:script.mine") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hot reload never listed it: {json}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The packaged System Health script against this machine's own systemctl,
/// bootc, rpm-ostree and flatpak, whatever they are: on a systemd host its
/// first section lists failed units, anywhere else it says why not, and
/// either way it answers within the script's own limit.
#[test]
fn the_system_health_script_answers_on_whatever_host_runs_it() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/rhai-examples");
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        let target = dir.join("data/compass/scripts/system-health");
        std::fs::create_dir_all(&target).unwrap();
        for file in ["script.toml", "main.rhai"] {
            std::fs::copy(examples.join("system-health").join(file), target.join(file)).unwrap();
        }
        Vec::new()
    });
    let started = daemon.request(Request::RunExtensionCommand {
        id: "rhai:script.system-health".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("the script did not open: {started:?}");
    };
    let (view, _) = wait_for_view(&daemon, session, |view, _| matches!(view, View::List(_)));
    let View::List(list) = view else {
        unreachable!()
    };
    let sections: Vec<&str> = list
        .sections
        .iter()
        .map(|s| s.title.as_deref().unwrap_or_default())
        .collect();
    assert!(
        sections == ["Failed System Units", "Failed User Units", "Updates"]
            || sections == ["systemd", "Updates"],
        "{sections:?}"
    );
    let tools: Vec<&str> = list
        .sections
        .last()
        .unwrap()
        .items
        .iter()
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(tools, ["bootc", "rpm-ostree", "flatpak"]);
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

// ---- The extension stores, against a local fake ----

/// A local stand-in for `api.vicinae.com` (under `/v1`) and
/// `backend.raycast.com` (under `/raycast`), serving fixture listings and
/// bundles. The Raycast extension's commit can be moved on to publish an
/// update.
struct FakeStore {
    base: String,
    raycast_commit: std::sync::Arc<std::sync::Mutex<String>>,
    _server: std::sync::Arc<tiny_http::Server>,
}

fn bundle(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in entries {
        out.start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("zip entry");
        out.write_all(content.as_bytes()).expect("zip write");
    }
    out.finish().expect("zip").into_inner()
}

/// `GET /extensions/raycast/github`, recorded from the live Raycast API.
const RAYCAST_GITHUB: &str =
    include_str!("../../compass-core/tests/fixtures/raycast-extension-github.json");

impl FakeStore {
    fn start() -> FakeStore {
        let server =
            std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("fake store"));
        let port = server.server_addr().to_ip().expect("ip").port();
        let base = format!("http://127.0.0.1:{port}");
        let raycast_commit = std::sync::Arc::new(std::sync::Mutex::new("c1".to_owned()));

        let clock = bundle(&[
            (
                "clock/package.json",
                r#"{"name": "clock", "title": "Clock", "author": "zoe",
                    "commands": [{"name": "show-time", "title": "Show Time Now", "mode": "view"}]}"#,
            ),
            ("clock/show-time.js", "module.exports = {};"),
        ]);
        let hn = bundle(&[
            (
                "hn/package.json",
                r#"{"name": "hn", "title": "Hacker News", "author": "ray",
                    "dependencies": {"@raycast/api": "1.0.0"},
                    "commands": [{"name": "front", "title": "Front Page Stories", "mode": "view"}]}"#,
            ),
            ("hn/front.js", "module.exports = {};"),
        ]);
        let github = bundle(&[
            (
                "github/package.json",
                r#"{"name": "github", "title": "GitHub", "author": "thomaslombart",
                    "owner": "raycast", "dependencies": {"@raycast/api": "1.0.0"},
                    "commands": [{"name": "my-pull-requests", "title": "My Pull Requests",
                                  "mode": "view"}]}"#,
            ),
            ("github/my-pull-requests.js", "module.exports = {};"),
        ]);
        let evil = bundle(&[
            ("evil/package.json", r#"{"name": "evil", "commands": []}"#),
            ("evil/../../escaped.txt", "gotcha"),
        ]);

        let vicinae_listing = serde_json::json!({
            "extensions": [
                {
                    "id": "x1", "name": "clock", "title": "Clock",
                    "description": "Shows the time",
                    "author": {"handle": "zoe", "name": "Zoë", "avatarUrl": "", "profileUrl": ""},
                    "downloadCount": 1001, "checksum": "v1",
                    "platforms": ["linux"], "categories": [{"id": "system", "name": "System"}],
                    "commands": [{"id": "c", "name": "show-time", "title": "Show Time Now",
                                  "subtitle": "", "description": "Says the time", "mode": "view"}],
                    "readmeUrl": format!("{base}/readme/clock.md"),
                    "downloadUrl": format!("{base}/dl/clock.zip"),
                    "updatedAt": "2026-07-02T11:50:22.441Z"
                },
                {
                    "id": "x2", "name": "evil", "title": "Evil", "description": "Climbs out",
                    "author": {"handle": "mallory", "name": "Mallory", "avatarUrl": "", "profileUrl": ""},
                    "downloadUrl": format!("{base}/dl/evil.zip")
                },
                {
                    "id": "x3", "name": "mac-only", "title": "Mac Only", "description": "Not here",
                    "platforms": ["macos"]
                }
            ],
            "pagination": {"page": 1, "limit": 500, "total": 3, "totalPages": 1}
        })
        .to_string();

        let thread_server = std::sync::Arc::clone(&server);
        let commit = std::sync::Arc::clone(&raycast_commit);
        let base_for_thread = base.clone();
        std::thread::spawn(move || {
            for request in thread_server.incoming_requests() {
                let url = request.url().to_owned();
                let hn_json = || {
                    serde_json::json!({
                        "id": "u1", "name": "hn", "title": "Hacker News",
                        "description": "Read the front page",
                        "author": {"name": "Ray", "handle": "ray"},
                        "platforms": ["macOS"], "download_count": 2500,
                        "commit_sha": *commit.lock().unwrap(),
                        "metadata_count": 1,
                        "readme_assets_path": format!("{base_for_thread}/assets/"),
                        "store_url": "https://www.raycast.com/ray/hn",
                        "download_url": format!("{base_for_thread}/dl/hn.zip"),
                        "updated_at": 1_788_465_682,
                        "commands": [{"id": "c", "name": "front", "title": "Front Page Stories",
                                      "mode": "view"}]
                    })
                };
                // An organisation's extension, as the live API serves it: filed
                // under its owner, `raycast`, not its author.
                let github_json = || {
                    let mut listing: serde_json::Value =
                        serde_json::from_str(RAYCAST_GITHUB).expect("the recorded listing");
                    listing["download_url"] = format!("{base_for_thread}/dl/github.zip").into();
                    listing
                };
                let (status, body): (u16, Vec<u8>) = match url.as_str() {
                    "/v1/store/list?page=1&limit=500" => (200, vicinae_listing.clone().into_bytes()),
                    "/v1/raycast/get-compat" => (
                        200,
                        br#"{"hn": {"status": "partial", "confidence": "high", "notes": ["No menu bar"]}}"#
                            .to_vec(),
                    ),
                    "/readme/clock.md" => (200, b"## Usage\n\nPress it.".to_vec()),
                    "/dl/clock.zip" => (200, clock.clone()),
                    "/dl/hn.zip" => (200, hn.clone()),
                    "/dl/evil.zip" => (200, evil.clone()),
                    "/dl/github.zip" => (200, github.clone()),
                    "/raycast/store_listings/search?q=github" => (
                        200,
                        serde_json::json!({"data": [github_json()]})
                            .to_string()
                            .into_bytes(),
                    ),
                    "/raycast/extensions/raycast/github" => {
                        (200, github_json().to_string().into_bytes())
                    }
                    "/raycast/store_listings?page=1&per_page=50"
                    | "/raycast/store_listings/search?q=hacker%20news" => (
                        200,
                        serde_json::json!({"data": [hn_json()]})
                            .to_string()
                            .into_bytes(),
                    ),
                    "/raycast/extensions/ray/hn" => (200, hn_json().to_string().into_bytes()),
                    _ => (404, b"not found".to_vec()),
                };
                let _ =
                    request.respond(tiny_http::Response::from_data(body).with_status_code(status));
            }
        });
        FakeStore {
            base,
            raycast_commit,
            _server: server,
        }
    }

    fn start_engine(&self) -> Daemon {
        self.start_engine_with_config("{}")
    }

    fn start_engine_with_config(&self, config: &str) -> Daemon {
        let vicinae = format!("{}/v1", self.base);
        let raycast = format!("{}/raycast", self.base);
        Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], config, move |_| {
            vec![
                ("COMPASS_VICINAE_API_URL", vicinae.into()),
                ("COMPASS_RAYCAST_API_URL", raycast.into()),
            ]
        })
    }
}

fn root_ids(daemon: &Daemon, text: &str) -> Vec<String> {
    use compass_ipc::{Request, Response};
    let Response::QueryResults { hits } = daemon.request(Request::Query { text: text.into() })
    else {
        panic!("no query results");
    };
    hits.into_iter().map(|hit| hit.id).collect()
}

#[test]
fn the_vicinae_store_lists_installs_into_root_search_and_uninstalls() {
    use compass_ipc::{ErrorKind, Request, Response, StoreKind};
    let store = FakeStore::start();
    let daemon = store.start_engine();
    let data_home = daemon._dirs.path().join("data-home/compass");
    let extensions = data_home.join("extensions");

    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert_eq!(heading, "Extensions");
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["clock", "evil"], "the macOS-only one is dropped");
    assert_eq!(entries[0].id, "store.vicinae.clock");
    assert_eq!(entries[0].downloads, "1.1K");
    assert!(!entries[0].installed);

    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: "time".into(),
    }) else {
        panic!("no filtered listing");
    };
    assert_eq!(entries.len(), 1, "filtered by the description");

    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Vicinae,
        owner: "zoe".into(),
        name: "clock".into(),
    }) else {
        panic!("no detail");
    };
    assert!(
        detail.markdown.starts_with("# Clock\n"),
        "{}",
        detail.markdown
    );
    assert!(detail.markdown.contains("**Updated** 2026-07-02"));
    assert!(
        detail
            .markdown
            .contains("- **Show Time Now** — Says the time")
    );
    assert!(
        detail.markdown.contains("Press it."),
        "the README is appended"
    );

    assert!(
        !root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id.contains("store.vicinae.clock")),
        "not in root search before it is installed"
    );
    let Response::StoreInstalled { id, title } = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        owner: "zoe".into(),
        name: "clock".into(),
    }) else {
        panic!("not installed");
    };
    assert_eq!(
        (id.as_str(), title.as_str()),
        ("store.vicinae.clock", "Clock")
    );
    assert!(
        extensions
            .join("store.vicinae.clock/package.json")
            .is_file()
    );
    assert!(
        extensions
            .join("store.vicinae.clock/show-time.js")
            .is_file()
    );
    assert!(
        root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id == "@zoe/store.vicinae.clock:show-time"),
        "the installed command is in root search"
    );
    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert!(entries[0].installed && !entries[0].update_available);

    // A bundle whose entry climbs out is refused whole.
    let Response::Error(err) = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        owner: "mallory".into(),
        name: "evil".into(),
    }) else {
        panic!("a zip-slip bundle was installed");
    };
    assert!(
        err.message.contains("outside its directory"),
        "{}",
        err.message
    );
    assert!(!extensions.join("store.vicinae.evil").exists());
    assert!(!extensions.join("escaped.txt").exists());
    assert!(!data_home.join("escaped.txt").exists());

    assert!(matches!(
        daemon.request(Request::StoreUninstall {
            id: "store.vicinae.clock".into()
        }),
        Response::Ack
    ));
    assert!(!extensions.join("store.vicinae.clock").exists());
    assert!(
        !root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id.contains("store.vicinae.clock")),
        "gone from root search"
    );
    let Response::Error(err) = daemon.request(Request::StoreUninstall {
        id: "store.vicinae.clock".into(),
    }) else {
        panic!("uninstalled twice");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    let Response::Error(err) = daemon.request(Request::OpenUrl {
        url: "file:///etc/passwd".into(),
    }) else {
        panic!("a file URL was opened");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
}

/// GitHub is written by `thomaslombart` and owned by `raycast`, and the API
/// files it under the owner: asking for it by its author is a 404. The rows
/// carry both, the detail and the install go by the owner, and the author is
/// what is shown.
#[test]
fn an_organisations_raycast_extension_opens_and_installs_by_its_owner() {
    use compass_ipc::{Request, Response, StoreKind};
    let store = FakeStore::start();
    let daemon = store.start_engine();

    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: "github".into(),
    }) else {
        panic!("no search");
    };
    let row = entries.first().expect("GitHub is found");
    assert_eq!(row.id, "store.raycast.github");
    assert_eq!(row.owner, "raycast");
    assert_eq!(row.author, "thomaslombart");
    assert_eq!(row.author_name, "Thomas Lombart");

    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        owner: row.owner.clone(),
        name: row.name.clone(),
    }) else {
        panic!("the detail page did not open by the owner");
    };
    assert_eq!(detail.entry.owner, "raycast");
    assert!(
        detail.markdown.contains("Thomas Lombart"),
        "the author is shown"
    );

    let Response::StoreInstalled { id, title } = daemon.request(Request::StoreInstall {
        store: StoreKind::Raycast,
        owner: row.owner.clone(),
        name: row.name.clone(),
    }) else {
        panic!("not installed by the owner");
    };
    assert_eq!(
        (id.as_str(), title.as_str()),
        ("store.raycast.github", "GitHub")
    );
    assert!(
        root_ids(&daemon, "my pull requests")
            .iter()
            .any(|id| id == "@thomaslombart/store.raycast.github:my-pull-requests"),
        "the installed id is the one an install by author would have made"
    );

    let Response::Error(err) = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        owner: "thomaslombart".into(),
        name: "github".into(),
    }) else {
        panic!("the fake store must refuse the author, as the API does");
    };
    assert!(err.message.contains("github"), "{}", err.message);
}

/// `extensions.installed` (#321): the engine installs what the file declares
/// and is missing, in either form, when it starts and when the file changes;
/// it leaves an installed one alone, and removing an entry uninstalls
/// nothing.
#[test]
fn the_engine_installs_the_extensions_compass_json_declares() {
    use std::os::unix::fs::MetadataExt as _;
    let store = FakeStore::start();
    let daemon = store.start_engine_with_config(
        r#"{"extensions": {"installed": [
            "vicinae:zoe/clock", "store.raycast.github", "local.not-here", "brew:x/y"
        ]}}"#,
    );
    let extensions = daemon._dirs.path().join("data-home/compass/extensions");
    let config = daemon._dirs.path().join("config/compass/compass.json");
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "{what} never happened");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    wait_for("the declared extensions installing", &|| {
        extensions
            .join("store.vicinae.clock/package.json")
            .is_file()
            && extensions
                .join("store.raycast.github/package.json")
                .is_file()
    });
    wait_for("the installed command reaching root search", &|| {
        root_ids(&daemon, "show time now")
            .iter()
            .any(|id| id == "@zoe/store.vicinae.clock:show-time")
    });
    let clock = std::fs::metadata(extensions.join("store.vicinae.clock"))
        .unwrap()
        .ino();

    // A new entry is installed when the file changes; the one already
    // installed is not installed again.
    std::fs::write(
        &config,
        r#"{"extensions": {"installed": ["vicinae:zoe/clock", "raycast:ray/hn"]}}"#,
    )
    .unwrap();
    wait_for("the added extension installing", &|| {
        extensions.join("store.raycast.hn/package.json").is_file()
    });
    assert_eq!(
        std::fs::metadata(extensions.join("store.vicinae.clock"))
            .unwrap()
            .ino(),
        clock,
        "an installed extension is left alone"
    );

    // Taking everything out of the list uninstalls nothing.
    std::fs::write(&config, r#"{"extensions": {"installed": []}}"#).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    for id in [
        "store.vicinae.clock",
        "store.raycast.github",
        "store.raycast.hn",
    ] {
        assert!(extensions.join(id).join("package.json").is_file(), "{id}");
    }
}

#[test]
fn the_raycast_store_badges_compatibility_and_notices_an_update() {
    use compass_ipc::{ErrorKind, Request, Response, StoreKind};
    let store = FakeStore::start();
    let daemon = store.start_engine();

    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: String::new(),
    }) else {
        panic!("no listing");
    };
    assert_eq!(heading, "Extensions");
    assert_eq!(entries.len(), 1, "a macOS listing is offered on Linux");
    assert_eq!(entries[0].id, "store.raycast.hn");
    if cfg!(target_os = "linux") {
        assert_eq!(entries[0].compat, Some(1), "partial, from the sheet");
    }

    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        owner: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("no detail");
    };
    assert_eq!(
        detail.screenshots,
        [format!("{}/assets/metadata/hn-1.png", store.base)]
    );
    assert_eq!(
        detail.store_url.as_deref(),
        Some("https://www.raycast.com/ray/hn")
    );
    if cfg!(target_os = "linux") {
        assert!(detail.markdown.contains("works but has a few quirks"));
        assert!(detail.markdown.contains("No menu bar"));
    }

    let Response::StoreInstalled { id, .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Raycast,
        owner: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("not installed");
    };
    assert_eq!(id, "store.raycast.hn");
    assert!(
        root_ids(&daemon, "front page stories")
            .iter()
            .any(|id| id == "@ray/store.raycast.hn:front")
    );

    let search = || {
        let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
            store: StoreKind::Raycast,
            query: "hacker news".into(),
        }) else {
            panic!("no search");
        };
        assert_eq!(heading, "Results");
        entries.into_iter().next().expect("a result")
    };
    let current = search();
    assert!(current.installed && !current.update_available);
    *store.raycast_commit.lock().unwrap() = "c2".into();
    assert!(search().update_available, "a new commit is an update");

    let Response::StoreInstalled { .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Raycast,
        owner: "ray".into(),
        name: "hn".into(),
    }) else {
        panic!("not updated");
    };
    assert!(
        !search().update_available,
        "reinstalling applies the update"
    );

    let Response::Error(err) = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        owner: "ray".into(),
        name: "missing".into(),
    }) else {
        panic!("a missing extension had a page");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(err.message.contains("\"ray/missing\""), "{}", err.message);
}

/// The real stores, end to end: browse both, open a detail page, install a
/// Vicinae store extension into a temp data home and uninstall it. Network,
/// so ignored by default; run with
/// `cargo test -p compass --test engine_end_to_end -- --ignored real_stores`.
#[test]
#[ignore = "talks to api.vicinae.com and backend.raycast.com"]
fn real_stores_smoke() {
    use compass_ipc::{Request, Response, StoreKind};
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        // The invoking shell's proxy, which the harness otherwise strips.
        [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "NO_PROXY",
        ]
        .into_iter()
        .filter_map(|name| Some((name, std::env::var_os(name)?)))
        .collect()
    });

    let entries = match daemon.request(Request::StoreBrowse {
        store: StoreKind::Vicinae,
        query: String::new(),
    }) {
        Response::StoreListing { entries, .. } => entries,
        other => panic!("no Vicinae listing: {other:?}"),
    };
    assert!(entries.len() > 10, "{} extensions", entries.len());
    let first = entries[0].clone();
    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Vicinae,
        owner: first.owner.clone(),
        name: first.name.clone(),
    }) else {
        panic!("no Vicinae detail");
    };
    assert!(detail.markdown.contains(&first.title));

    let Response::StoreInstalled { id, .. } = daemon.request(Request::StoreInstall {
        store: StoreKind::Vicinae,
        owner: first.owner.clone(),
        name: first.name.clone(),
    }) else {
        panic!("not installed");
    };
    let installed = daemon
        ._dirs
        .path()
        .join("data-home/compass/extensions")
        .join(&id);
    assert!(installed.join("package.json").is_file());
    assert!(matches!(
        daemon.request(Request::StoreUninstall { id }),
        Response::Ack
    ));
    assert!(!installed.exists());

    let Response::StoreListing { entries, .. } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: String::new(),
    }) else {
        panic!("no Raycast listing");
    };
    assert!(!entries.is_empty());
    let Response::StoreListing { heading, entries } = daemon.request(Request::StoreBrowse {
        store: StoreKind::Raycast,
        query: "spotify".into(),
    }) else {
        panic!("no Raycast search");
    };
    assert_eq!(heading, "Results");
    let hit = entries.first().expect("a Raycast result").clone();
    let Response::StoreExtension { detail } = daemon.request(Request::StoreExtension {
        store: StoreKind::Raycast,
        owner: hit.owner,
        name: hit.name,
    }) else {
        panic!("no Raycast detail");
    };
    assert!(detail.markdown.starts_with("# "));
}

/// The input server is started, told every snippet keyword, told again as
/// snippets change, and stopped and started by `SetInputServerEnabled` —
/// against a scripted helper that logs what it is asked and touches no
/// device.
#[test]
fn the_input_server_is_told_the_keywords_and_follows_the_setting() {
    use compass_ipc::{Request, Response};

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_input_server.py");
    let log = std::sync::Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let log_path = std::sync::Arc::clone(&log);
    let daemon = Daemon::start_prepared(&[], "{}", move |root| {
        let file = root.join("input-server.log");
        *log_path.lock().unwrap() = file.clone();
        vec![
            ("COMPASS_INPUT_SERVER_BIN", script.into_os_string()),
            ("FAKE_INPUT_LOG", file.into_os_string()),
        ]
    });
    let log = log.lock().unwrap().clone();
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    let wait_for = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if calls().contains(what) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "the input server was never sent {what}; it got:\n{}",
            calls()
        );
    };
    let status = |daemon: &Daemon| match daemon.request(Request::InputServerStatus) {
        Response::InputServerStatus(status) => status,
        other => panic!("unexpected {other:?}"),
    };

    wait_for("Snippet/getCapabilities");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !status(&daemon).running {
        assert!(
            Instant::now() < deadline,
            "never running: {:?}",
            status(&daemon)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let now = status(&daemon);
    assert!(now.enabled && now.injection, "{now:?}");

    let Response::Snippets { snippets } = daemon.request(Request::SaveSnippet {
        id: None,
        name: "Sig".into(),
        text: "Best".into(),
        keyword: Some(";sig".into()),
        word: true,
        apps: vec![],
    }) else {
        panic!("not saved");
    };
    wait_for(r#""trigger": ";sig""#);
    assert!(
        calls().contains(r#""mode": "Word""#),
        "registered as a word snippet: {}",
        calls()
    );
    assert_eq!(status(&daemon).keywords, 1);

    daemon.request(Request::RemoveSnippet {
        id: snippets[0].id.clone(),
    });
    wait_for("Snippet/removeSnippet");

    let Response::InputServerStatus(off) =
        daemon.request(Request::SetInputServerEnabled { enabled: false })
    else {
        panic!("no status");
    };
    assert!(!off.enabled && !off.running, "{off:?}");
    let saved =
        std::fs::read_to_string(log.parent().unwrap().join("config/compass/compass.json")).unwrap();
    assert!(saved.contains("\"input_server\""), "{saved}");

    let before = calls().matches("Snippet/getCapabilities").count();
    let Response::InputServerStatus(on) =
        daemon.request(Request::SetInputServerEnabled { enabled: true })
    else {
        panic!("no status");
    };
    assert!(on.enabled, "{on:?}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while calls().matches("Snippet/getCapabilities").count() == before {
        assert!(Instant::now() < deadline, "not restarted");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A second extension, `hosts`, whose commands reach the host APIs the
/// engine routes since IPC v15: `FileSearch`, `Wallpaper`, `BrowserExtension`,
/// workspaces through `WindowManagement`, and `Command`. Each writes what it
/// was answered into its support directory (the one place the sandbox lets
/// it write) or shows it in a `Detail`.
fn install_host_extension(root: &Path) -> PathBuf {
    let ext = root.join("data-home/compass/extensions/hosts");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "hosts", "title": "Hosts", "author": "someone",
            "commands": [
              {"name": "probe", "title": "Probe Hosts", "mode": "view"},
              {"name": "launcher", "title": "Launch Sibling", "mode": "no-view"},
              {"name": "target", "title": "Launch Target", "mode": "no-view",
               "arguments": [{"name": "who", "type": "text", "placeholder": "Who",
                              "required": false}]},
              {"name": "badlaunch", "title": "Launch Nothing", "mode": "no-view"},
              {"name": "prefs", "title": "Open Preferences", "mode": "no-view"}
            ]}"#,
    )
    .unwrap();
    let support = root.join("data-home/compass/support/hosts");
    let wall = root.join("wall.png");
    std::fs::write(&wall, b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(
        ext.join("probe.js"),
        format!(
            "const React = require('react');
             const {{ Detail, FileSearch, Wallpaper, BrowserExtension, WindowManagement,
                      environment }} = require('@vicinae/api');
             const said = (p) => p.then((v) => v === undefined ? 'ok' : JSON.stringify(v),
                                        (e) => 'error: ' + (e && e.message ? e.message : String(e)));
             module.exports.default = () => {{
               const [text, setText] = React.useState('');
               React.useEffect(() => {{
                 Promise.all([
                   Promise.resolve(JSON.stringify([FileSearch, Wallpaper, BrowserExtension,
                     WindowManagement].map((api) => environment.canAccess(api)))),
                   said(FileSearch.search('quarterly', {{ limit: 5 }})),
                   said(Wallpaper.set({wall:?}, {{ fit: 'Contain' }})),
                   said(Wallpaper.set('/nonexistent/wall.png')),
                   said(BrowserExtension.getTabs()),
                   said(WindowManagement.getWorkspaces()),
                   said(WindowManagement.getActiveWorkspace()),
                   said(WindowManagement.getActiveWindow().then((w) => w.id)),
                 ]).then((answers) => setText(answers.join(' | ')));
               }}, []);
               return React.createElement(Detail, {{ markdown: text || 'asking' }});
             }};",
            wall = wall.to_string_lossy()
        ),
    )
    .unwrap();
    let launched = support.join("launched.txt");
    std::fs::write(
        ext.join("launcher.js"),
        format!(
            "const {{ launchCommand, updateCommandMetadata, LaunchType }} = require('@vicinae/api');
             module.exports.default = async () => {{
               await updateCommandMetadata({{ subtitle: '3 unread' }});
               let out = 'launched';
               try {{
                 await launchCommand({{ name: 'target', type: LaunchType.UserInitiated,
                   arguments: {{ who: 'ada' }}, context: {{ from: 'launcher' }} }});
               }} catch (e) {{ out = 'error: ' + ((e && e.message) || String(e)); }}
               require('node:fs').writeFileSync({launched:?}, out);
             }};",
            launched = launched.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("target.js"),
        format!(
            "module.exports.default = async (props) => {{
               require('node:fs').writeFileSync({target:?},
                 JSON.stringify({{ who: props.arguments.who, context: props.launchContext }}));
             }};",
            target = support.join("target.txt").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("badlaunch.js"),
        format!(
            "const {{ launchCommand, LaunchType }} = require('@vicinae/api');
             module.exports.default = async () => {{
               let out = 'launched';
               try {{
                 await launchCommand({{ name: 'nothing', type: LaunchType.UserInitiated }});
               }} catch (e) {{ out = 'error: ' + ((e && e.message) || String(e)); }}
               require('node:fs').writeFileSync({bad:?}, out);
             }};",
            bad = support.join("bad.txt").to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        ext.join("prefs.js"),
        format!(
            "const {{ openCommandPreferences }} = require('@vicinae/api');
             module.exports.default = async () => {{
               await openCommandPreferences();
               require('node:fs').writeFileSync({opened:?}, 'opened');
             }};",
            opened = support.join("prefs.txt").to_string_lossy()
        ),
    )
    .unwrap();
    support
}

/// A directory of stand-in programs for the wallpaper backends: `gsettings`
/// records its arguments, and `hyprctl`, `swww` and `awww` fail, so a real
/// one on the invoking machine's path is never reached.
fn fake_wallpaper_programs(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("gsettings.log");
    let scripts = [
        (
            "gsettings",
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
        ),
        ("hyprctl", "#!/bin/sh\nexit 1\n".to_owned()),
        ("swww", "#!/bin/sh\nexit 1\n".to_owned()),
        ("awww", "#!/bin/sh\nexit 1\n".to_owned()),
    ];
    for (name, body) in scripts {
        let path = bin.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn hyprland_fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../compass-platform-linux/tests/fixtures/hyprland")
            .join(name),
    )
    .unwrap()
}

#[test]
fn an_extension_searches_files_sets_the_wallpaper_and_sees_hyprland_workspaces() {
    use compass_extension_api::View;
    use compass_ipc::{Request, Response};
    use compass_testkit::fake_compositor::{FakeSocket, Framing};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = PathBuf::new();
    let mut hyprland = None;
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        root = dir.to_path_buf();
        install_host_extension(dir);
        let documents = dir.join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join("quarterly-report.pdf"), "%PDF").unwrap();
        let runtime_dir = dir.join("run");
        hyprland = Some(FakeSocket::replaying(
            &FakeSocket::hyprland_path(&runtime_dir, "v0.50_test"),
            Framing::Hyprland,
            vec![
                ("-j/clients".into(), hyprland_fixture("clients.json")),
                ("-j/workspaces".into(), hyprland_fixture("workspaces.json")),
                (
                    "-j/activeworkspace".into(),
                    hyprland_fixture("activeworkspace.json"),
                ),
                (
                    "-j/activewindow".into(),
                    hyprland_fixture("activewindow.json"),
                ),
            ],
        ));
        let bin = fake_wallpaper_programs(dir);
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&path));
        vec![
            ("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string()),
            ("XDG_CURRENT_DESKTOP", "GNOME".into()),
            ("XDG_RUNTIME_DIR", runtime_dir.into_os_string()),
            ("HYPRLAND_INSTANCE_SIGNATURE", "v0.50_test".into()),
            ("PATH", std::env::join_paths(paths).unwrap()),
        ]
    });
    // The index scans in the background; the extension asks once it has.
    search_files_until(&daemon, "quarterly", None, |_, files| {
        files.iter().any(|file| file.name == "quarterly-report.pdf")
    });

    let started = daemon.request(Request::RunExtensionCommand {
        id: "@someone/hosts:probe".into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    let (view, _) = wait_for_view(
        &daemon,
        session,
        |view, _| matches!(view, View::Detail(detail) if detail.markdown.as_deref() != Some("asking")),
    );
    let View::Detail(detail) = view else {
        unreachable!()
    };
    let text = detail.markdown.unwrap_or_default();
    let answers: Vec<&str> = text.split(" | ").collect();
    assert_eq!(answers.len(), 8, "{text}");
    assert_eq!(
        answers[0], "[true,true,false,true]",
        "canAccess: files, wallpaper, no browser, windows"
    );
    let files: serde_json::Value = serde_json::from_str(answers[1]).expect(answers[1]);
    assert_eq!(files[0]["category"], "Document", "{files}");
    assert!(
        files[0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("/Documents/quarterly-report.pdf")),
        "{files}"
    );
    assert_eq!(answers[2], "ok", "the wallpaper was set");
    assert_eq!(answers[3], "error: No such file: /nonexistent/wall.png");
    assert_eq!(answers[4], "[]", "no browser is ever connected");
    let workspaces: serde_json::Value = serde_json::from_str(answers[5]).expect(answers[5]);
    assert_eq!(
        workspaces
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["id"].as_str().unwrap(), w["active"].as_bool().unwrap()))
            .collect::<Vec<_>>(),
        [("1", true), ("3", false)]
    );
    let active: serde_json::Value = serde_json::from_str(answers[6]).expect(answers[6]);
    assert_eq!(active["id"], "1");
    assert_eq!(active["monitorId"], "DP-1");
    assert_eq!(
        answers[7], "\"0x5581c8a4f310\"",
        "Hyprland's frontmost window"
    );

    let wall = root.join("wall.png");
    let log = std::fs::read_to_string(root.join("gsettings.log")).unwrap();
    let uri = format!("file://{}", wall.display());
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            format!("set org.gnome.desktop.background picture-uri {uri}"),
            format!("set org.gnome.desktop.background picture-uri-dark {uri}"),
            "set org.gnome.desktop.background picture-options scaled".to_owned(),
        ],
        "GNOME's three keys, Contain as `scaled`; the missing file set nothing"
    );
    assert!(
        hyprland
            .as_ref()
            .unwrap()
            .seen()
            .iter()
            .all(|request| request.starts_with("-j/")),
        "WindowManagement only asked"
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );
}

#[test]
fn an_extension_launches_a_sibling_relabels_itself_and_opens_its_preferences() {
    use compass_ipc::{ErrorKind, Request, Response, WindowCommand};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut support = PathBuf::new();
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |dir| {
        support = install_host_extension(dir);
        vec![("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string())]
    });
    let run = |id: &str| {
        daemon.request(Request::RunExtensionCommand {
            id: id.into(),
            arguments_json: None,
        })
    };

    // No window: a no-view sibling runs in the engine, with its arguments
    // and the launch context.
    assert_eq!(run("@someone/hosts:launcher"), Response::Ack);
    wait_for_content(&support.join("launched.txt"));
    assert_eq!(
        std::fs::read_to_string(support.join("launched.txt")).unwrap(),
        "launched"
    );
    wait_for_content(&support.join("target.txt"));
    let target: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(support.join("target.txt")).unwrap())
            .unwrap();
    assert_eq!(
        target,
        serde_json::json!({"who": "ada", "context": {"from": "launcher"}})
    );

    // The subtitle it set shows in root search and is served to the window.
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "launch sibling".into(),
    }) else {
        panic!("no results");
    };
    let hit = hits
        .iter()
        .find(|hit| hit.id == "@someone/hosts:launcher")
        .expect("the launcher command");
    assert_eq!(hit.subtitle.as_deref(), Some("3 unread"));
    assert_eq!(
        daemon.request(Request::ExtensionSubtitles),
        Response::ExtensionSubtitles {
            subtitles: vec![("@someone/hosts:launcher".into(), "3 unread".into())]
        }
    );

    // A command that is not installed is the C++'s own refusal.
    assert_eq!(run("@someone/hosts:badlaunch"), Response::Ack);
    wait_for_content(&support.join("bad.txt"));
    assert_eq!(
        std::fs::read_to_string(support.join("bad.txt")).unwrap(),
        "error: No such command"
    );

    // With a window: it is handed the launch, which it takes once.
    std::fs::remove_file(support.join("launched.txt")).unwrap();
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    assert_eq!(run("@someone/hosts:launcher"), Response::Ack);
    wait_for_content(&support.join("launched.txt"));
    let token = wait_for_launch(&window, 0);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::ExtensionLaunch {
            id: "@someone/hosts:target".into(),
            arguments_json: Some(r#"{"who":"ada"}"#.into()),
            preferences: false,
        }
    );
    let Response::Error(err) = daemon.request(Request::ExtensionLaunchFetch { token }) else {
        panic!("a launch was taken twice");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    assert_eq!(run("@someone/hosts:prefs"), Response::Ack);
    wait_for_content(&support.join("prefs.txt"));
    let token = wait_for_launch(&window, 1);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::ExtensionLaunch {
            id: "@someone/hosts:prefs".into(),
            arguments_json: None,
            preferences: true,
        }
    );
    assert!(window.seen().contains(&WindowCommand::Launch(token)));
    // Without a keyring there is nowhere to keep preferences, which is said.
    let Response::Error(err) = daemon.request(Request::ExtensionPreferences {
        id: "@someone/hosts:prefs".into(),
    }) else {
        panic!("preferences without a keyring were not refused");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

/// Waits for the window to have been handed more than `after` launches, and
/// returns the token of the next one.
fn wait_for_launch(window: &FakeWindow, after: usize) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let launches: Vec<u64> = window
            .seen()
            .iter()
            .filter_map(|command| match command {
                compass_ipc::WindowCommand::Launch(token) => Some(*token),
                _ => None,
            })
            .collect();
        if let Some(token) = launches.get(after) {
            return *token;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the window was never handed a launch: {:?}", window.seen());
}

// --- the rest of the C++ CLI: cmd, app, state, logs, fs, server ------------

/// A fake application that appends `launched` and then each argument to
/// `<dir>/<name>.log`, and its desktop entry (`%U`, so every argument).
fn logging_app(dir: &Path, name: &str) -> (String, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join(format!("{name}.log"));
    let program = dir.join(name);
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\necho launched >> '{log}'\nfor arg in \"$@\"; do echo \"$arg\" >> '{log}'; done\n",
            log = log.display()
        ),
    )
    .expect("fake application");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={} %U\n",
        program.display()
    );
    (entry, log)
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cmd_ls_lists_every_root_item_by_id_and_cmd_launch_hands_the_window_a_launch() {
    use compass_ipc::{Request, Response, WindowCommand};
    let daemon = Daemon::start(&[
        ("zeta.desktop", &entry("Zeta Editor", "")),
        ("alpha.desktop", &entry("Alpha", "")),
    ]);

    let ids: Vec<String> = daemon
        .client(&["cmd", "ls"])
        .lines()
        .map(str::to_owned)
        .collect();
    assert!(ids.contains(&"applications:alpha".to_owned()), "{ids:?}");
    assert!(ids.contains(&"applications:zeta".to_owned()));
    assert!(ids.contains(&"commands:clipboard-history".to_owned()));
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "sorted by id, as the C++ sorts them");

    let listed: Vec<serde_json::Value> =
        serde_json::from_str(&daemon.client(&["command", "list", "--json"])).unwrap();
    assert_eq!(listed.len(), ids.len());
    assert!(
        listed.contains(&serde_json::json!({"id": "applications:zeta", "name": "Zeta Editor"}))
    );

    // Checked before anything is launched, in the C++'s words.
    for (args, expected) in [
        (
            &["cmd", "launch", "nothing"][..],
            "Ill-formed command entrypoint: nothing",
        ),
        (
            &["cmd", "launch", "commands:nope"][..],
            "Unknown command entrypoint: commands:nope",
        ),
        (
            &["cmd", "launch", "commands:search-files", "extra"][..],
            "Too many arguments: expected at most 0, got 1",
        ),
    ] {
        let out = daemon.try_client(args);
        assert!(!out.status.success(), "{args:?}");
        let err = stderr_of(&out);
        assert!(err.contains("Failed to launch command"), "{err}");
        assert!(err.contains(expected), "{args:?}: {err}");
    }

    // With no window, a builtin has nowhere to open.
    let out = daemon.try_client(&["cmd", "launch", "commands:search-files"]);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("no launcher window"),
        "{}",
        stderr_of(&out)
    );

    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    daemon.client(&["cmd", "launch", "commands:search-files", "-q", "report"]);
    let token = wait_for_launch(&window, 0);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::CommandLaunch {
            id: "commands:search-files".into(),
            arguments_json: None,
            fallback_text: Some("report".into()),
        }
    );
    daemon.client(&["cmd", "launch", "commands:clipboard-history"]);
    let token = wait_for_launch(&window, 1);
    assert_eq!(
        daemon.request(Request::ExtensionLaunchFetch { token }),
        Response::ExtensionLaunch {
            id: "commands:clipboard-history".into(),
            arguments_json: None,
            preferences: false,
        }
    );
    assert_eq!(
        window
            .seen()
            .iter()
            .filter(|c| matches!(c, WindowCommand::Launch(_)))
            .count(),
        2
    );
}

#[test]
fn app_launch_and_cmd_launch_start_the_application_with_its_arguments() {
    let bin = TempDir::new().expect("tempdir");
    let (recorder, log) = logging_app(bin.path(), "recorder");
    let (other, other_log) = logging_app(bin.path(), "other");
    let daemon = Daemon::start(&[("recorder.desktop", &recorder), ("other.desktop", &other)]);

    // No window manager here, so nothing to focus: it launches, as the C++
    // does with its dummy provider.
    let out = daemon.client(&["app", "launch", "recorder.desktop", "/tmp/a b", "--", "-x"]);
    assert_eq!(out, "", "nothing was focused, so nothing is said");
    assert_eq!(launched_with(&log), ["launched", "/tmp/a b", "-x"]);

    // The root id works too, and so does `cmd launch` on an application.
    daemon.client(&["cmd", "launch", "applications:other"]);
    assert_eq!(launched_with(&other_log), ["launched"]);

    let out = daemon.try_client(&["app", "launch", "missing.desktop"]);
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(err.contains("Failed to launch app"), "{err}");
    assert!(err.contains("No app with id"), "{err}");
}

#[test]
fn state_open_asks_the_window_and_exits_by_the_answer() {
    use compass_ipc::{WindowCommand, WindowOutcome};
    let daemon = Daemon::start(&[]);
    assert_eq!(
        daemon.try_client(&["state", "open"]).status.code(),
        Some(1),
        "no window is not an open window"
    );

    let shown = FakeWindow::attach(&daemon.socket, WindowOutcome::Shown);
    let out = daemon.try_client(&["state", "open"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(shown.seen(), vec![WindowCommand::Describe]);
    drop(shown);

    let hidden = FakeWindow::attach(&daemon.socket, WindowOutcome::Hidden);
    assert_eq!(daemon.try_client(&["state", "open"]).status.code(), Some(1));
    assert_eq!(hidden.seen(), vec![WindowCommand::Describe]);
}

/// Runs the client with the daemon's own home, for the commands that read
/// files rather than ask the engine.
fn client_at_home(daemon: &Daemon, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .args(args)
        .env("HOME", daemon._dirs.path())
        .env_remove("XDG_STATE_HOME")
        .output()
        .expect("run the client")
}

#[test]
fn the_engine_keeps_a_log_file_and_logs_prints_its_last_lines() {
    let dirs = TempDir::new().unwrap();
    let out = Command::new(binary())
        .args(["logs"])
        .env("HOME", dirs.path())
        .env_remove("XDG_STATE_HOME")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr_of(&out).contains("Has the server been started yet?"));

    let daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let log = daemon._dirs.path().join(".local/state/compass/compass.log");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while std::fs::metadata(&log).map_or(0, |m| m.len()) == 0 {
        assert!(Instant::now() < deadline, "the engine never wrote its log");
        std::thread::sleep(Duration::from_millis(25));
    }
    let written = std::fs::read_to_string(&log).unwrap();
    let last = written.lines().last().unwrap().to_owned();
    assert!(!last.contains('\u{1b}'), "no colours in the file: {last}");

    let out = client_at_home(&daemon, &["logs", "-n", "1"]);
    assert!(out.status.success(), "{}", stderr_of(&out));
    let printed = String::from_utf8(out.stdout).unwrap();
    assert_eq!(printed.lines().count(), 1, "{printed}");
    // Read again: the engine may have written more since.
    let now = std::fs::read_to_string(&log).unwrap();
    assert!(
        now.lines().any(|line| line == printed.trim_end()),
        "{printed}"
    );
}

#[test]
fn fs_query_asks_the_index_alone_and_names_categories_as_the_cpp_does() {
    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |root| {
        let documents = root.join("Documents");
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::write(documents.join("quarterly-report.pdf"), "%PDF").unwrap();
        std::fs::write(documents.join("quarterly-chart.png"), "png").unwrap();
        Vec::new()
    });
    let out = daemon.try_client(&["fs", "query", "ab"]);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("at least 3 characters"));

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let rows = loop {
        let out = daemon.try_client(&["fs", "q", "quarterly", "-c", "document", "--json"]);
        if out.status.success() {
            let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
            if !rows.is_empty() {
                break rows;
            }
        }
        assert!(Instant::now() < deadline, "{}", stderr_of(&out));
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        rows[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("/Documents/quarterly-report.pdf")
    );
    assert!(
        rows.iter().all(|row| row["category"] == "document"),
        "{rows:?}"
    );

    let plain = daemon.client(&["fs", "query", "quarterly", "-n", "1"]);
    assert_eq!(plain.lines().count(), 1, "{plain}");
}

#[test]
fn server_refuses_a_running_engine_and_replace_kills_it_and_serves_in_its_place() {
    use compass_ipc::{Request, Response};
    let mut daemon = Daemon::start(&[("alpha.desktop", &entry("Alpha", ""))]);
    let first = daemon.child.id();

    let out = daemon.try_client(&["server"]);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains(&format!(
            "A server is already running (pid {first}). Pass --replace to replace it."
        )),
        "{}",
        stderr_of(&out)
    );

    // The replacement reads the configuration it is given.
    let config = daemon._dirs.path().join("elsewhere.json");
    std::fs::write(&config, r#"{"launcher": {"max_results": 1}}"#).unwrap();
    let dirs = daemon._dirs.path();
    let mut server = Command::new(binary())
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("server")
        .arg("--replace")
        .arg("--config")
        .arg(&config)
        .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
        .env("XDG_DATA_DIRS", dirs.join("data"))
        .env("XDG_DATA_HOME", dirs.join("data-home"))
        .env("XDG_CONFIG_HOME", dirs.join("config"))
        .env("XDG_CACHE_HOME", dirs.join(".cache"))
        .env("HOME", dirs)
        .env_remove("XDG_STATE_HOME")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .env_remove("NIRI_SOCKET")
        .env_remove("XDG_CURRENT_DESKTOP")
        .env("COMPASS_INPUT_SERVER_BIN", "/bin/true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The engine this test started is killed: it exits on a signal.
    let status = daemon.child.wait().unwrap();
    assert!(!status.success(), "{status:?}");

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let ping = |socket: &Path| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut client = compass_ipc::Client::connect(socket).await.ok()?;
                match client.request(Request::Ping).await.ok()? {
                    Response::Pong { pid, .. } => Some(pid),
                    _ => None,
                }
            })
    };
    let pid = loop {
        if let Some(pid) = ping(&daemon.socket) {
            break pid;
        }
        assert!(Instant::now() < deadline, "the replacement never answered");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(pid, server.id(), "the server became the engine itself");
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: String::new(),
    }) else {
        panic!("expected results");
    };
    assert_eq!(hits.len(), 1, "launcher.max_results from --config");

    let _ = server.kill();
    let _ = server.wait();
}

/// A `sleep` this test owns, to stand behind a window: the only processes
/// Force Quit may ever be pointed at here.
fn sleeper() -> Child {
    Command::new("sleep")
        .arg("600")
        .spawn()
        .expect("spawn sleep")
}

/// Whether `child` was ended by `SIGKILL`, waiting for it to be.
fn killed(child: &mut Child) -> bool {
    use std::os::unix::process::ExitStatusExt;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status.signal() == Some(9);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Asks `request` until the engine, which reaches the Shell extension in the
/// background, answers something `done` accepts.
fn until_answered(
    daemon: &Daemon,
    request: &compass_ipc::Request,
    done: impl Fn(&compass_ipc::Response) -> bool,
) -> compass_ipc::Response {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let response = daemon.request(request.clone());
        if done(&response) {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "the engine never reached the extension: {response:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn on_gnome_switch_workspaces_lists_and_switches_through_the_shell_extension() {
    use compass_ipc::{Request, Response};
    use shell_mock::{MockOptions, MockShell, MockWindow, MockWorkspace};
    let Some(bus) = shell_bus::start_or_skip(
        "on_gnome_switch_workspaces_lists_and_switches_through_the_shell_extension",
    ) else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let shell = runtime
        .block_on(MockShell::start(bus.address(), MockOptions::default()))
        .expect("the mock shell");
    shell.set_workspaces(vec![
        MockWorkspace::new(0, "Mail").active(),
        MockWorkspace::new(1, ""),
    ]);
    shell.set_windows(vec![
        MockWindow::new(1, "firefox", "Inbox").focused(),
        MockWindow::new(2, "firefox", "Docs"),
        MockWindow {
            workspace: Some(1),
            ..MockWindow::new(3, "unknown-app", "Elsewhere")
        },
    ]);
    let address = bus.address().to_owned();
    let daemon = Daemon::start_prepared(
        &[(
            "firefox.desktop",
            &entry("Firefox", "StartupWMClass=firefox\nIcon=firefox\n"),
        )],
        "{}",
        move |_| vec![("DBUS_SESSION_BUS_ADDRESS", address.into())],
    );

    let caps = until_answered(
        &daemon,
        &Request::WindowManagerCapabilities,
        |r| matches!(r, Response::WindowManagerCapabilities(caps) if caps.workspaces),
    );
    let Response::WindowManagerCapabilities(caps) = caps else {
        unreachable!()
    };
    assert!(
        !caps.fullscreen && !caps.floating && !caps.overview,
        "GNOME offers Switch Workspaces only: {caps:?}"
    );

    let Response::Workspaces { workspaces } = daemon.request(Request::ListWorkspaces) else {
        panic!("no workspaces");
    };
    assert_eq!(
        workspaces
            .iter()
            .map(|w| (w.id.as_str(), w.name.as_str(), w.window_count, w.active))
            .collect::<Vec<_>>(),
        [("0", "Mail", 2, true), ("1", "2", 1, false)],
        "named by GNOME, else counted from one"
    );
    assert_eq!(
        workspaces[0]
            .apps
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Firefox"],
        "each application once"
    );

    assert_eq!(
        daemon.request(Request::FocusWorkspace { id: "1".into() }),
        Response::Ack
    );
    assert_eq!(shell.calls(), [("ActivateWorkspace", 1)]);
    let Response::Workspaces { workspaces } = daemon.request(Request::ListWorkspaces) else {
        panic!("no workspaces");
    };
    assert!(workspaces[1].active && !workspaces[0].active);
    drop(daemon);
}

#[test]
fn an_extension_a_release_behind_offers_no_switch_workspaces_and_says_to_update() {
    use compass_ipc::{ErrorKind, Request, Response};
    use shell_mock::{MockOptions, MockShell, MockWindow};
    let Some(bus) = shell_bus::start_or_skip(
        "an_extension_a_release_behind_offers_no_switch_workspaces_and_says_to_update",
    ) else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let shell = runtime
        .block_on(MockShell::start(
            bus.address(),
            MockOptions {
                version: 3,
                ..MockOptions::default()
            },
        ))
        .expect("the mock shell");
    shell.set_windows(vec![MockWindow::new(1, "firefox", "Inbox").focused()]);
    let address = bus.address().to_owned();
    let daemon = Daemon::start_prepared(&[], "{}", move |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.into())]
    });

    // Window switching still works with the older extension.
    until_answered(
        &daemon,
        &Request::ListWindows,
        |r| matches!(r, Response::Windows { windows } if windows.len() == 1),
    );
    assert_eq!(
        daemon.request(Request::WindowManagerCapabilities),
        Response::WindowManagerCapabilities(compass_ipc::WindowManagerCapabilities::default())
    );
    match daemon.request(Request::ListWorkspaces) {
        Response::Error(err) => {
            assert_eq!(err.kind, ErrorKind::Unsupported);
            assert!(
                err.message.contains("Update the extension"),
                "{}",
                err.message
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(shell.calls().is_empty());
    drop(daemon);
}

#[test]
fn quit_closes_an_applications_windows_and_force_quit_kills_their_processes() {
    use compass_ipc::{ErrorKind, Request, Response};
    use shell_mock::{MockOptions, MockShell, MockWindow};
    let Some(bus) = shell_bus::start_or_skip(
        "quit_closes_an_applications_windows_and_force_quit_kills_their_processes",
    ) else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let shell = runtime
        .block_on(MockShell::start(bus.address(), MockOptions::default()))
        .expect("the mock shell");
    let (mut first, mut second, mut bystander) = (sleeper(), sleeper(), sleeper());
    let window = |id: u32, class: &str, title: &str, pid: Option<u32>| MockWindow {
        pid,
        ..MockWindow::new(id, class, title)
    };
    let windows = vec![
        window(1, "recorder", "Recording one", Some(first.id())),
        window(2, "recorder", "Recording two", Some(first.id())),
        window(3, "recorder", "A pidless recording", None),
        window(4, "Recorder", "Recording three", Some(second.id())),
        window(5, "bystander", "Not this one", Some(bystander.id())).focused(),
    ];
    shell.set_windows(windows.clone());

    let bin = TempDir::new().unwrap();
    let (recorder, _) = logging_app(bin.path(), "recorder");
    let (other, _) = logging_app(bin.path(), "other");
    let address = bus.address().to_owned();
    let daemon = Daemon::start_prepared(
        &[("recorder.desktop", &recorder), ("other.desktop", &other)],
        "{}",
        move |_| vec![("DBUS_SESSION_BUS_ADDRESS", address.into())],
    );

    // The engine reaches the extension in the background.
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let (running, frontmost, listed) = loop {
        if let Response::AppRuntime {
            running,
            frontmost,
            windows,
        } = daemon.request(Request::AppRuntime {
            id: "recorder.desktop".into(),
        }) && running
        {
            break (running, frontmost, windows);
        }
        assert!(
            Instant::now() < deadline,
            "the engine never saw the windows"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(running);
    assert!(!frontmost, "the focused window is the bystander's");
    assert_eq!(
        listed.iter().map(|w| w.id).collect::<Vec<_>>(),
        [1, 2, 3, 4],
        "every window of its class, and only those"
    );
    let Response::AppRuntime { running, .. } = daemon.request(Request::AppRuntime {
        id: "other.desktop".into(),
    }) else {
        panic!("expected an answer");
    };
    assert!(!running);

    // Quit: every window closed, no process touched.
    assert_eq!(
        daemon.request(Request::QuitApp {
            id: "recorder.desktop".into(),
            force: false,
        }),
        Response::Ack
    );
    let mut closed: Vec<u32> = shell
        .calls()
        .into_iter()
        .filter(|(call, _)| *call == "CloseWindow")
        .map(|(_, id)| id)
        .collect();
    closed.sort_unstable();
    assert_eq!(closed, [1, 2, 3, 4]);
    for child in [&mut first, &mut second, &mut bystander] {
        assert!(child.try_wait().unwrap().is_none(), "Quit killed nothing");
    }

    // Force Quit: each owning process killed once, the pidless window closed,
    // the bystander left alone.
    shell.set_windows(windows.clone());
    assert_eq!(
        daemon.request(Request::QuitApp {
            id: "recorder.desktop".into(),
            force: true,
        }),
        Response::Ack
    );
    assert!(killed(&mut first), "the first window's process");
    assert!(killed(&mut second), "the other window's process");
    assert_eq!(
        shell
            .calls()
            .into_iter()
            .filter(|(call, _)| *call == "CloseWindow")
            .map(|(_, id)| id)
            .skip(4)
            .collect::<Vec<_>>(),
        [3],
        "only the window with no process is closed"
    );
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "the bystander lives"
    );

    // From the window switcher: the application the window belongs to.
    let mut third = sleeper();
    shell.set_windows(vec![
        window(6, "recorder", "Again", Some(third.id())),
        window(5, "bystander", "Not this one", Some(bystander.id())),
    ]);
    assert_eq!(
        daemon.request(Request::QuitWindowApp {
            window: 6,
            force: true
        }),
        Response::Ack
    );
    assert!(killed(&mut third));
    let Response::Error(err) = daemon.request(Request::QuitWindowApp {
        window: 5,
        force: true,
    }) else {
        panic!("a window of no known application was quit");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(bystander.try_wait().unwrap().is_none());

    // Nothing to quit, and nothing by that name, are both said.
    let Response::Error(err) = daemon.request(Request::QuitApp {
        id: "other.desktop".into(),
        force: false,
    }) else {
        panic!("quitting what does not run succeeded");
    };
    assert_eq!(err.message, "Failed to quit other");
    let Response::Error(err) = daemon.request(Request::QuitApp {
        id: "missing.desktop".into(),
        force: true,
    }) else {
        panic!("an unknown application was quit");
    };
    assert_eq!(err.kind, ErrorKind::BadRequest);

    let _ = bystander.kill();
    let _ = bystander.wait();
    drop(daemon);
    runtime.block_on(shell.shutdown());
}

#[test]
fn calculator_history_is_a_builtin_and_without_a_keyring_is_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[]);
    let Response::QueryResults { hits } = daemon.request(Request::Query {
        text: "calculator history".into(),
    }) else {
        panic!("expected results");
    };
    assert_eq!(
        hits.first().map(|hit| hit.id.as_str()),
        Some("commands:calculator-history")
    );
    let Response::Error(err) = daemon.request(Request::CalculatorHistory {
        query: String::new(),
    }) else {
        panic!("the history opened without a keyring");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
    assert!(err.message.contains("keyring"), "{}", err.message);
}

/// A fake application's tray icon on a private bus: a `StatusNotifierItem`
/// at a non-default path and its `dbusmenu`, recording what is called.
mod fake_tray {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use zbus::zvariant::{OwnedValue, Value};

    pub type Calls = Arc<Mutex<Vec<String>>>;

    /// The specification's `(sa(iiay)ss)`: icon name, pixmaps, title, body.
    type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

    pub struct Item {
        pub calls: Calls,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Item {
        fn activate(&self, _x: i32, _y: i32) {
            self.calls.lock().unwrap().push("Activate".into());
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {
            self.calls.lock().unwrap().push("SecondaryActivate".into());
        }
        #[zbus(property)]
        fn category(&self) -> String {
            "Communications".into()
        }
        #[zbus(property)]
        fn id(&self) -> String {
            "fake-chat".into()
        }
        #[zbus(property)]
        fn title(&self) -> String {
            "Fake Chat".into()
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "NeedsAttention".into()
        }
        #[zbus(property)]
        fn icon_name(&self) -> String {
            "fake-chat-icon".into()
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
            zbus::zvariant::OwnedObjectPath::try_from("/FakeMenu").unwrap()
        }
        #[zbus(property)]
        fn tool_tip(&self) -> ToolTip {
            (
                String::new(),
                Vec::new(),
                "Fake Chat".into(),
                "3 unread messages".into(),
            )
        }
    }

    pub struct Menu {
        pub calls: Calls,
    }

    type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    fn node(id: i32, props: &[(&str, Value<'_>)], children: Vec<OwnedValue>) -> Layout {
        (
            id,
            props
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.try_to_owned().unwrap()))
                .collect(),
            children,
        )
    }

    fn boxed(layout: Layout) -> OwnedValue {
        Value::from(zbus::zvariant::Structure::from(layout))
            .try_to_owned()
            .unwrap()
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl Menu {
        fn get_layout(&self, _parent: i32, _depth: i32, _props: Vec<String>) -> (u32, Layout) {
            let open = node(1, &[("label", Value::from("_Open Chat"))], Vec::new());
            let away = node(
                3,
                &[
                    ("label", Value::from("Away")),
                    ("toggle-type", Value::from("checkmark")),
                    ("toggle-state", Value::from(1i32)),
                ],
                Vec::new(),
            );
            let status = node(
                2,
                &[
                    ("label", Value::from("Status")),
                    ("children-display", Value::from("submenu")),
                ],
                vec![boxed(away)],
            );
            let separator = node(4, &[("type", Value::from("separator"))], Vec::new());
            let root = node(0, &[], vec![boxed(open), boxed(separator), boxed(status)]);
            (1, root)
        }
        fn get_group_properties(
            &self,
            _ids: Vec<i32>,
            _props: Vec<String>,
        ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
            Vec::new()
        }
        fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("Event {id} {event_id}"));
        }
        fn about_to_show(&self, _id: i32) -> bool {
            self.calls.lock().unwrap().push("AboutToShow".into());
            false
        }
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "normal".into()
        }
    }
}

#[test]
fn the_tray_host_lists_activates_and_browses_another_applications_item() {
    use compass_ipc::{Request, Response, TrayMenuEntry};
    use std::io::{BufRead, BufReader};
    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();

    let daemon = Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], "{}", |_| {
        vec![("DBUS_SESSION_BUS_ADDRESS", address.clone().into())]
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let calls = fake_tray::Calls::default();
    let item_path = "/org/fake/StatusNotifierItem";
    let _connection = runtime.block_on(async {
        let connection = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .serve_at(
                item_path,
                fake_tray::Item {
                    calls: calls.clone(),
                },
            )
            .unwrap()
            .serve_at(
                "/FakeMenu",
                fake_tray::Menu {
                    calls: calls.clone(),
                },
            )
            .unwrap()
            .build()
            .await
            .expect("the fake item connects to the private bus");
        // The engine's host serves the watcher; wait for it to own the name.
        let mut registered = false;
        for _ in 0..100 {
            let reply = connection
                .call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    // As libappindicator does: the path alone, from the
                    // item's own connection.
                    &(item_path),
                )
                .await;
            if reply.is_ok() {
                registered = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(registered, "the engine never served the watcher");
        // Compass's own icon registers with the same watcher; wait for it,
        // so the list below is asked with both there.
        let watcher = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination("org.kde.StatusNotifierWatcher")
            .unwrap()
            .path("/StatusNotifierWatcher")
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut both = false;
        for _ in 0..200 {
            let items = watcher
                .get(
                    zbus::names::InterfaceName::try_from("org.kde.StatusNotifierWatcher").unwrap(),
                    "RegisteredStatusNotifierItems",
                )
                .await
                .ok()
                .and_then(|value| Vec::<String>::try_from(value).ok())
                .unwrap_or_default();
            if items.len() >= 2 {
                both = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(both, "Compass's own icon never registered");
        connection
    });

    let mut items = Vec::new();
    for _ in 0..100 {
        match daemon.request(Request::TrayItems) {
            Response::TrayItems { items: found } if !found.is_empty() => {
                items = found;
                break;
            }
            Response::TrayItems { .. } => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => panic!("TrayItems: {other:?}"),
        }
    }
    let [item] = items.as_slice() else {
        panic!("the fake item was not listed: {items:?}");
    };
    assert_eq!(item.title, "Fake Chat");
    assert_eq!(item.subtitle, "3 unread messages");
    assert!(item.attention);
    assert!(item.has_menu && !item.item_is_menu);
    assert_eq!(item.icon_name.as_deref(), Some("fake-chat-icon"));
    let key = item.key.clone();

    assert!(matches!(
        daemon.request(Request::TrayActivate {
            key: key.clone(),
            secondary: false
        }),
        Response::Ack
    ));
    assert!(matches!(
        daemon.request(Request::TrayActivate {
            key: key.clone(),
            secondary: true
        }),
        Response::Ack
    ));

    let Response::TrayMenu { entries } = daemon.request(Request::TrayMenu { key: key.clone() })
    else {
        panic!("no menu");
    };
    assert_eq!(
        entries,
        [
            TrayMenuEntry {
                id: 1,
                label: "Open Chat".into(),
                toggled: None,
                icon_name: None,
            },
            TrayMenuEntry {
                id: 3,
                label: "Status › Away".into(),
                toggled: Some(true),
                icon_name: None,
            },
        ]
    );
    assert!(matches!(
        daemon.request(Request::TrayTriggerMenu { key, id: 3 }),
        Response::Ack
    ));

    let calls = calls.lock().unwrap().clone();
    let _ = bus.kill();
    let _ = bus.wait();
    assert!(
        calls.starts_with(&["Activate".to_owned(), "SecondaryActivate".to_owned()]),
        "the item was activated at its own path: {calls:?}"
    );
    assert!(calls.contains(&"AboutToShow".to_owned()), "{calls:?}");
    assert!(calls.contains(&"Event 3 clicked".to_owned()), "{calls:?}");
}

/// A desktop's `StatusNotifierWatcher` on a private bus, recording each item
/// that registers.
mod fake_watcher {
    use std::sync::{Arc, Mutex};

    /// `(the name it registered, the connection it came from)`.
    pub type Registered = Arc<Mutex<Vec<(String, String)>>>;

    pub struct Watcher {
        pub registered: Registered,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(
            &self,
            service: String,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            let sender = header.sender().map(ToString::to_string).unwrap_or_default();
            self.registered.lock().unwrap().push((service, sender));
        }
        fn register_status_notifier_host(&self, _service: String) {}
        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.registered
                .lock()
                .unwrap()
                .iter()
                .map(|(service, _)| service.clone())
                .collect()
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn protocol_version(&self) -> i32 {
            0
        }
    }
}

#[test]
fn compass_shows_its_own_tray_icon_with_the_cpp_menu_as_the_setting_says() {
    use compass_ipc::{Request, Response, WindowCommand};
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use zbus::zvariant::{OwnedValue, Value};

    let mut bus = match Command::new("dbus-daemon")
        .args(["--session", "--print-address", "--nofork"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(bus) => bus,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED: no dbus-daemon on this machine");
            return;
        }
        Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
    };
    let mut address = String::new();
    BufReader::new(bus.stdout.take().expect("piped stdout"))
        .read_line(&mut address)
        .expect("dbus-daemon prints its address");
    let address = address.trim().to_owned();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let registered = fake_watcher::Registered::default();
    // The desktop's watcher is there before the engine, as it is in a
    // session: the engine's own host then leaves the name alone.
    let connection = runtime.block_on(async {
        zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                fake_watcher::Watcher {
                    registered: registered.clone(),
                },
            )
            .unwrap()
            .build()
            .await
            .expect("the fake watcher owns its name on the private bus")
    });

    // The browser the link entries open: a script on the engine's PATH that
    // writes down what it was given.
    let opened_log = std::sync::OnceLock::new();
    let browser = "[Desktop Entry]\nType=Application\nName=Recorder\n\
                   Exec=compass-test-browser %u\n\
                   MimeType=x-scheme-handler/http;x-scheme-handler/https;\n";
    let mut daemon = Daemon::start_prepared(
        &[
            ("a.desktop", &entry("Alpha", "")),
            ("recorder.desktop", browser),
        ],
        r#"{"tray": {"enabled": false}}"#,
        |root| {
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let log = root.join("opened.log");
            let script = bin.join("compass-test-browser");
            std::fs::write(
                &script,
                format!("#!/bin/sh\necho \"$1\" >> '{}'\n", log.display()),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            opened_log.set(log).unwrap();
            let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            )))
            .unwrap();
            vec![
                ("DBUS_SESSION_BUS_ADDRESS", address.clone().into()),
                ("PATH", path),
            ]
        },
    );
    let opened = || {
        std::fs::read_to_string(opened_log.get().unwrap())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let window = FakeWindow::attach(&daemon.socket, compass_ipc::WindowOutcome::Shown);
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let set_tray = |on: bool| {
        assert_eq!(
            daemon.request(Request::SetSetting {
                key: "tray.enabled".to_owned(),
                value_json: on.to_string(),
            }),
            Response::Ack
        );
    };

    // Off in the file: nothing registers.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(registered.lock().unwrap().is_empty(), "the icon is off");

    // Turned on from the settings view: shown at once.
    set_tray(true);
    wait_for("the icon to register", &|| {
        !registered.lock().unwrap().is_empty()
    });
    let (service, sender) = registered.lock().unwrap()[0].clone();
    assert!(
        service.starts_with("org.kde.StatusNotifierItem-"),
        "the specification's name: {service}"
    );

    let (id, title, menu_path, labels) = runtime.block_on(async {
        let item = zbus::fdo::PropertiesProxy::builder(&connection)
            .destination(service.as_str())
            .unwrap()
            .path("/StatusNotifierItem")
            .unwrap()
            .build()
            .await
            .unwrap();
        let iface = zbus::names::InterfaceName::try_from("org.kde.StatusNotifierItem").unwrap();
        let id = String::try_from(item.get(iface.clone(), "Id").await.unwrap()).unwrap();
        let title = String::try_from(item.get(iface.clone(), "Title").await.unwrap()).unwrap();
        let menu = zbus::zvariant::OwnedObjectPath::try_from(
            item.get(iface.clone(), "Menu").await.unwrap(),
        )
        .unwrap();
        connection
            .call_method(
                Some(service.as_str()),
                "/StatusNotifierItem",
                Some("org.kde.StatusNotifierItem"),
                "Activate",
                &(0i32, 0i32),
            )
            .await
            .expect("Activate");
        let reply = connection
            .call_method(
                Some(service.as_str()),
                menu.as_str(),
                Some("com.canonical.dbusmenu"),
                "GetGroupProperties",
                &(Vec::<i32>::new(), vec!["label".to_owned()]),
            )
            .await
            .expect("GetGroupProperties");
        let mut entries: Vec<(i32, HashMap<String, OwnedValue>)> =
            reply.body().deserialize().unwrap();
        entries.sort_by_key(|(id, _)| *id);
        let labels: Vec<(i32, String)> = entries
            .into_iter()
            .filter_map(|(id, properties)| {
                let label = properties.get("label")?;
                Some((id, String::try_from(label.try_clone().ok()?).ok()?))
            })
            .collect();
        (id, title, menu, labels)
    });
    assert_eq!(id, "org.tunaos.compass");
    assert_eq!(title, "Compass");
    let texts: Vec<&str> = labels.iter().map(|(_, label)| label.as_str()).collect();
    let version = format!("Compass {}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        texts,
        [
            "Toggle Compass",
            version.as_str(),
            "About Compass",
            "Settings…",
            "Sponsor Upstream Vicinae",
            "Compass on GitHub",
            "Quit Compass",
        ]
    );
    let entry = |label: &str| {
        labels
            .iter()
            .find(|(_, text)| text == label)
            .map(|(id, _)| *id)
            .unwrap()
    };
    let click = |id: i32| {
        runtime.block_on(async {
            connection
                .call_method(
                    Some(service.as_str()),
                    menu_path.as_str(),
                    Some("com.canonical.dbusmenu"),
                    "Event",
                    &(id, "clicked", Value::from(0i32), 0u32),
                )
                .await
                .expect("Event");
        });
    };
    // Every entry of the menu model, clicked, does what the model says it
    // does (#254): a new entry, or one that stops reaching anything, fails
    // here. Quit is clicked last, below, as it stops the engine.
    use compass_core::tray::{Activation, EntryKind};
    let mut told = vec![WindowCommand::Toggle]; // `Activate`, above
    let mut links = Vec::new();
    for menu_entry in compass_core::tray::menu_entries(false) {
        let kind = menu_entry.kind;
        if kind == EntryKind::Separator || !compass_core::tray::entry_enabled(kind) {
            continue;
        }
        let label = compass_core::tray::entry_label(kind, env!("CARGO_PKG_VERSION"));
        let activation = compass_core::tray::activate(&[menu_entry], menu_entry.id)
            .unwrap_or_else(|| panic!("{label} can be clicked and does nothing"));
        match activation {
            Activation::Quit => continue,
            Activation::Toggle => told.push(WindowCommand::Toggle),
            Activation::OpenSettings { tab } => told.push(WindowCommand::Deeplink(match tab {
                Some(tab) => format!("compass://settings/open?tab={tab}"),
                None => "compass://settings/open".to_owned(),
            })),
            Activation::OpenLink(link) => links.push(link.url().to_owned()),
        }
        click(entry(&label));
    }
    wait_for("the window to be told", &|| {
        window.seen().len() >= told.len()
    });
    wait_for("the browser to open the links", &|| {
        opened().len() >= links.len()
    });
    assert_eq!(window.seen(), told);
    // Each link's browser runs in its own process, so they land in any order.
    let mut seen_links = opened();
    seen_links.sort();
    links.sort();
    assert_eq!(seen_links, links);

    // Turned off: the item leaves the bus. On again: it comes back.
    let has_owner = |name: &str| {
        runtime.block_on(async {
            zbus::fdo::DBusProxy::new(&connection)
                .await
                .unwrap()
                .name_has_owner(zbus::names::BusName::try_from(name).unwrap())
                .await
                .unwrap()
        })
    };
    assert!(has_owner(&sender));
    set_tray(false);
    wait_for("the icon to leave", &|| !has_owner(&sender));
    set_tray(true);
    wait_for("the icon to come back", &|| {
        registered.lock().unwrap().len() == 2
    });
    let (service, _) = registered.lock().unwrap()[1].clone();

    // Quit stops the engine, as `compass shutdown` does.
    let quit = runtime.block_on(async {
        let reply = connection
            .call_method(
                Some(service.as_str()),
                menu_path.as_str(),
                Some("com.canonical.dbusmenu"),
                "GetGroupProperties",
                &(Vec::<i32>::new(), vec!["label".to_owned()]),
            )
            .await
            .unwrap();
        let entries: Vec<(i32, HashMap<String, OwnedValue>)> = reply.body().deserialize().unwrap();
        entries
            .into_iter()
            .find(|(_, properties)| {
                properties
                    .get("label")
                    .and_then(|label| String::try_from(label.try_clone().ok()?).ok())
                    .is_some_and(|label| label == "Quit Compass")
            })
            .map(|(id, _)| id)
            .unwrap()
    });
    runtime.block_on(async {
        connection
            .call_method(
                Some(service.as_str()),
                menu_path.as_str(),
                Some("com.canonical.dbusmenu"),
                "Event",
                &(quit, "clicked", Value::from(0i32), 0u32),
            )
            .await
            .expect("Event");
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = daemon.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "Quit did not stop the engine");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "a clean stop: {status:?}");
    drop(window);
    let _ = bus.kill();
    let _ = bus.wait();
}

#[test]
fn with_no_session_bus_the_tray_is_refused_by_name() {
    use compass_ipc::{ErrorKind, Request, Response};
    let daemon = Daemon::start(&[("a.desktop", &entry("Alpha", ""))]);
    let Response::Error(err) = daemon.request(Request::TrayItems) else {
        panic!("the tray answered with no session bus");
    };
    assert_eq!(err.kind, ErrorKind::Unsupported);
}

#[test]
fn open_with_lists_what_opens_a_target_the_default_first() {
    use compass_ipc::{ErrorKind, Request, Response};
    let web = entry("Web", "MimeType=x-scheme-handler/https;text/html;\n");
    let surf = entry("Surf", "MimeType=x-scheme-handler/https;\n");
    let editor = entry("Editor", "MimeType=text/plain;\n");
    let daemon = Daemon::start(&[
        ("web.desktop", web.as_str()),
        ("surf.desktop", surf.as_str()),
        ("editor.desktop", editor.as_str()),
    ]);
    let config = daemon._dirs.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("mimeapps.list"),
        "[Default Applications]\nx-scheme-handler/https=web.desktop\n",
    )
    .unwrap();
    let Response::Openers { apps } = daemon.request(Request::ListOpeners {
        target: "https://docs.rs/{crate}".into(),
    }) else {
        panic!("no openers");
    };
    assert_eq!(
        apps.iter()
            .map(|app| (app.id.as_str(), app.default))
            .collect::<Vec<_>>(),
        [("web.desktop", true), ("surf.desktop", false)],
        "what opens https, the default first, not the editor"
    );
    assert!(matches!(
        daemon.request(Request::OpenWith {
            app: "nothing.desktop".into(),
            target: "https://docs.rs".into(),
        }),
        Response::Error(e) if e.kind == ErrorKind::BadRequest
    ));
}

#[test]
fn a_files_panel_learns_its_mime_type_and_an_appimage_is_made_executable_and_run() {
    use compass_ipc::{ErrorKind, Request, Response};
    use std::os::unix::fs::PermissionsExt;
    let daemon = Daemon::start(&[]);
    let home = daemon._dirs.path();
    let image = home.join("sunset.png");
    std::fs::write(&image, b"\x89PNG").unwrap();
    let Response::FileActions(info) = daemon.request(Request::FileActions {
        path: image.to_string_lossy().into_owned(),
    }) else {
        panic!("no file actions");
    };
    assert_eq!(info.mime.as_deref(), Some("image/png"));
    assert!(!info.has_opener, "no application is installed");

    let marker = home.join("ran");
    let tool = home.join("Tool.AppImage");
    std::fs::write(
        &tool,
        format!("#!/bin/sh\necho ran > '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        daemon.request(Request::RunExecutable {
            path: tool.to_string_lossy().into_owned(),
            make_executable: true,
        }),
        Response::Ack
    );
    assert_ne!(
        std::fs::metadata(&tool).unwrap().permissions().mode() & 0o100,
        0
    );
    wait_for_content(&marker);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "ran\n");

    assert!(matches!(
        daemon.request(Request::SetWallpaper {
            path: home.join("gone.png").to_string_lossy().into_owned(),
        }),
        Response::Error(e) if e.kind == ErrorKind::BadRequest
    ));
}

// ---- The update check, against a local feed ----

/// A local stand-in for `api.github.com/repos/tuna-os/compass/releases/latest`,
/// counting how often it is asked.
struct FakeFeed {
    url: String,
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    _server: std::sync::Arc<tiny_http::Server>,
}

impl FakeFeed {
    fn start(tag: &'static str) -> FakeFeed {
        let server =
            std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("fake feed"));
        let port = server.server_addr().to_ip().expect("ip").port();
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (thread_server, count) = (
            std::sync::Arc::clone(&server),
            std::sync::Arc::clone(&asked),
        );
        std::thread::spawn(move || {
            for request in thread_server.incoming_requests() {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let body = serde_json::json!({
                    "tag_name": tag,
                    "html_url": format!("https://github.com/tuna-os/compass/releases/tag/{tag}"),
                    "draft": false,
                    "prerelease": false,
                    "assets": [],
                })
                .to_string();
                let _ = request.respond(tiny_http::Response::from_string(body));
            }
        });
        FakeFeed {
            url: format!("http://127.0.0.1:{port}/repos/tuna-os/compass/releases/latest"),
            asked,
            _server: server,
        }
    }

    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn start_engine(&self, config: &str) -> Daemon {
        let url = self.url.clone();
        Daemon::start_prepared(&[("a.desktop", &entry("Alpha", ""))], config, move |_| {
            vec![
                ("COMPASS_UPDATE_FEED_URL", url.into()),
                ("COMPASS_UPDATE_VERSION", "v0.1.0".into()),
            ]
        })
    }
}

fn update_status(daemon: &Daemon) -> (String, Option<compass_ipc::UpdateOffer>) {
    let compass_ipc::Response::UpdateStatus { current, available } =
        daemon.request(compass_ipc::Request::UpdateStatus)
    else {
        panic!("no update status");
    };
    (current, available)
}

#[test]
fn a_newer_release_is_offered_once_checked_and_skipping_it_is_remembered() {
    let feed = FakeFeed::start("v0.2.0");
    let daemon = feed.start_engine("{}");

    let (current, offer) = update_status(&daemon);
    assert_eq!(current, "v0.1.0");
    let offer = offer.expect("offered");
    assert_eq!(offer.tag, "v0.2.0");
    assert_eq!(offer.version, "0.2.0");
    assert_eq!(
        offer.release_url,
        "https://github.com/tuna-os/compass/releases/tag/v0.2.0"
    );
    assert!(update_status(&daemon).1.is_some());
    assert_eq!(feed.asked(), 1, "the second answer came from the cache");
    assert!(
        daemon
            ._dirs
            .path()
            .join(".cache/compass/latest-release.json")
            .is_file(),
        "the check is remembered under the cache directory"
    );

    assert_eq!(
        daemon.request(compass_ipc::Request::SkipUpdate {
            tag: "v0.2.0".into()
        }),
        compass_ipc::Response::Ack
    );
    assert_eq!(update_status(&daemon).1, None, "skipped");
    let state = std::fs::read_to_string(
        daemon
            ._dirs
            .path()
            .join(".local/state/compass/updates.json"),
    )
    .expect("the state file");
    assert!(state.contains("\"skippedVersion\":\"v0.2.0\""), "{state}");
}

#[test]
fn with_update_checks_off_the_feed_is_never_asked() {
    let feed = FakeFeed::start("v0.2.0");
    let daemon = feed.start_engine(r#"{"launcher": {"check_for_updates": false}}"#);
    assert_eq!(update_status(&daemon).1, None);
    assert_eq!(feed.asked(), 0);
}

// ---- The calculator's exchange rates, against a local fake ECB ----

const ECB_FIXTURE: &str = include_str!("../../compass-core/tests/fixtures/eurofxref-daily.xml");

/// A local stand-in for the ECB's daily file, which can be made to fail.
struct FakeEcb {
    url: String,
    failing: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fetches: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    _server: std::sync::Arc<tiny_http::Server>,
}

impl FakeEcb {
    fn start() -> FakeEcb {
        use std::sync::atomic::Ordering;
        let server = std::sync::Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("fake ECB"));
        let port = server.server_addr().to_ip().expect("ip").port();
        let failing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fetches = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (thread_server, thread_failing, thread_fetches) = (
            std::sync::Arc::clone(&server),
            std::sync::Arc::clone(&failing),
            std::sync::Arc::clone(&fetches),
        );
        std::thread::spawn(move || {
            for request in thread_server.incoming_requests() {
                thread_fetches.fetch_add(1, Ordering::SeqCst);
                let response = if thread_failing.load(Ordering::SeqCst) {
                    tiny_http::Response::from_data(b"unavailable".to_vec()).with_status_code(503)
                } else {
                    tiny_http::Response::from_data(ECB_FIXTURE.as_bytes().to_vec())
                };
                let _ = request.respond(response);
            }
        });
        FakeEcb {
            url: format!("http://127.0.0.1:{port}/stats/eurofxref/eurofxref-daily.xml"),
            failing,
            fetches,
            _server: server,
        }
    }
}

fn exchange_rates(daemon: &Daemon) -> Option<compass_ipc::ExchangeRateTable> {
    match daemon.request(compass_ipc::Request::ExchangeRates) {
        compass_ipc::Response::ExchangeRates { rates } => rates,
        other => panic!("unexpected answer: {other:?}"),
    }
}

#[test]
fn the_engine_fetches_the_ecb_rates_at_start_caches_them_and_keeps_them_when_a_refresh_fails() {
    use std::sync::atomic::Ordering;
    let ecb = FakeEcb::start();
    let url = ecb.url.clone();
    let daemon = Daemon::start_prepared(&[], "{}", move |_| {
        vec![
            ("COMPASS_DISABLE_AUTO_RATE_REFRESH", "".into()),
            ("COMPASS_EXCHANGE_RATES_URL", url.into()),
        ]
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    let rates = loop {
        if let Some(rates) = exchange_rates(&daemon) {
            break rates;
        }
        assert!(
            Instant::now() < deadline,
            "the rates were never fetched at start"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(rates.date, "2026-09-24");
    assert!(
        rates
            .rates
            .contains(&("USD".to_owned(), "1.1367".to_owned()))
    );
    assert!(rates.rates.contains(&("EUR".to_owned(), "1".to_owned())));
    let cache = daemon
        ._dirs
        .path()
        .join(".cache/compass/exchange-rates.json");
    assert!(
        cache.is_file(),
        "the rates are cached under Compass's cache dir"
    );

    match daemon.request(compass_ipc::Request::RefreshExchangeRates) {
        compass_ipc::Response::ExchangeRates { rates: Some(fresh) } => {
            assert_eq!(fresh.date, "2026-09-24");
            assert!(fresh.fetched_at >= rates.fetched_at);
        }
        other => panic!("unexpected answer: {other:?}"),
    }
    assert_eq!(
        ecb.fetches.load(Ordering::SeqCst),
        2,
        "start, then the forced refresh"
    );

    ecb.failing.store(true, Ordering::SeqCst);
    match daemon.request(compass_ipc::Request::RefreshExchangeRates) {
        compass_ipc::Response::Error(error) => {
            assert!(
                error
                    .message
                    .contains("could not refresh the exchange rates"),
                "{error:?}"
            );
        }
        other => panic!("a failed refresh must say so: {other:?}"),
    }
    assert_eq!(
        exchange_rates(&daemon).map(|rates| rates.date),
        Some("2026-09-24".to_owned()),
        "the rates held survive a failed refresh"
    );
}

#[test]
fn an_offline_engine_answers_with_the_cached_rates_or_none() {
    let cached = compass_core::exchange_rates::parse_ecb(ECB_FIXTURE, 1).unwrap();
    let daemon = Daemon::start_prepared(&[], "{}", move |root| {
        let path = compass_core::exchange_rates::cache_path(&root.join(".cache"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&cached).unwrap()).unwrap();
        Vec::new()
    });
    let rates = exchange_rates(&daemon).expect("the cached rates");
    assert_eq!(rates.date, "2026-09-24");
    assert_eq!(rates.fetched_at, 1);

    let empty = Daemon::start(&[]);
    assert_eq!(
        exchange_rates(&empty),
        None,
        "no cache and no network: no rates"
    );
    assert!(matches!(
        empty.request(compass_ipc::Request::RefreshExchangeRates),
        compass_ipc::Response::Error(_)
    ));
}

/// A stand-in for Raycast's Brew extension, installed as a Raycast store
/// extension, doing what its Show Installed command does: find Homebrew's
/// prefix with `execSync("brew --prefix")` at load (falling back to macOS's
/// prefix), then list what is installed with the promisified `exec` of
/// `<prefix>/bin/brew info --json=v2 --installed`. And a fake `brew` in a
/// temporary prefix, which is the only Homebrew this test ever runs.
fn install_brewlike(root: &Path) -> PathBuf {
    let ext = root.join("data-home/compass/extensions/store.raycast.brewlike");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("package.json"),
        r#"{"name": "brewlike", "title": "Brewlike", "author": "nhojb",
            "commands": [{"name": "installed", "title": "Show Installed", "mode": "view"}]}"#,
    )
    .unwrap();
    std::fs::write(
        ext.join("installed.js"),
        "const React = require('react');
         const { List } = require('@raycast/api');
         const { execSync, exec } = require('child_process');
         const { promisify } = require('util');
         const prefix = (() => {
           try { return execSync('brew --prefix', { encoding: 'utf8' }).trim(); }
           catch (e) { return '/opt/homebrew'; }
         })();
         module.exports.default = () => {
           const [items, setItems] = React.useState(null);
           React.useEffect(() => {
             promisify(exec)(`${prefix}/bin/brew info --json=v2 --installed`, {
               env: { ...process.env, HOMEBREW_NO_AUTO_UPDATE: '1' },
             }).then(
               ({ stdout }) => setItems(JSON.parse(stdout).formulae.map((f) => f.name)),
               (e) => setItems(['failed: ' + e.message]),
             );
           }, []);
           return React.createElement(List, { isLoading: items === null },
             (items || []).map((name) => React.createElement(List.Item, { key: name, title: name })));
         };",
    )
    .unwrap();

    let prefix = root.join("linuxbrew");
    let bin = prefix.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let brew = bin.join("brew");
    std::fs::write(
        &brew,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --prefix) echo {} ;;\n  \
             info) echo '{{\"formulae\":[{{\"name\":\"wget\"}},{{\"name\":\"jq\"}}],\"casks\":[]}}' ;;\n  \
             *) exit 1 ;;\nesac\n",
            prefix.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// The titles of the list `session` shows once it has finished loading.
fn listed(daemon: &Daemon, session: u64) -> Vec<String> {
    use compass_extension_api::View;
    let (view, _) = wait_for_view(
        daemon,
        session,
        |view, _| matches!(view, View::List(list) if list.sections.iter().any(|s| !s.items.is_empty())),
    );
    let View::List(list) = view else {
        unreachable!()
    };
    list.sections
        .iter()
        .flat_map(|section| section.items.iter().map(|item| item.title.clone()))
        .collect()
}

#[test]
fn raycasts_brew_runs_homebrew_on_the_host_once_the_person_allows_it() {
    use compass_ipc::{Request, Response};
    let Some(runtime) = extension_runtime() else {
        assert!(
            std::env::var_os("COMPASS_REQUIRE_RUNTIME").is_none_or(|v| v != "1"),
            "COMPASS_REQUIRE_RUNTIME=1 but no runtime bundle; `make extension-runtime`"
        );
        eprintln!("skipping: no extension runtime bundle");
        return;
    };
    let mut root = PathBuf::new();
    let daemon = Daemon::start_prepared(&[], "{}", |dir| {
        root = dir.to_path_buf();
        let bin = install_brewlike(dir);
        // The fake prefix first: the broker looks `brew` up on the engine's
        // PATH before Linuxbrew's own prefix.
        let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .unwrap();
        vec![
            ("COMPASS_EXTENSION_RUNTIME", runtime.into_os_string()),
            ("PATH", path),
        ]
    });
    let id = "@nhojb/store.raycast.brewlike:installed";
    let grants = root.join("config/compass/host-command-grants.json");

    // First run: the extension blocks on `execSync("brew --prefix")` while
    // the person is asked.
    let started = daemon.request(Request::RunExtensionCommand {
        id: id.into(),
        arguments_json: None,
    });
    let Response::ExtensionStarted { session } = started else {
        panic!("no session: {started:?}");
    };
    wait_for_alert(&daemon, session, "Allow Brewlike to run brew?");
    let Response::ExtensionView { alert, .. } =
        daemon.request(Request::ExtensionView { session, after: 0 })
    else {
        panic!("no view answer");
    };
    let alert = alert.expect("the consent prompt");
    assert_eq!(
        (alert.confirm_text.as_str(), alert.cancel_text.as_str()),
        ("Allow Once", "Deny")
    );
    assert_eq!(alert.remember_text.as_deref(), Some("Always Allow"));
    assert_eq!(
        daemon.request(Request::ExtensionAlertRemember { session }),
        Response::Ack
    );
    assert_eq!(
        listed(&daemon, session),
        ["wget", "jq"],
        "Show Installed lists what the host's brew reports"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&grants).unwrap())
            .unwrap(),
        serde_json::json!({ "extensions": { "store.raycast.brewlike": ["brew"] } }),
        "Always Allow is kept"
    );
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );

    // Script Permissions lists it, beside the Rhai scripts.
    let Response::ScriptGrants {
        grants: listed_grants,
    } = daemon.request(Request::ListScriptGrants)
    else {
        panic!("no grants");
    };
    assert_eq!(
        listed_grants,
        [compass_ipc::ScriptGrantEntry {
            id: "store.raycast.brewlike".into(),
            title: "Brewlike".into(),
            capabilities: vec!["host.run:brew".into()],
            descriptions: vec!["run brew on your computer, outside the extension sandbox".into()],
        }]
    );

    // Second run: remembered, so nobody is asked.
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: id.into(),
        arguments_json: None,
    }) else {
        panic!("no second session");
    };
    assert_eq!(listed(&daemon, session), ["wget", "jq"]);
    assert_eq!(
        daemon.request(Request::CloseExtension { session }),
        Response::Ack
    );

    // Revoked, the next run asks again. Denied, the extension's
    // `execSync` throws and it falls back to macOS's prefix, as it would on
    // a Mac without Homebrew; that brew goes to the broker too, is asked
    // about again, and denied again the extension is told why by name.
    assert_eq!(
        daemon.request(Request::RevokeScriptGrant {
            id: "store.raycast.brewlike".into()
        }),
        Response::ScriptGrants { grants: vec![] }
    );
    let Response::ExtensionStarted { session } = daemon.request(Request::RunExtensionCommand {
        id: id.into(),
        arguments_json: None,
    }) else {
        panic!("no third session");
    };
    for _ in 0..2 {
        wait_for_alert(&daemon, session, "Allow Brewlike to run brew?");
        assert_eq!(
            daemon.request(Request::ExtensionAlertAnswer {
                session,
                confirmed: false
            }),
            Response::Ack
        );
    }
    let titles = listed(&daemon, session);
    assert_eq!(titles.len(), 1);
    assert!(
        titles[0].contains("You did not allow Brewlike to run brew on your computer"),
        "{titles:?}"
    );
}
