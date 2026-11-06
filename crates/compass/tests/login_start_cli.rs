//! `compass login-start`, the real binary against a real engine, on the path
//! with no Background portal: outside the Flatpak and with no session bus,
//! which is a wlroots session without a portal backend. The engine writes and
//! removes its own `$XDG_CONFIG_HOME/autostart/compass.desktop`.
//!
//! The portal path is the tier-2 install journey's
//! (`scripts/tier2/install-journey.sh`), against the real
//! `xdg-desktop-portal`; `compass-platform-linux`'s `login_start` tests cover
//! it against a mock portal.
//!
//! Every process runs with `env_clear()` and a temporary home, runtime
//! directory and socket, so nothing here reaches the developer's session.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const NO_SESSION_BUS: &str = "unix:path=/nonexistent/compass-test-no-session-bus";
const WAIT: Duration = Duration::from_secs(30);

struct Session {
    dir: TempDir,
    engine: Option<Child>,
}

impl Session {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        for sub in ["run", "empty"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
        }
        Self { dir, engine: None }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn autostart(&self) -> PathBuf {
        self.root().join(".config/autostart/compass.desktop")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root())
            .env("XDG_CONFIG_HOME", self.root().join(".config"))
            .env("XDG_DATA_HOME", self.root().join(".local/share"))
            .env("XDG_CACHE_HOME", self.root().join(".cache"))
            .env("XDG_STATE_HOME", self.root().join(".local/state"))
            .env("XDG_RUNTIME_DIR", self.root().join("run"))
            .env("XDG_DATA_DIRS", self.root().join("empty"))
            .env("DBUS_SESSION_BUS_ADDRESS", NO_SESSION_BUS)
            .env("COMPASS_DISABLE_AUTO_RATE_REFRESH", "1")
            .env(
                "COMPASS_UPDATE_FEED_URL",
                "http://127.0.0.1:9/releases/latest",
            )
            .arg("--socket")
            .arg(self.root().join("run/compass.sock"));
        command
    }

    fn cli(&self, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("the CLI binary should run")
    }

    fn start_engine(&mut self) {
        let child = self
            .command()
            .args(["serve", "--no-hotkey"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the engine should start");
        self.engine = Some(child);
        let deadline = Instant::now() + WAIT;
        while !self.cli(&["ping"]).status.success() {
            assert!(
                Instant::now() < deadline,
                "the engine never answered a ping"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// `compass login-start <args> --json`: the exit status and the JSON.
    fn login_start(&self, verb: &str) -> (bool, serde_json::Value) {
        let output = self.cli(&["login-start", verb, "--json"]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json = serde_json::from_str(&stdout).unwrap_or_else(|error| {
            panic!(
                "`login-start {verb} --json` printed no JSON ({error}): {stdout}\nstderr: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.success(), json)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut engine) = self.engine.take() {
            let _ = self.cli(&["shutdown"]);
            let deadline = Instant::now() + Duration::from_secs(5);
            while engine.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = engine.kill();
            let _ = engine.wait();
        }
    }
}

#[test]
fn without_a_portal_the_switch_writes_and_removes_compass_s_own_autostart_entry() {
    let mut session = Session::new();
    session.start_engine();

    let (ok, status) = session.login_start("status");
    assert!(ok);
    assert_eq!(status["state"], "off", "{status}");
    assert!(!session.autostart().exists());

    let (ok, status) = session.login_start("enable");
    assert!(ok, "enable should succeed: {status}");
    assert_eq!(status["state"], "on", "{status}");
    assert_eq!(status["mechanism"], "autostart-file", "{status}");
    let entry = std::fs::read_to_string(session.autostart()).expect("the autostart entry");
    assert!(entry.contains("\nExec=compass start --hidden\n"), "{entry}");
    assert!(entry.contains("\nX-Compass-Login-Start=true\n"), "{entry}");

    // The answer outlives the engine: a restart reads it back.
    let (_, again) = session.login_start("status");
    assert_eq!(again["mechanism"], "autostart-file", "{again}");

    let (ok, status) = session.login_start("disable");
    assert!(ok, "disable should succeed: {status}");
    assert_eq!(status["state"], "off", "{status}");
    assert!(!session.autostart().exists(), "disable left the entry");

    let human = session.cli(&["login-start", "status"]);
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8_lossy(&human.stdout).trim(),
        "Compass does not start when you log in."
    );
}

#[test]
fn someone_else_s_autostart_entry_is_left_alone_and_enable_fails() {
    let mut session = Session::new();
    let foreign = "[Desktop Entry]\nType=Application\nName=Mine\nExec=compass start\n";
    std::fs::create_dir_all(session.autostart().parent().expect("parent")).expect("mkdir");
    std::fs::write(session.autostart(), foreign).expect("write");
    session.start_engine();

    let output = session.cli(&["login-start", "enable"]);
    assert!(
        !output.status.success(),
        "enable must fail over a foreign entry"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("was not written by Compass"),
        "the refusal should say why: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(session.autostart()).expect("still there"),
        foreign
    );

    let (ok, _) = session.login_start("disable");
    assert!(ok);
    assert_eq!(
        std::fs::read_to_string(session.autostart()).expect("still there"),
        foreign,
        "disable must not remove an entry Compass did not write"
    );
}

#[test]
fn with_no_engine_it_says_so_and_fails() {
    let session = Session::new();
    let output = session.cli(&["login-start", "status"]);
    assert!(!output.status.success());
    assert!(!session.autostart().exists());
}
