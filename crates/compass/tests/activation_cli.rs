//! A command that needs the engine brings it up itself, with no service, no
//! login-start entry and no window, on a machine with no graphical session.
//!
//! The spawned engine is detached from this test: it is stopped with
//! `compass shutdown` at the end, and every path lives under a temporary
//! directory with a cleared environment.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

struct Session {
    dir: TempDir,
    socket: PathBuf,
}

impl Session {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("run")).expect("mkdir");
        let socket = dir.path().join("run/compass.sock");
        Self { dir, socket }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join(".config"))
            .env("XDG_DATA_HOME", self.dir.path().join(".local/share"))
            .env("XDG_CACHE_HOME", self.dir.path().join(".cache"))
            .env("XDG_STATE_HOME", self.dir.path().join(".local/state"))
            .env("XDG_RUNTIME_DIR", self.dir.path().join("run"))
            .env("XDG_DATA_DIRS", self.dir.path().join("empty"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent/compass-test-no-session-bus",
            )
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env(
                "COMPASS_UPDATE_FEED_URL",
                "http://127.0.0.1:9/releases/latest",
            )
            .arg("--socket")
            .arg(&self.socket);
        command
    }

    fn cli(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("the CLI binary should run")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.cli(&["shutdown"]);
    }
}

#[test]
fn query_cold_starts_a_headless_engine_and_it_stays_up_after_the_command_exits() {
    let session = Session::new();

    let output = session.cli(&["query", "--json", "firefox"]);
    assert!(
        output.status.success(),
        "query should start the engine it needs: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hits: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("query --json printed JSON");
    assert!(hits.is_array(), "query printed {hits}");

    // The engine outlived the short-lived query that started it.
    let ping = session.cli(&["ping"]);
    assert!(
        ping.status.success(),
        "the engine should still be answering: {}",
        String::from_utf8_lossy(&ping.stderr)
    );
    assert!(String::from_utf8_lossy(&ping.stdout).contains("engine alive"));
}

#[test]
fn a_later_client_reuses_the_engine_a_cold_command_started() {
    let session = Session::new();
    assert!(
        session
            .cli(&["query", "--json", "firefox"])
            .status
            .success()
    );

    // A second query must not start another engine on the same socket. If it
    // tried to, bind() would fail and this command would report the error.
    let again = session.cli(&["query", "--json", "firefox"]);
    assert!(
        again.status.success(),
        "the second query should reuse the engine: {}",
        String::from_utf8_lossy(&again.stderr)
    );
}

#[test]
fn the_cold_engine_is_owned_by_no_client_and_stops_on_shutdown() {
    let session = Session::new();
    assert!(
        session
            .cli(&["query", "--json", "firefox"])
            .status
            .success()
    );

    let shutdown = session.cli(&["shutdown"]);
    assert!(
        shutdown.status.success(),
        "shutdown should reach the cold engine: {}",
        String::from_utf8_lossy(&shutdown.stderr)
    );
    let ping = session.cli(&["ping"]);
    assert!(!ping.status.success(), "the engine should have stopped");
    assert!(!Path::new(&session.socket).exists());
}
