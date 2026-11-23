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

/// The extra read and execute grants a Node installation needs beyond the
/// system trees (#407). Homebrew's `bin/node` is a symlink into
/// `Cellar/node/<version>/bin/node`, where its shared libraries live one
/// directory up, and its OpenSSL configuration lives under the prefix's `etc`.
/// Granting only the symlink's parent leaves all three unreadable.
fn node_grants(node: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let resolved = std::fs::canonicalize(node).unwrap_or_else(|_| node.to_path_buf());
    let mut read = Vec::new();
    let mut execute = Vec::new();
    for path in [node.to_path_buf(), resolved.clone()] {
        execute.push(path.clone());
        if let Some(bin) = path.parent() {
            execute.push(bin.to_path_buf());
            read.push(bin.to_path_buf());
            // Homebrew keeps `lib/` beside `bin/` under the version directory.
            if let Some(version) = bin.parent() {
                read.push(version.to_path_buf());
            }
        }
        read.push(path);
    }
    if let Some(config) = std::env::var_os("OPENSSL_CONF") {
        read.push(PathBuf::from(config));
    }
    for prefix in ["/home/linuxbrew/.linuxbrew", "/opt/homebrew", "/usr/local"] {
        let etc = Path::new(prefix).join("etc");
        let Ok(entries) = std::fs::read_dir(&etc) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("openssl@") {
                read.push(entry.path().join("openssl.cnf"));
            }
        }
    }
    (read, execute)
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
    let (node_read, node_execute) = node_grants(&runtime.node);
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
    .chain(node_read)
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
        .chain(node_execute)
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

/// The variables of the engine's environment a worker is given; everything
/// else is withheld.
///
/// What is left out matters more than what is kept: `DBUS_SESSION_BUS_ADDRESS`,
/// `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DISPLAY`, `SSH_AUTH_SOCK` and
/// `COMPASS_SOCKET` each name a socket outside the filesystem boundary, and
/// `NODE_OPTIONS` would change what Node loads before the bundle does. The
/// syscall filter already refuses the socket; not handing over the address is
/// the second layer, and the one that still holds where no filter is applied.
const WORKER_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LANGUAGE",
    "TZ",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];

/// The part of `vars` a worker is given: [`WORKER_ENV`] and the locale's
/// `LC_*`.
#[must_use]
pub fn worker_env<I>(vars: I) -> Vec<(std::ffi::OsString, std::ffi::OsString)>
where
    I: IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
{
    vars.into_iter()
        .filter(|(key, _)| {
            key.to_str().is_some_and(|key| {
                WORKER_ENV.contains(&key)
                    || key.strip_prefix("LC_").is_some_and(|rest| !rest.is_empty())
            })
        })
        .collect()
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

    #[test]
    fn the_worker_is_not_handed_the_session_bus_or_any_other_socket() {
        let engine = [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/me"),
            ("LANG", "en_GB.UTF-8"),
            ("LC_TIME", "C"),
            ("LC_", "x"),
            ("https_proxy", "http://proxy:3128"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh-agent"),
            ("COMPASS_SOCKET", "/run/user/1000/compass/ipc.sock"),
            ("NODE_OPTIONS", "--require /tmp/x.js"),
        ]
        .map(|(key, value)| (key.into(), value.into()));
        let given: Vec<String> = worker_env(engine)
            .into_iter()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        assert_eq!(given, ["PATH", "HOME", "LANG", "LC_TIME", "https_proxy"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_node_grants_its_real_binary_and_homebrew_libraries() {
        let root = tempfile::tempdir().expect("tempdir");
        let version = root.path().join("Cellar/node/26.13.1_2");
        let bin = version.join("bin");
        let lib = version.join("lib");
        std::fs::create_dir_all(&bin).expect("bin");
        std::fs::create_dir_all(&lib).expect("lib");
        std::fs::write(bin.join("node"), "binary").expect("node");
        std::fs::write(lib.join("libnode.so.147"), "library").expect("libnode");
        let prefix_bin = root.path().join("bin");
        std::fs::create_dir_all(&prefix_bin).expect("prefix bin");
        std::os::unix::fs::symlink(bin.join("node"), prefix_bin.join("node")).expect("symlink");
        let link = prefix_bin.join("node");

        let (read, execute) = super::node_grants(&link);
        let resolved = std::fs::canonicalize(&link).expect("resolved node");
        assert!(
            execute.contains(&resolved),
            "the real binary is executable: {execute:?}"
        );
        assert!(
            execute.contains(&bin),
            "its bin directory is executable too"
        );
        assert!(
            read.contains(&version),
            "the version dir holding lib/ is readable: {read:?}"
        );
        assert!(
            read.contains(&prefix_bin),
            "the symlink's directory is readable"
        );
        assert!(read.contains(&resolved), "the real binary is readable");
    }
}
