//! The Arch package's install script: what it does to running engines and to
//! users' data on upgrade and removal (`packaging/README.md`, "Upgrades and
//! removal").
//!
//! The script is run for real under `sh`, with every system command it calls
//! replaced by a stub on `PATH` that records its arguments. The stubs describe
//! a machine with three users: alice runs Compass as `compass.service`, bob
//! runs it from his desktop's autostart (two processes), and carol runs the
//! Flatpak, whose executable is not this package's.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn packaging() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging")
}

fn install_script() -> PathBuf {
    packaging().join("arch/compass.install")
}

const STUBS: &[(&str, &str)] = &[
    (
        "pgrep",
        r#"echo "pgrep $*" >> "$STUB_LOG"; for p in $STUB_PIDS; do echo "$p"; done"#,
    ),
    (
        "readlink",
        r#"pid=${1#/proc/}; pid=${pid%/exe}; eval "printf '%s\n' \"\$STUB_EXE_$pid\"""#,
    ),
    (
        "stat",
        r#"for a in "$@"; do last=$a; done; pid=${last#/proc/}; eval "echo \"\$STUB_UID_$pid\"""#,
    ),
    (
        "id",
        r#"for a in "$@"; do last=$a; done; eval "echo \"\$STUB_USER_$last\"""#,
    ),
    (
        "systemctl",
        r#"echo "systemctl $*" >> "$STUB_LOG"
case " $* " in
  *" is-active "*)
    for a in "$@"; do case $a in --machine=*) u=${a#--machine=}; u=${u%@.host};; esac; done
    case " $STUB_ACTIVE " in *" $u "*) exit 0;; *) exit 3;; esac;;
esac"#,
    ),
    (
        "runuser",
        r#"echo "runuser $*" >> "$STUB_LOG"; [ -z "$STUB_RUNUSER_FAILS" ]"#,
    ),
    ("setcap", r#"echo "setcap $*" >> "$STUB_LOG""#),
];

struct Machine {
    dir: TempDir,
    env: Vec<(&'static str, String)>,
}

impl Machine {
    /// alice (1000) under compass.service, bob (1001) twice from autostart,
    /// carol (1002) in the Flatpak. alice's binary is shown as replaced, as
    /// it is after an upgrade.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        for (name, body) in STUBS {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write stub");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
        let env = vec![
            ("STUB_PIDS", "101 102 103 104".to_owned()),
            ("STUB_EXE_101", "/usr/bin/compass (deleted)".to_owned()),
            ("STUB_EXE_102", "/usr/bin/compass".to_owned()),
            ("STUB_EXE_103", "/usr/bin/compass".to_owned()),
            ("STUB_EXE_104", "/app/bin/compass".to_owned()),
            ("STUB_UID_101", "1000".to_owned()),
            ("STUB_UID_102", "1001".to_owned()),
            ("STUB_UID_103", "1001".to_owned()),
            ("STUB_UID_104", "1002".to_owned()),
            ("STUB_USER_1000", "alice".to_owned()),
            ("STUB_USER_1001", "bob".to_owned()),
            ("STUB_USER_1002", "carol".to_owned()),
            ("STUB_ACTIVE", "alice".to_owned()),
        ];
        Self { dir, env }
    }

    fn set(mut self, key: &'static str, value: &str) -> Self {
        self.env.push((key, value.to_owned()));
        self
    }

    /// Runs one of the script's functions, as pacman does, and returns its
    /// output and every command the stubs saw.
    fn run(&self, function: &str) -> (Output, Vec<String>) {
        let log = self.dir.path().join("log");
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(". \"$1\" && {function} 1.0-1 0.9-1"))
            .arg("sh")
            .arg(install_script())
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.dir.path().join("bin").display()),
            )
            .env("STUB_LOG", &log)
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .output()
            .expect("sh runs");
        let calls = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.starts_with("pgrep "))
            .map(str::to_owned)
            .collect();
        (output, calls)
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn removal_stops_each_users_engine_through_their_own_manager_or_socket() {
    let (output, calls) = Machine::new().run("pre_remove");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        calls,
        [
            "systemctl --user --machine=alice@.host --quiet is-active compass.service",
            "systemctl --user --machine=alice@.host stop compass.service",
            "systemctl --user --machine=bob@.host --quiet is-active compass.service",
            "runuser -u bob -- env -i XDG_RUNTIME_DIR=/run/user/1001 /usr/bin/compass shutdown",
        ]
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn removal_says_so_when_an_engine_would_not_stop() {
    let (output, _) = Machine::new()
        .set("STUB_RUNUSER_FAILS", "1")
        .run("pre_remove");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        stdout(&output),
        "Compass could not be stopped for bob. It stops when they log out.\n"
    );
}

#[test]
fn an_upgrade_restarts_the_service_and_tells_everyone_else() {
    let (output, calls) = Machine::new().run("post_upgrade");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        calls,
        [
            "setcap cap_dac_override+ep /usr/lib/compass/compass-input-server",
            "systemctl --user --machine=alice@.host --quiet is-active compass.service",
            "systemctl --user --machine=alice@.host daemon-reload",
            "systemctl --user --machine=alice@.host try-restart compass.service",
            "systemctl --user --machine=bob@.host --quiet is-active compass.service",
        ]
    );
    assert_eq!(
        stdout(&output),
        "Compass is still running the previous version for bob. Run `compass shutdown` and \
         then start Compass again to use the new one.\n"
    );
}

#[test]
fn nothing_running_means_nothing_done() {
    let machine = Machine::new().set("STUB_PIDS", "");
    for function in ["pre_remove", "post_upgrade"] {
        let (output, calls) = machine.run(function);
        assert!(output.status.success(), "{output:?}");
        assert!(
            calls.iter().all(|call| call.starts_with("setcap ")),
            "{calls:?}"
        );
        assert_eq!(stdout(&output), "");
    }
}

#[test]
fn removal_keeps_user_data_and_says_where_it_is() {
    let (output, calls) = Machine::new().run("post_remove");
    assert!(output.status.success(), "{output:?}");
    assert!(calls.is_empty(), "{calls:?}");
    let out = stdout(&output);
    for path in [
        "~/.config/compass",
        "~/.local/share/compass",
        "~/.cache/compass",
        "~/.local/state/compass",
        "compass data remove --yes",
    ] {
        assert!(out.contains(path), "{path} missing from\n{out}");
    }
}

#[test]
fn the_script_never_deletes_or_signals() {
    let script = std::fs::read_to_string(install_script()).expect("read compass.install");
    let code: Vec<&str> = script
        .lines()
        .map(str::trim_start)
        .filter(|line| !line.starts_with('#'))
        .collect();
    for line in &code {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ';' || c == '(' || c == '`')
            .collect();
        for forbidden in ["rm", "rmdir", "unlink", "kill", "pkill", "killall", "shred"] {
            assert!(
                !words.contains(&forbidden),
                "compass.install runs `{forbidden}`: {line}"
            );
        }
        assert!(
            !line.contains("$HOME") && !line.contains("/home/"),
            "{line}"
        );
    }
    for function in ["post_install", "post_upgrade", "pre_remove", "post_remove"] {
        assert!(
            code.iter()
                .any(|line| line.starts_with(&format!("{function}() {{"))),
            "compass.install defines no {function}"
        );
    }
    let syntax = Command::new("sh")
        .arg("-n")
        .arg(install_script())
        .output()
        .expect("sh runs");
    assert!(syntax.status.success(), "{syntax:?}");

    let pkgbuild = std::fs::read_to_string(packaging().join("arch/PKGBUILD")).expect("PKGBUILD");
    assert!(
        pkgbuild
            .lines()
            .any(|line| line == "install=compass.install")
    );
}

#[test]
fn the_packaging_docs_state_the_rules() {
    let readme = std::fs::read_to_string(packaging().join("README.md")).expect("README");
    for needle in [
        "## Upgrades and removal",
        "compass data remove",
        "--include-vicinae",
        "flatpak uninstall --delete-data org.tunaos.compass",
    ] {
        assert!(
            readme.contains(needle),
            "packaging/README.md lacks {needle}"
        );
    }
}
