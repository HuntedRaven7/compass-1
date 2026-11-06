//! `compass clipboard reset` and `compass doctor`'s clipboard checks, run as the
//! real binary over a fixture home with no keyring.
//!
//! Every child runs with `env_clear()`, `HOME` and `XDG_DATA_HOME` in a
//! `TempDir`, a session bus address nothing listens on and `--socket` naming a
//! path in it, so nothing here can reach the developer's keyring, history or a
//! live engine.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Output;

use compass_ipc::{Listener, Request, Response};
use tempfile::TempDir;

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn history_dir(&self) -> PathBuf {
        self.root().join("share/compass")
    }

    fn socket(&self) -> PathBuf {
        self.root().join("ipc.sock")
    }

    /// A history database whose key nobody has: what a reset keyring leaves.
    fn with_history(self) -> Self {
        std::fs::create_dir_all(self.history_dir().join("compass-clipboard")).expect("mkdir");
        std::fs::write(
            self.history_dir().join("compass-clipboard.db"),
            [0x5a; 4096],
        )
        .expect("write");
        self
    }

    async fn run(&self, args: &[&str]) -> Output {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_compass"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root())
            .env("XDG_DATA_HOME", self.root().join("share"))
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", self.root().join("no-bus").display()),
            )
            .arg("--socket")
            .arg(self.socket())
            .args(args)
            .output()
            .await
            .expect("the CLI binary should run")
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn with_no_history_there_is_nothing_to_reset() {
    let home = Home::new();
    let output = home.run(&["clipboard", "reset", "--yes"]).await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(stdout(&output), "There is no clipboard history to reset.\n");
    assert!(!home.history_dir().exists(), "nothing was created");
}

#[tokio::test]
async fn without_a_keyring_a_history_is_never_touched() {
    let home = Home::new().with_history();
    for args in [
        &["clipboard", "reset"][..],
        &["clipboard", "reset", "--yes"][..],
    ] {
        let output = home.run(args).await;
        assert!(!output.status.success(), "{output:?}");
        let err = stderr(&output);
        assert!(err.contains("No keyring answered"), "{err}");
        assert!(home.history_dir().join("compass-clipboard.db").is_file());
        assert!(home.history_dir().join("compass-clipboard").is_dir());
    }
}

#[tokio::test]
async fn yes_is_refused_while_an_engine_answers() {
    let home = Home::new().with_history();
    let listener = Listener::bind(&home.socket()).await.expect("bind");
    tokio::spawn(listener.serve(|request: Request| async move {
        match request {
            Request::Ping => Response::Pong {
                protocol_version: compass_ipc::PROTOCOL_VERSION,
                pid: 4242,
            },
            _ => Response::Ack,
        }
    }));
    let output = home.run(&["clipboard", "reset", "--yes"]).await;
    assert!(!output.status.success(), "{output:?}");
    assert!(stderr(&output).contains("Compass is running"), "{output:?}");
    assert!(home.history_dir().join("compass-clipboard.db").is_file());
}

#[tokio::test]
async fn doctor_reports_the_history_it_could_not_check_and_its_size() {
    let home = Home::new().with_history();
    let output = home.run(&["doctor", "--json"]).await;
    let report: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("json");
    let storage = report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|check| check["name"] == "clipboard.storage")
        .expect("a clipboard.storage check");
    assert_eq!(storage["status"], "warn", "{storage}");
    let detail = storage["detail"].as_str().expect("detail");
    assert!(detail.contains("No keyring answered"), "{detail}");
    assert!(detail.contains("Its database takes 4096 bytes"), "{detail}");
    assert_eq!(
        std::fs::read(home.history_dir().join("compass-clipboard.db")).expect("read"),
        [0x5a; 4096],
        "the doctor changed nothing"
    );
}
