//! `compass data remove`, run as the real binary over a fixture home.
//!
//! Every child runs with `env_clear()`, `HOME` and the XDG base variables
//! pointing into a `TempDir`, and `--socket` naming a path in it, so nothing
//! here can reach the developer's real directories or a live engine.

#![cfg(unix)]

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Output;

use compass_ipc::{Listener, Request, Response};
use tempfile::TempDir;

/// A fixture home: Compass's four directories, the compatibility link the
/// cutover leaves, an upstream Vicinae directory, a dotfiles-linked state
/// directory and neighbours that must survive.
struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Self {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        let config = home.config();
        let data = home.data();
        mkdir(&config.join("compass/themes"));
        std::fs::write(config.join("compass/compass.json"), "{}").expect("write");
        symlink("compass", config.join("vicinae")).expect("link");
        mkdir(&config.join("compass-notes"));
        std::fs::write(config.join("other.json"), "{}").expect("write");
        mkdir(&data.join("compass/extensions"));
        mkdir(&data.join("vicinae/extensions"));
        mkdir(&home.cache().join("compass"));
        let dotfiles = home.root().join("dotfiles/compass-state");
        mkdir(&dotfiles);
        std::fs::write(dotfiles.join("kept.log"), "kept").expect("write");
        mkdir(&home.state());
        symlink(&dotfiles, home.state().join("compass")).expect("link");
        home
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }
    fn config(&self) -> PathBuf {
        self.root().join(".config")
    }
    fn data(&self) -> PathBuf {
        self.root().join(".local/share")
    }
    fn cache(&self) -> PathBuf {
        self.root().join(".cache")
    }
    fn state(&self) -> PathBuf {
        self.root().join(".local/state")
    }
    fn socket(&self) -> PathBuf {
        self.root().join("run/compass/ipc.sock")
    }

    async fn run(&self, args: &[&str]) -> Output {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_compass"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root())
            .env("XDG_CONFIG_HOME", self.config())
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CACHE_HOME", self.cache())
            .env("XDG_STATE_HOME", self.state())
            .arg("--socket")
            .arg(self.socket())
            .args(args)
            .output()
            .await
            .expect("the CLI binary should run")
    }

    /// Whether something (a link included) is at `path`.
    fn has(path: &Path) -> bool {
        path.symlink_metadata().is_ok()
    }
}

fn mkdir(path: &Path) {
    std::fs::create_dir_all(path).expect("mkdir");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test]
async fn without_yes_it_lists_every_path_and_removes_nothing() {
    let home = Home::new();
    let output = home.run(&["data", "remove"]).await;
    assert!(output.status.success(), "{output:?}");
    let out = stdout(&output);

    for heading in [
        "Configuration ($XDG_CONFIG_HOME):",
        "Data ($XDG_DATA_HOME):",
        "Cache ($XDG_CACHE_HOME):",
        "State ($XDG_STATE_HOME):",
    ] {
        assert!(out.contains(heading), "missing {heading} in\n{out}");
    }
    let config = home.config();
    assert!(out.contains(&format!(
        "{} will be removed with everything in it.",
        config.join("compass").display()
    )));
    assert!(out.contains(&format!(
        "{} is the link to Compass's directory left at Vicinae's old name.",
        config.join("vicinae").display()
    )));
    assert!(out.contains(&format!(
        "{} belongs to Vicinae and will be kept.",
        home.data().join("vicinae").display()
    )));
    assert!(out.contains(&format!(
        "{} does not exist.",
        home.cache().join("vicinae").display()
    )));
    assert!(out.contains(&format!(
        "{} is a link to {}. The link will be removed; what it points to is kept.",
        home.state().join("compass").display(),
        home.root().join("dotfiles/compass-state").display()
    )));
    assert!(out.contains("Run `compass data remove --yes` to remove the paths above."));
    // Nothing outside the four bases is ever named.
    assert!(!out.contains("compass-notes"));
    assert!(!out.contains("other.json"));

    assert!(config.join("compass/compass.json").is_file());
    assert!(Home::has(&config.join("vicinae")));
    assert!(home.data().join("compass/extensions").is_dir());
    assert!(home.cache().join("compass").is_dir());
    assert!(Home::has(&home.state().join("compass")));
}

#[tokio::test]
async fn with_yes_it_removes_compass_s_paths_and_nothing_else() {
    let home = Home::new();
    let output = home.run(&["data", "remove", "--yes"]).await;
    assert!(output.status.success(), "{output:?}");
    let out = stdout(&output);
    assert!(out.contains(&format!(
        "Removed {}.",
        home.config().join("compass").display()
    )));
    assert!(!out.contains("Nothing has been removed yet."), "{out}");

    let config = home.config();
    assert!(!Home::has(&config.join("compass")));
    assert!(!Home::has(&config.join("vicinae")));
    assert!(!Home::has(&home.data().join("compass")));
    assert!(!Home::has(&home.cache().join("compass")));
    assert!(!Home::has(&home.state().join("compass")));

    assert!(config.join("compass-notes").is_dir());
    assert!(config.join("other.json").is_file());
    assert!(home.data().join("vicinae/extensions").is_dir());
    assert!(
        home.root()
            .join("dotfiles/compass-state/kept.log")
            .is_file()
    );
}

#[tokio::test]
async fn include_vicinae_takes_vicinae_s_directory_too() {
    let home = Home::new();
    let output = home
        .run(&["data", "remove", "--yes", "--include-vicinae"])
        .await;
    assert!(output.status.success(), "{output:?}");
    assert!(!Home::has(&home.data().join("vicinae")));
    assert!(home.config().join("compass-notes").is_dir());
}

#[tokio::test]
async fn it_refuses_while_an_engine_answers() {
    let home = Home::new();
    let socket = home.socket();
    mkdir(socket.parent().expect("parent"));
    let listener = Listener::bind(&socket).await.expect("bind");
    tokio::spawn(listener.serve(|request: Request| async move {
        match request {
            Request::Ping => Response::Pong {
                protocol_version: compass_ipc::PROTOCOL_VERSION,
                pid: 4242,
            },
            _ => Response::Ack,
        }
    }));

    let output = home.run(&["data", "remove", "--yes"]).await;
    assert!(!output.status.success(), "{output:?}");
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("Compass is running"), "{err}");
    assert!(home.config().join("compass/compass.json").is_file());
    assert!(home.data().join("compass/extensions").is_dir());
}
