//! What the runtime may touch while it runs a command: the Landlock policy
//! `compass-sandbox-exec` applies, and the per-extension directories it
//! grants writing to.
//!
//! Apart from starting a run because it is the security boundary: what an
//! extension can reach is decided here and nowhere else, and its tests read
//! as the list of what is granted.

use std::path::{Path, PathBuf};

use compass_core::extension_commands::ExtensionCommand;

use super::Runtime;

/// The files name resolution reads that live outside `etc` through a
/// symlink. Under systemd-resolved `/etc/resolv.conf` points into
/// `/run/systemd/resolve/`; granting `/etc` alone leaves the link unreadable,
/// and every lookup an extension makes fails with `EAI_AGAIN`.
fn resolver_targets(etc: &Path) -> Vec<PathBuf> {
    [
        "resolv.conf",
        "hosts",
        "nsswitch.conf",
        "host.conf",
        "gai.conf",
    ]
    .into_iter()
    .filter_map(|name| std::fs::canonicalize(etc.join(name)).ok())
    .filter(|target| !target.starts_with(etc))
    .collect()
}

/// What the runtime may touch while it runs `command`, with the user's
/// `$HOME` as the process has it; see [`policy_in`].
#[must_use]
pub fn policy(
    runtime: &Runtime,
    command: &ExtensionCommand,
    data_dir: &Path,
) -> compass_sandbox::Policy {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    policy_in(runtime, command, data_dir, home.as_deref())
}

/// What the runtime may touch while it runs `command`.
///
/// Read: the system (Node's libraries, certificates, ICU data, `/proc`), Node,
/// the bundle and the extension's own directory, and of `home` only
/// [`compass_sandbox::home::HOME_READ_ALLOWLIST`] (`~/.ssh/config`, the
/// password store, the Hyprland, Sway and niri configurations), read-only and
/// where it exists. Write: only the support and
/// asset directories the runtime creates for this extension. Not `/tmp`:
/// every other process's temporary files are there, so the extension gets its
/// own, [`tmp_dir`], as `TMPDIR`.
/// Execute: Node and the system's programs, so an extension that shells out
/// still can, inside the same boundary. Paths that do not exist are left out,
/// since Landlock cannot name them.
#[must_use]
pub fn policy_in(
    runtime: &Runtime,
    command: &ExtensionCommand,
    data_dir: &Path,
    home: Option<&Path>,
) -> compass_sandbox::Policy {
    let parent = |path: &Path| path.parent().map(Path::to_path_buf);
    let home_reads = home
        .map(compass_sandbox::home::home_reads)
        .unwrap_or_default();
    for (path, why) in &home_reads.refused {
        tracing::info!(
            path = %path.display(), ?why,
            "not granting an extension this allowlisted path"
        );
    }
    let read = [
        "/usr", "/etc", "/proc", "/sys", "/dev", "/lib", "/lib64", "/bin", "/app",
    ]
    .into_iter()
    .map(PathBuf::from)
    .chain(parent(&runtime.node))
    .chain(parent(&runtime.bundle))
    .chain([command.extension_dir.clone()])
    .chain(resolver_targets(Path::new("/etc")))
    .chain(home_reads.granted)
    // A certificate bundle the user pointed TLS at (a corporate CA, say):
    // Node reads it at start, and without it every fetch fails.
    .chain(
        ["NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE", "SSL_CERT_DIR"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from),
    );
    let write = [
        support_dir(data_dir, command),
        assets_dir(data_dir, command),
    ];
    // The system's trees whole, since a program run from `/usr/bin` may run
    // its own helpers from `/usr/lib` or `/usr/libexec` (git does). The
    // extension's installed directory too, for the few that ship a binary.
    // Never the directories it may write: a program it wrote there would be
    // any program it liked.
    let execute = ["/usr", "/bin", "/lib", "/lib64", "/app"]
        .into_iter()
        .map(PathBuf::from)
        .chain([runtime.node.clone(), command.extension_dir.clone()]);

    let mut policy =
        compass_sandbox::Policy::new().data_limit(compass_worker_host::cgroups::DATA_LIMIT_BYTES);
    for path in read.filter(|path| path.exists()) {
        policy = policy.read(path);
    }
    for path in write.into_iter().filter(|path| path.exists()) {
        policy = policy.read(path.clone()).write(path);
    }
    for path in execute.filter(|path| path.exists()) {
        policy = policy.execute(path);
    }
    policy
}

/// `<data>/support/<extension id>`, where the runtime keeps an extension's
/// support files and logs.
#[must_use]
pub fn support_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    data_dir.join("support").join(&command.extension_id)
}

/// `<support>/.tmp`, the extension's private temporary directory.
#[must_use]
pub fn tmp_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    support_dir(data_dir, command).join(".tmp")
}

/// `<data>/extensions/<extension id>/assets`, which the runtime creates.
#[must_use]
pub fn assets_dir(data_dir: &Path, command: &ExtensionCommand) -> PathBuf {
    data_dir
        .join("extensions")
        .join(&command.extension_id)
        .join("assets")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_in(extension_dir: &Path) -> ExtensionCommand {
        ExtensionCommand {
            id: "@me/ssh:hosts".to_owned(),
            provider_id: "@me/ssh".to_owned(),
            extension_id: "ssh".to_owned(),
            extension_dir: extension_dir.to_path_buf(),
            name: "hosts".to_owned(),
            title: "Hosts".to_owned(),
            extension_title: "SSH".to_owned(),
            keywords: Vec::new(),
            mode: compass_core::manifest::CommandMode::View,
            entrypoint: extension_dir.join("hosts.js"),
            default_disabled: false,
            extension_name: "ssh".to_owned(),
            author: "me".to_owned(),
            owner: None,
            is_raycast: false,
            preferences: Vec::new(),
            arguments: Vec::new(),
            icon: None,
            extension_icon: String::new(),
        }
    }

    #[test]
    fn the_policy_reads_only_the_allowlisted_home_paths_and_never_writes_them() {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).expect("ssh");
        std::fs::write(ssh.join("config"), "Host box\n").expect("config");
        std::fs::write(ssh.join("id_ed25519"), "PRIVATE\n").expect("key");
        std::fs::create_dir_all(home.join(".password-store")).expect("pass");
        std::fs::create_dir_all(home.join(".config/niri")).expect("niri");
        std::fs::create_dir_all(home.join(".gnupg")).expect("gnupg");
        std::fs::create_dir_all(home.join("Documents")).expect("documents");
        let extension = root.path().join("extensions/ssh");
        std::fs::create_dir_all(&extension).expect("extension");
        let data = root.path().join("data");
        let runtime = Runtime {
            node: PathBuf::from("/usr/bin/node"),
            bundle: root.path().join("runtime/runtime.js"),
            sandbox: None,
        };
        let command = command_in(&extension);
        std::fs::create_dir_all(support_dir(&data, &command)).expect("support");

        let policy = policy_in(&runtime, &command, &data, Some(&home));
        let home = std::fs::canonicalize(&home).expect("canonical home");
        for granted in [".ssh/config", ".password-store", ".config/niri"] {
            assert!(
                policy.read.contains(&home.join(granted)),
                "{granted} is readable: {:?}",
                policy.read
            );
        }
        let under_home = |paths: &[PathBuf]| -> Vec<PathBuf> {
            paths
                .iter()
                .filter(|path| path.starts_with(&home))
                .cloned()
                .collect()
        };
        assert_eq!(
            under_home(&policy.read),
            [
                home.join(".ssh/config"),
                home.join(".password-store"),
                home.join(".config/niri"),
            ],
            "nothing else of $HOME: not the keys, ~/.ssh, ~/.gnupg or the home itself"
        );
        assert!(under_home(&policy.write).is_empty(), "read-only");
        assert!(under_home(&policy.execute).is_empty(), "not executable");

        let without = policy_in(&runtime, &command, &data, None);
        assert!(under_home(&without.read).is_empty());
    }

    #[test]
    fn a_resolver_file_linked_out_of_etc_is_granted_where_it_points() {
        let root = tempfile::tempdir().expect("tempdir");
        let etc = root.path().join("etc");
        let run = root.path().join("run/systemd/resolve");
        std::fs::create_dir_all(&etc).expect("etc");
        std::fs::create_dir_all(&run).expect("run");
        std::fs::write(run.join("stub-resolv.conf"), "nameserver 127.0.0.53\n").expect("stub");
        std::os::unix::fs::symlink(run.join("stub-resolv.conf"), etc.join("resolv.conf"))
            .expect("link");
        std::fs::write(etc.join("hosts"), "127.0.0.1 localhost\n").expect("hosts");

        let targets = resolver_targets(&etc);
        assert_eq!(
            targets,
            [std::fs::canonicalize(run.join("stub-resolv.conf")).expect("canonical")],
            "only the linked-out file, not what /etc already covers"
        );
    }
}
