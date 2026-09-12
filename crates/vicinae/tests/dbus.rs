//! Doctor checks against a *real* D-Bus session bus.
//!
//! The unit tests in `doctor::checks` prove the checks reason correctly from
//! injected facts. These prove the one thing a fake cannot: that [`ZbusProbe`]
//! turns real bus behaviour into those facts correctly — in particular that a
//! bus with nothing on it produces "that capability is absent" rather than
//! "the bus is broken", which is the distinction the whole diagnostic rests on.
//!
//! `dbus-run-session` gives us a private bus with no portal, no GNOME Shell and
//! no ambient session, which is exactly the absent case. If it is not installed
//! the tests skip rather than fail: this is an environment-dependent tier.
//!
//! [`ZbusProbe`]: vicinae::doctor::ZbusProbe

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn dbus_run_session() -> Option<PathBuf> {
    ["/usr/bin/dbus-run-session", "/bin/dbus-run-session"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Runs `vicinae doctor --json` inside a private session bus.
///
/// Returns `None` when `dbus-run-session` is unavailable.
fn doctor_on_a_private_bus(home: &Path, extra: &[(&str, &str)]) -> Option<serde_json::Value> {
    let launcher = dbus_run_session()?;

    let mut command = std::process::Command::new(launcher);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("share"))
        .env("XDG_DATA_DIRS", home.join("sys-share"))
        .env("XDG_RUNTIME_DIR", home);
    for (key, value) in extra {
        command.env(key, value);
    }

    let output = command
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_vicinae"))
        .arg("--socket")
        .arg(home.join("ipc.sock"))
        .args(["doctor", "--json"])
        .output()
        .expect("dbus-run-session should run");

    assert!(
        output.status.success(),
        "doctor exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    Some(serde_json::from_slice(&output.stdout).expect("doctor --json must emit valid JSON"))
}

fn statuses(value: &serde_json::Value) -> BTreeMap<String, String> {
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

fn detail(value: &serde_json::Value, name: &str) -> String {
    value["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check named {name}"))["detail"]
        .as_str()
        .expect("detail")
        .to_string()
}

#[test]
fn a_real_but_empty_session_bus_is_reachable_and_reported_as_such() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(report) = doctor_on_a_private_bus(dir.path(), &[]) else {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    };

    let by_name = statuses(&report);
    assert_eq!(
        by_name["dbus.session"],
        "ok",
        "a real session bus should be reported as reachable: {}",
        detail(&report, "dbus.session")
    );
    assert!(detail(&report, "dbus.session").contains("DBUS_SESSION_BUS_ADDRESS="));
}

#[test]
fn an_empty_bus_reports_the_portal_as_absent_not_as_a_bus_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(report) = doctor_on_a_private_bus(dir.path(), &[]) else {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    };

    let by_name = statuses(&report);
    assert_eq!(by_name["portal.desktop"], "fail");
    let portal = detail(&report, "portal.desktop");
    assert!(
        portal.contains("not installed or not running"),
        "should be diagnosed as absence, got: {portal}"
    );
    assert!(
        !portal.contains("could not ask") && !portal.contains("could not query"),
        "a reachable bus must not be reported as unqueryable: {portal}"
    );

    assert_eq!(by_name["portal.global-shortcuts"], "fail");
    let shortcuts = detail(&report, "portal.global-shortcuts");
    assert!(
        shortcuts.contains("not implemented by the running portal backend"),
        "should be diagnosed as absence, got: {shortcuts}"
    );
}

#[test]
fn a_gnome_session_without_shell_is_not_blamed_on_the_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(report) = doctor_on_a_private_bus(dir.path(), &[("XDG_CURRENT_DESKTOP", "GNOME")])
    else {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    };

    let by_name = statuses(&report);
    assert_eq!(by_name["desktop.environment"], "warn");
    assert!(detail(&report, "desktop.environment").contains("did not answer ShellVersion"));

    assert_eq!(by_name["gnome.shell-extension"], "warn");
    let extension = detail(&report, "gnome.shell-extension");
    assert!(
        extension.contains("nobody owns org.gnome.Shell"),
        "wrong diagnosis with GNOME Shell absent: {extension}"
    );
    // Whatever the cause, the cost must still be spelled out precisely.
    assert!(extension.contains("window switching, clipboard history and paste"));
}

#[test]
fn the_private_bus_is_never_the_developers_own_bus() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(report) = doctor_on_a_private_bus(dir.path(), &[]) else {
        eprintln!("skipping: dbus-run-session is not installed");
        return;
    };

    // If this ever picked up a real desktop session, a portal would answer.
    assert_eq!(statuses(&report)["portal.desktop"], "fail");
    assert!(
        report["socket"]
            .as_str()
            .expect("socket")
            .starts_with(dir.path().to_str().expect("utf-8 tempdir")),
        "the socket under test must live in the tempdir"
    );
}
