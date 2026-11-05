//! End-to-end tests: the real binary, a real Unix socket, a real session bus.
//!
//! # Isolation
//!
//! Every child process is started with `env_clear()` and given only variables
//! this test built, and every socket lives in a `tempfile::TempDir`. Nothing
//! here can reach the developer's live socket, `$HOME` or session bus: with the
//! environment cleared there is no `XDG_RUNTIME_DIR` for `zbus` to find a bus
//! under and no `DBUS_SESSION_BUS_ADDRESS` to follow, and `--socket` is passed
//! explicitly on every invocation.
//!
//! # No sleeping
//!
//! The listener is bound before the child is spawned, so the socket is in the
//! filesystem and accepting into its backlog by the time the child connects.
//! Requests are read off an unbounded channel with `recv().await`, which
//! resolves when the request arrives rather than after a guessed interval.

use std::path::{Path, PathBuf};
use std::process::Output;

use compass_ipc::{Listener, Request, Response};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// Pid the stub engine reports, distinctive enough to assert on.
const STUB_PID: u32 = 4242;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_compass"))
}

/// Runs the CLI with a hermetic environment.
async fn run_cli(home: &Path, socket: &Path, args: &[&str]) -> Output {
    let mut command = tokio::process::Command::new(binary());
    command
        .env_clear()
        // `PATH` only so the child can be exec'd normally; nothing in these
        // code paths shells out.
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("share"))
        .env("XDG_DATA_DIRS", home.join("sys-share"))
        // A deliberately dead bus address: reachable-looking, never reachable.
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", home.join("no-bus").display()),
        )
        .arg("--socket")
        .arg(socket)
        .args(args);
    command.output().await.expect("the CLI binary should run")
}

/// Binds a listener and serves a stub engine that records what it was asked.
async fn stub_engine(socket: &Path) -> UnboundedReceiver<Request> {
    let listener = Listener::bind(socket).await.expect("bind the test socket");
    let (tx, rx) = unbounded_channel();

    tokio::spawn(listener.serve(move |request: Request| {
        let tx = tx.clone();
        async move {
            let response = match &request {
                Request::Ping => Response::Pong {
                    protocol_version: compass_ipc::PROTOCOL_VERSION,
                    pid: STUB_PID,
                },
                _ => Response::Ack,
            };
            // Send after deciding, before answering: the client cannot observe
            // the response until the request is already recorded.
            tx.send(request).expect("the test still holds the receiver");
            response
        }
    }));

    rx
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// --------------------------------------------------------------------------
// Round trips against a live engine
// --------------------------------------------------------------------------

#[tokio::test]
async fn window_commands_send_the_matching_request() {
    for (argument, expected) in [
        ("toggle", Request::Toggle),
        ("show", Request::Show),
        ("hide", Request::Hide),
        // The C++ spellings must reach the same requests.
        ("open", Request::Show),
        ("close", Request::Hide),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("ipc.sock");
        let mut requests = stub_engine(&socket).await;

        let output = run_cli(dir.path(), &socket, &[argument]).await;
        assert!(
            output.status.success(),
            "`{argument}` failed: {}",
            stderr(&output)
        );

        let received = requests.recv().await.expect("a request should arrive");
        assert_eq!(received, expected, "for `compass {argument}`");
    }
}

#[tokio::test]
async fn ping_reports_the_protocol_version_and_pid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let mut requests = stub_engine(&socket).await;

    let output = run_cli(dir.path(), &socket, &["ping"]).await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(requests.recv().await, Some(Request::Ping));

    let text = stdout(&output);
    assert!(
        text.contains(&format!("protocol v{}", compass_ipc::PROTOCOL_VERSION)),
        "missing protocol version: {text}"
    );
    assert!(
        text.contains(&format!("pid {STUB_PID}")),
        "missing pid: {text}"
    );
}

#[tokio::test]
async fn only_the_requested_command_is_sent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let mut requests = stub_engine(&socket).await;

    let output = run_cli(dir.path(), &socket, &["toggle"]).await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(requests.recv().await, Some(Request::Toggle));

    // The client closed its connection; nothing further was sent.
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn a_launch_deeplink_goes_to_the_engine_bare_or_as_a_subcommand() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let mut requests = stub_engine(&socket).await;
    let url = "compass://launch/commands/clipboard-history";
    // `run_cli` puts `--socket <path>` first, so the bare form is the one a
    // desktop entry's `Exec=compass %u` runs with a global option before it.
    for args in [&["deeplink", url][..], &[url][..]] {
        let output = run_cli(dir.path(), &socket, args).await;
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert_eq!(
            requests.recv().await,
            Some(Request::OpenDeeplink { url: url.into() })
        );
    }
}

#[tokio::test]
async fn vici_develops_links_go_to_the_engine_and_a_malformed_one_says_why() {
    // `vici develop` reaches the engine through this command, so a link the
    // CLI refused would stop the SDK before it watches anything.
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let mut requests = stub_engine(&socket).await;
    for action in ["start", "refresh", "stop"] {
        let url = format!("compass://api/extensions/develop/{action}?id=hello");
        let output = run_cli(dir.path(), &socket, &["deeplink", &url]).await;
        assert!(output.status.success(), "{url}: {}", stderr(&output));
        assert_eq!(requests.recv().await, Some(Request::OpenDeeplink { url }));
    }

    let output = run_cli(
        dir.path(),
        &socket,
        &["deeplink", "compass://api/extensions/develop/start"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("?id="), "{}", stderr(&output));
    assert!(
        requests.try_recv().is_err(),
        "a malformed link reached the engine"
    );
}

// --------------------------------------------------------------------------
// No daemon
// --------------------------------------------------------------------------

#[tokio::test]
async fn commands_without_a_daemon_explain_themselves() {
    for argument in ["toggle", "show", "hide", "ping"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("absent.sock");

        let output = run_cli(dir.path(), &socket, &[argument]).await;
        assert_eq!(
            output.status.code(),
            Some(1),
            "`{argument}` should exit 1 with no daemon"
        );

        let text = stderr(&output);
        assert!(
            text.contains("no Compass engine is listening"),
            "`{argument}` did not explain itself: {text}"
        );
        assert!(text.contains("absent.sock"), "path not named: {text}");
        assert!(text.contains("compass doctor"), "no next step: {text}");
        // The raw IO error is context at the end, not the headline.
        assert!(
            !text.lines().next().unwrap_or_default().contains("os error"),
            "raw io error leaked into the headline: {text}"
        );
        assert!(text.contains("underlying error:"), "cause dropped: {text}");
    }
}

#[tokio::test]
async fn a_stale_socket_file_is_still_reported_as_no_daemon() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("stale.sock");
    // A plain file where a socket should be: connect() fails differently, and
    // the message must still be the actionable one.
    std::fs::write(&socket, b"not a socket").expect("write");

    let output = run_cli(dir.path(), &socket, &["toggle"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("no Compass engine is listening"));
}

// --------------------------------------------------------------------------
// The engine switch
// --------------------------------------------------------------------------

#[tokio::test]
async fn engine_cpp_refuses_engine_commands_rather_than_doing_the_rust_thing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let mut requests = stub_engine(&socket).await;

    let output = run_cli(dir.path(), &socket, &["--engine", "cpp", "toggle"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot dispatch"));
    // Crucially: nothing was sent to the Rust engine behind the user's back.
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn the_engine_flag_can_come_from_the_environment() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");

    let mut command = tokio::process::Command::new(binary());
    let output = command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.path())
        .env("COMPASS_ENGINE", "cpp")
        .arg("--socket")
        .arg(&socket)
        .arg("toggle")
        .output()
        .await
        .expect("run");

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot dispatch"));
}

#[tokio::test]
async fn an_unknown_engine_is_a_usage_error_not_a_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(
        dir.path(),
        &dir.path().join("s.sock"),
        &["--engine", "go", "toggle"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

// --------------------------------------------------------------------------
// doctor
// --------------------------------------------------------------------------

/// Parses `doctor --json` output into a name → status map.
fn statuses(value: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    value["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().expect("name").to_string(),
                c["status"].as_str().expect("status").to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn doctor_json_is_valid_json_with_a_stable_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");

    let output = run_cli(dir.path(), &socket, &["doctor", "--json"]).await;
    assert!(output.status.success(), "{}", stderr(&output));

    let value: serde_json::Value =
        serde_json::from_str(&stdout(&output)).expect("doctor --json must emit valid JSON");

    assert_eq!(value["schema"], compass::doctor::JSON_SCHEMA_VERSION);
    assert_eq!(value["engine"], "rust");
    assert_eq!(value["socket"], socket.to_string_lossy().as_ref());

    let summary = &value["summary"];
    for field in ["ok", "warn", "fail", "total"] {
        assert!(
            summary[field].is_u64(),
            "summary.{field} should be a number"
        );
    }
    assert!(
        ["ok", "warn", "fail"].contains(&summary["status"].as_str().expect("status")),
        "unexpected summary status: {summary}"
    );

    let checks = value["checks"].as_array().expect("checks array");
    assert_eq!(checks.len(), summary["total"].as_u64().unwrap() as usize);
    for check in checks {
        assert!(check["name"].is_string());
        assert!(["ok", "warn", "fail"].contains(&check["status"].as_str().unwrap()));
        assert!(check["detail"].is_string());
    }

    // Every check documented in the CLI must actually appear.
    let by_name = statuses(&value);
    for expected in [
        "engine.selected",
        "session.type",
        "desktop.environment",
        "xdg.runtime-dir",
        "ipc.socket",
        "dbus.session",
        "portal.desktop",
        "portal.global-shortcuts",
        "gnome.shell-extension",
        "kde.kwin",
        "flatpak.sandbox",
        "xdg.application-dirs",
    ] {
        assert!(by_name.contains_key(expected), "missing check: {expected}");
    }
}

#[tokio::test]
async fn doctor_detects_absence_in_a_bare_container() {
    // The container has no display server, no session bus and no portal. A
    // doctor that reported "fine" here would be exactly the useless one
    // PLAN.md §8.6 warns about.
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(
        dir.path(),
        &dir.path().join("ipc.sock"),
        &["doctor", "--json"],
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let by_name = statuses(&value);

    assert_eq!(by_name["session.type"], "fail", "no display server here");
    assert_eq!(by_name["dbus.session"], "fail", "no session bus here");
    assert_eq!(by_name["portal.desktop"], "fail", "no portal here");
    // No desktop is named, so a missing portal is a warning (bind the key by
    // hand) rather than a failure, as it is on GNOME and KDE.
    assert_eq!(by_name["portal.global-shortcuts"], "warn");
    assert_eq!(value["summary"]["status"], "fail");
    assert!(value["summary"]["fail"].as_u64().unwrap() >= 4);
}

#[tokio::test]
async fn doctor_sees_the_running_engine_on_the_socket() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");
    let _requests = stub_engine(&socket).await;

    let output = run_cli(dir.path(), &socket, &["doctor", "--json"]).await;
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(statuses(&value)["ipc.socket"], "ok");
}

#[tokio::test]
async fn doctor_reports_the_selected_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket = dir.path().join("ipc.sock");

    let output = run_cli(
        dir.path(),
        &socket,
        &["--engine", "cpp", "doctor", "--json"],
    )
    .await;
    // doctor must still run under --engine cpp: refusing to diagnose because
    // of the flag under diagnosis helps nobody.
    assert!(output.status.success(), "{}", stderr(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(value["engine"], "cpp");
    assert_eq!(statuses(&value)["engine.selected"], "warn");
}

#[tokio::test]
async fn doctor_human_output_is_aligned_and_summarised() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(dir.path(), &dir.path().join("ipc.sock"), &["doctor"]).await;
    assert!(output.status.success());

    let text = stdout(&output);
    assert!(text.contains("compass doctor — engine: rust"));
    assert!(text.contains("[FAIL] session.type"));
    assert!(text.contains("checks:"));

    // Markers line up because they are all the same width.
    let markers: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with('['))
        .map(|l| &l[..6])
        .collect();
    assert!(!markers.is_empty());
    assert!(
        markers
            .iter()
            .all(|m| ["[ ok ]", "[warn]", "[FAIL]"].contains(m))
    );
}

#[tokio::test]
async fn doctor_without_check_only_exits_zero_even_when_checks_fail() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(dir.path(), &dir.path().join("ipc.sock"), &["doctor"]).await;
    assert_eq!(output.status.code(), Some(0));
    assert!(stdout(&output).contains("[FAIL]"));
}

#[tokio::test]
async fn doctor_check_only_exits_non_zero_on_a_failure_and_hides_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(
        dir.path(),
        &dir.path().join("ipc.sock"),
        &["doctor", "--check-only"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("[FAIL]"));
    assert!(
        !text.contains("[ ok ]"),
        "check-only should hide passes: {text}"
    );
}

#[tokio::test]
async fn doctor_check_only_json_still_emits_the_full_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(
        dir.path(),
        &dir.path().join("ipc.sock"),
        &["doctor", "--check-only", "--json"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(value["checks"].as_array().expect("checks").len(), 20);
}

#[tokio::test]
async fn help_documents_the_exit_codes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_cli(
        dir.path(),
        &dir.path().join("s.sock"),
        &["doctor", "--help"],
    )
    .await;
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Exit codes:"));
    assert!(text.contains("no check failed"));
    assert!(text.contains("could not be parsed"));
}

// File-index size diagnostics need neither a daemon nor a writable database.
#[tokio::test]
async fn fs_stats_missing_database_is_structured_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let output = run_cli(
        dir.path(),
        &dir.path().join("absent.sock"),
        &["fs", "stats", "--json"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(report["schema"], 1);
    assert_eq!(report["status"], "missing");
    assert_eq!(report["database_bytes"]["status"], "missing");
    assert!(report["database_bytes"]["value"].is_null());
    assert!(!dir.path().join(".cache").exists());
    assert!(!stdout(&output).contains(&dir.path().display().to_string()));
}

#[tokio::test]
async fn fs_stats_reads_existing_index_without_a_daemon_or_data_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = compass::indexer_service::database_path(&dir.path().join(".cache"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = compass_sqlcipher_sys::open(&path, &[]).unwrap();
    db.execute_batch("PRAGMA journal_mode = DELETE").unwrap();
    compass_db::sqlite_writer::ensure_file_index_schema(&db).unwrap();
    db.execute_batch(
        "INSERT INTO indexed_file(path, skeleton_path) VALUES ('/private/report.txt', 'report txt')",
    )
    .unwrap();
    drop(db);
    let before = std::fs::read(&path).unwrap();
    let output = run_cli(
        dir.path(),
        &dir.path().join("absent.sock"),
        &["fs", "stats", "--json"],
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(report["status"], "ok");
    assert_eq!(report["indexed_rows"]["value"], 1);
    assert_eq!(report["database_bytes"]["value"], before.len() as u64);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!stdout(&output).contains("/private/report.txt"));
    let pages = |key: &str| report[key]["value"].as_u64().unwrap();
    assert_eq!(
        pages("live_bytes") + pages("reclaimable_bytes"),
        pages("page_count") * pages("page_size"),
        "in-use and reclaimable bytes split the pages: {report}"
    );

    let output = run_cli(
        dir.path(),
        &dir.path().join("absent.sock"),
        &["doctor", "--json"],
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let check = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "files.index")
        .expect("a files.index check");
    assert_eq!(check["status"], "ok", "{check}");
    assert!(
        check["detail"]
            .as_str()
            .unwrap()
            .starts_with("The file index holds 1 entry."),
        "{check}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before, "doctor only reads");
}

/// `compass ext sync --dry-run` reads `extensions.installed` and says what
/// would become of each entry, without the network or the engine.
#[tokio::test]
async fn ext_sync_dry_run_reports_each_declared_extension() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config/compass");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("compass.json"),
        r#"{"extensions": {"installed": [
            "vicinae:zoe/clock", "store.raycast.github", "local.notes", "brew:x/y"
        ]}}"#,
    )
    .unwrap();
    let extensions = dir.path().join("share/compass/extensions");
    let notes = extensions.join("local.notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(
        notes.join("package.json"),
        r#"{"name": "notes", "title": "Notes", "author": "me",
            "commands": [{"name": "show", "title": "Show Notes", "mode": "view"}]}"#,
    )
    .unwrap();

    let output = run_cli(
        dir.path(),
        &dir.path().join("absent.sock"),
        &["ext", "sync", "--dry-run", "--json"],
    )
    .await;
    assert_eq!(output.status.code(), Some(1), "brew:x/y is a problem");
    let report: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let outcomes: Vec<(&str, &str)> = report
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["entry"].as_str().unwrap(),
                item["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("vicinae:zoe/clock", "would_install"),
            ("store.raycast.github", "would_install"),
            ("local.notes", "present"),
            ("brew:x/y", "unresolvable"),
        ]
    );
    assert_eq!(report[0]["id"], "store.vicinae.clock");
    let installed: Vec<_> = std::fs::read_dir(&extensions)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(installed, ["local.notes"], "a dry run installs nothing");

    let output = run_cli(
        dir.path(),
        &dir.path().join("absent.sock"),
        &["doctor", "--json"],
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let check = value["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "extensions.declared")
        .expect("an extensions.declared check");
    assert_eq!(check["status"], "warn", "{check}");
    let detail = check["detail"].as_str().unwrap();
    assert!(
        detail.starts_with(
            "2 declared extensions are not installed yet: vicinae:zoe/clock, store.raycast.github."
        ),
        "{detail}"
    );
    assert!(detail.contains("brew:x/y cannot be installed"), "{detail}");
}
