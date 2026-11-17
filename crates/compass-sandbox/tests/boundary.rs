//! Does the boundary actually bite?
//!
//! Every negative test here has a positive control beside it, because the
//! dangerous outcome is not "the sandbox blocked too much" but "the sandbox
//! blocked nothing and every log said it applied". A denial only means
//! something if the identical operation succeeds without the policy.
//!
//! These run the real launcher against real processes. `compass-sandbox-exec`
//! confines the process it runs in for good, so it cannot be exercised
//! in-process: a test that applied a policy would confine the test binary and
//! every test after it.

use std::path::{Path, PathBuf};
use std::process::Command;

use compass_sandbox::{Mode, Policy};

/// The launcher, as cargo built it for this test.
fn launcher() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_compass-sandbox-exec"))
}

/// Somewhere to read from, and something outside it.
struct Fixture {
    _dir: tempfile::TempDir,
    allowed: PathBuf,
    inside: PathBuf,
    outside: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let allowed = dir.path().join("allowed");
    std::fs::create_dir(&allowed).expect("the allowed directory");
    let inside = allowed.join("inside.txt");
    std::fs::write(&inside, b"inside\n").expect("the inside file");
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"outside\n").expect("the outside file");
    Fixture {
        _dir: dir,
        allowed,
        inside,
        outside,
    }
}

/// `cat`, wherever it is on this system.
fn cat() -> PathBuf {
    for candidate in ["/usr/bin/cat", "/bin/cat"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    panic!("no cat on this system; these tests need a real program to exec");
}

/// Whether this kernel enforces Landlock at all.
///
/// Not a skip switch. On a kernel without Landlock a strict policy *must* fail
/// closed, and that is what the tests below assert instead of asserting a
/// denial they cannot produce — the property worth checking on such a machine
/// is that nothing runs unconfined, not that reads are blocked by a feature
/// that is not there.
fn landlock_enforces() -> bool {
    let f = fixture();
    let (ok, stderr) = run(
        &runnable().read(&f.allowed),
        &cat(),
        &[f.inside.to_string_lossy().into_owned()],
    );
    if ok {
        return true;
    }
    assert!(
        stderr.contains("enforced") || stderr.contains("Landlock"),
        "the launcher failed for a reason that is not the kernel: {stderr}"
    );
    eprintln!(
        "note: this kernel does not enforce Landlock ({stderr}); the boundary tests assert \
         fail-closed instead of denial"
    );
    false
}

/// Asserts that a strict policy refuses to run anything here.
fn assert_fails_closed() {
    let f = fixture();
    let (ok, _) = run(
        &runnable().read(&f.allowed),
        &cat(),
        &[f.inside.to_string_lossy().into_owned()],
    );
    assert!(
        !ok,
        "a strict policy ran a program on a kernel that does not enforce it"
    );
}

/// Runs `program` behind `policy` and returns (success, stderr).
fn run(policy: &Policy, program: &Path, args: &[String]) -> (bool, String) {
    let output = policy
        .command(&launcher(), program, args)
        .output()
        .expect("the launcher runs");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The policy a `cat` needs in order to run at all: the binary and the
/// libraries it loads.
fn can_run_cat() -> Policy {
    Policy::new()
        .read("/usr")
        .read("/lib")
        .read("/lib64")
        .execute("/usr")
        .execute("/lib")
        .execute("/lib64")
}

/// As [`can_run_cat`], dropping paths this system does not have.
fn runnable() -> Policy {
    let mut policy = Policy::new();
    for path in ["/usr", "/lib", "/lib64", "/bin"] {
        if Path::new(path).exists() {
            policy = policy.read(path).execute(path);
        }
    }
    policy
}

#[test]
fn the_control_passes_without_a_policy_at_all() {
    // If this fails, every denial below is meaningless: it would mean `cat`
    // cannot read the file for reasons that have nothing to do with Landlock.
    let f = fixture();
    let plain = Command::new(cat())
        .arg(&f.outside)
        .output()
        .expect("cat runs");
    assert!(
        plain.status.success(),
        "the unsandboxed control failed: {}",
        String::from_utf8_lossy(&plain.stderr)
    );
}

#[test]
fn a_read_inside_the_policy_is_allowed() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    let f = fixture();
    let policy = runnable().read(&f.allowed);
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(
        ok,
        "a read of an allowed path was refused; the policy is too tight to prove anything: \
         {stderr}"
    );
}

#[test]
fn a_read_outside_the_policy_is_denied() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // The assertion. Same launcher, same `cat`, same fixture as the test
    // above; the only difference is which path is being read.
    let f = fixture();
    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(&policy, &cat(), &[f.outside.to_string_lossy().into_owned()]);
    assert!(
        !ok,
        "a read outside the policy succeeded: the ruleset applied and confined nothing"
    );
}

#[test]
fn a_write_to_a_read_only_path_is_denied() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // `read` grants reading, not writing. `sh -c` is used rather than `cat`
    // because a redirect is the shortest way to ask for a write.
    let f = fixture();
    let Some(sh) = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    else {
        panic!("no sh on this system");
    };

    let target = f.allowed.join("new.txt");
    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("echo x > {}", target.to_string_lossy()),
        ],
    );
    assert!(!ok, "a write to a read-only path succeeded");
    assert!(!target.exists(), "the file was created anyway");

    // Control: the same write, with the same everything, where the policy
    // grants writing.
    let policy = runnable().write(&f.allowed);
    let (ok, stderr) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("echo x > {}", target.to_string_lossy()),
        ],
    );
    assert!(ok, "the write control failed: {stderr}");
    assert!(target.exists(), "the write control wrote nothing");
}

#[test]
fn a_program_the_policy_does_not_allow_cannot_be_run() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // Execute is separate from read on purpose: a worker that may read its
    // own directory is not thereby allowed to run anything in it.
    let f = fixture();
    let policy = Policy::new().read(&f.allowed);
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(
        !ok,
        "a program outside the policy's execute paths was run anyway: {stderr}"
    );
}

#[test]
fn a_policy_that_names_a_missing_path_refuses_before_running_anything() {
    // No kernel branch: the check happens before the ruleset is built, so it
    // holds whether or not Landlock is enforced here.
    // Otherwise the rule would contribute nothing and the policy would read as
    // granting access it does not grant.
    let f = fixture();
    let policy = runnable().read(&f.allowed).read("/definitely/not/here");
    let (ok, stderr) = run(&policy, &cat(), &[f.inside.to_string_lossy().into_owned()]);
    assert!(!ok, "a policy naming a missing path was applied anyway");
    assert!(
        stderr.contains("/definitely/not/here"),
        "the refusal must name the path: {stderr}"
    );
}

#[test]
fn nothing_after_the_separator_is_refused_rather_than_silently_doing_nothing() {
    let output = Command::new(launcher())
        .args(["--read", "/usr", "--"])
        .output()
        .expect("the launcher runs");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("nothing to run"),
        "the launcher should say what was wrong"
    );
}

#[test]
fn an_unknown_flag_is_refused() {
    let output = Command::new(launcher())
        .args(["--allow-everything", "--", "/bin/true"])
        .output()
        .expect("the launcher runs");
    assert!(
        !output.status.success(),
        "an unknown flag must not be ignored: a policy that quietly skips one is weaker than \
         it reads"
    );
}

#[test]
fn the_policy_applies_to_a_grandchild_too() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // Landlock is inherited, and the worker will spawn things. `sh -c` runs
    // `cat` as its own child; the boundary has to hold there as well, or
    // every confinement is one `execve` deep.
    let f = fixture();
    let Some(sh) = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    else {
        panic!("no sh on this system");
    };

    let policy = runnable().read(&f.allowed);
    let (ok, _) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("cat {}", f.outside.to_string_lossy()),
        ],
    );
    assert!(!ok, "a grandchild read a path the policy denies");

    let (ok, stderr) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("cat {}", f.inside.to_string_lossy()),
        ],
    );
    assert!(ok, "the grandchild control failed: {stderr}");
}

#[test]
fn best_effort_is_not_the_default() {
    // A strict policy on a kernel that will not enforce it fails; this test
    // cannot force that kernel, so what it checks is that the flag has to be
    // asked for, in the arguments that actually reach the launcher.
    let strict = can_run_cat();
    assert_eq!(strict.mode, Mode::Strict);
    assert!(!strict.args().contains(&"--best-effort".to_owned()));
    assert!(
        strict
            .mode(Mode::BestEffort)
            .args()
            .contains(&"--best-effort".to_owned())
    );
}

/// A binary on this system, or `None`.
fn binary(name: &str) -> Option<PathBuf> {
    ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.exists())
}

/// The probe, as cargo built it for this test.
fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_compass-sandbox-probe"))
}

/// Runs the probe behind `policy` and returns the errno it printed.
fn probe_errno(policy: &Policy) -> String {
    let output = policy
        .command(&launcher(), &probe(), &[])
        .output()
        .expect("the launcher runs");
    assert!(
        output.status.success(),
        "the probe did not run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn a_denied_syscall_answers_eperm_while_the_same_call_gets_through_without_the_filter() {
    // `ptrace(PTRACE_ATTACH, 0)` cannot succeed: there is no process 0. That
    // is the point -- without the filter the kernel answers ESRCH, having
    // looked at the argument; behind the filter it answers EPERM, having not.
    // Two errnos from one call, so "it failed" cannot be mistaken for "it was
    // denied", and no privilege, namespace or second process is involved.
    let policy = runnable()
        .read(probe().parent().expect("the probe has a directory"))
        .execute(probe().parent().expect("the probe has a directory"));

    // The control, unsandboxed.
    let plain = Command::new(probe()).output().expect("the probe runs");
    assert_eq!(
        String::from_utf8_lossy(&plain.stdout).trim(),
        "errno=ESRCH",
        "the unsandboxed control did not answer ESRCH, so this machine cannot answer the \
         question"
    );

    // The second control: same policy, filter off. Separates a syscall denial
    // from Landlock denying something on the way.
    assert_eq!(
        probe_errno(&policy.clone().without_syscall_filter()),
        "errno=ESRCH",
        "the call did not get through with the filter off, so a denial below would prove nothing"
    );

    // The assertion.
    assert_eq!(
        probe_errno(&policy),
        "errno=EPERM",
        "ptrace was not denied behind the syscall filter: the filter is installed and inert"
    );
}

#[test]
fn ptrace_is_refused_while_ordinary_work_continues() {
    // A second witness, from a real tool rather than our own probe, on the
    // machines that have one. The test above is the one that must hold
    // everywhere.
    let Some(strace) = binary("strace") else {
        eprintln!("note: no strace on this system; skipping the ptrace half");
        return;
    };
    let Some(r#true) = binary("true") else {
        panic!("no true on this system");
    };

    let args = [
        "-o".to_owned(),
        "/dev/null".to_owned(),
        r#true.to_string_lossy().into_owned(),
    ];

    let permissive = runnable().read("/dev/null").write("/dev/null");
    // A loud skip rather than an assertion, and only here: this test is the
    // second witness, and whether strace can attach at all depends on
    // /proc/sys/kernel/yama/ptrace_scope and on the machine's own policy. The
    // test above is the one that has to hold everywhere, and it does not
    // depend on any of that.
    let (ok, stderr) = run(&permissive.clone().without_syscall_filter(), &strace, &args);
    if !ok {
        eprintln!("note: strace cannot attach here ({stderr}); skipping the second witness");
        return;
    }

    let (ok, _) = run(&permissive, &strace, &args);
    assert!(!ok, "strace attached behind the syscall filter");

    // The control that matters most: with the filter on, a program that makes
    // no denied call still runs. A filter that denied everything would pass
    // the assertion above and be useless.
    let f = fixture();
    let (ok, stderr) = run(
        &runnable().read(&f.allowed),
        &cat(),
        &[f.inside.to_string_lossy().into_owned()],
    );
    assert!(
        ok,
        "an ordinary program could not run behind the syscall filter: {stderr}"
    );
}

/// Runs the probe in `mode` behind `policy`, returning the errno it printed.
fn probe_mode(policy: &Policy, mode: &str) -> String {
    let output = policy
        .command(&launcher(), &probe(), &[mode.to_owned()])
        .output()
        .expect("the launcher runs");
    assert!(
        output.status.success(),
        "the probe did not run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The unconfined answer for `mode`.
fn probe_plain(mode: &str) -> String {
    let output = Command::new(probe())
        .arg(mode)
        .output()
        .expect("the probe runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// What the probe needs to run at all.
fn probe_policy() -> Policy {
    let dir = probe()
        .parent()
        .expect("the probe has a directory")
        .to_owned();
    runnable().read(&dir).execute(dir)
}

#[test]
fn a_raw_socket_is_refused_while_an_ordinary_one_is_not() {
    // Suite 1's negative list (§8.2) names raw sockets. The kernel refuses an
    // unprivileged one already, so the probe asks for raw protocol 0, which
    // the kernel rejects with EPROTONOSUPPORT *before* it checks
    // CAP_NET_RAW: that errno means the call reached the socket layer, root
    // or not, and EPERM means the filter stopped it first.
    assert_eq!(
        probe_plain("raw-socket"),
        "errno=EPROTONOSUPPORT",
        "the unsandboxed control did not reach the socket layer, so this machine cannot \
         answer the question"
    );
    assert_eq!(
        probe_mode(&probe_policy().without_syscall_filter(), "raw-socket"),
        "errno=EPROTONOSUPPORT",
        "the call did not get through with the filter off, so a denial below would prove nothing"
    );
    assert_eq!(
        probe_mode(&probe_policy(), "raw-socket"),
        "errno=EPERM",
        "a raw socket was not refused behind the syscall filter"
    );
    // A packet socket is raw at the link layer in every type it has. Only a
    // privileged control can tell the filter from the kernel here; where the
    // control opens one, the sandbox must not.
    assert_eq!(
        probe_mode(&probe_policy(), "packet-socket"),
        "errno=EPERM",
        "an AF_PACKET socket was opened"
    );
    if probe_plain("packet-socket") != "errno=ok" {
        eprintln!("note: AF_PACKET is refused here unconfined too; its denial is the kernel's");
    }
    // The control that matters most: the rules are about raw sockets, not
    // sockets. Every fetch an extension makes starts with this call.
    assert_eq!(
        probe_mode(&probe_policy(), "stream-socket"),
        "errno=ok",
        "an ordinary TCP socket was refused: the filter would break every extension"
    );
}

#[test]
fn a_unix_socket_outside_every_grant_cannot_be_reached() {
    // The session bus and the engine's IPC socket are pathname sockets that
    // no grant names, and Landlock V1 does not look at `connect`. A worker
    // that reached either would be out of the boundary without opening a
    // file, so the filter refuses the socket itself.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("bus.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&path).expect("a listening socket");
    let mode = format!("unix-connect={}", path.display());
    assert_eq!(
        probe_plain(&mode),
        "errno=ok",
        "the unsandboxed control could not connect, so a denial below would prove nothing"
    );
    assert_eq!(
        probe_mode(&probe_policy().without_syscall_filter(), &mode),
        "errno=ok",
        "with the filter off the connect should get through: Landlock V1 does not mediate it"
    );
    assert_eq!(
        probe_mode(&probe_policy(), &mode),
        "errno=EPERM",
        "a sandboxed worker connected to a unix socket outside its policy"
    );
    // Node makes a child process's stdio with `socketpair`; refusing that
    // would break every extension that runs a program.
    assert_eq!(
        probe_mode(&probe_policy(), "unix-socketpair"),
        "errno=ok",
        "socketpair was refused: an extension could no longer run a child process"
    );
}

#[test]
fn a_program_the_worker_wrote_itself_cannot_be_run() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    // Suite 1's `fork` case (§8.2). Spawning is allowed on purpose: an
    // extension that wraps a CLI is ordinary. What must fail closed is the
    // escape, a worker that writes a program into the one directory it may
    // write and then runs it, which would be every binary it liked.
    let f = fixture();
    let Some(sh) = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    else {
        panic!("no sh on this system");
    };
    let dropped = f.allowed.join("dropped");
    let script = format!(
        "cp {cat} {dropped} && chmod +x {dropped} && {dropped} {inside}",
        cat = cat().to_string_lossy(),
        dropped = dropped.to_string_lossy(),
        inside = f.inside.to_string_lossy()
    );
    let policy = runnable().read(&f.allowed).write(&f.allowed);
    let (ok, _) = run(&policy, &sh, &["-c".to_owned(), script.clone()]);
    assert!(
        !ok,
        "a program the worker copied into its own directory ran"
    );
    assert!(
        dropped.exists(),
        "the copy itself failed, so the refusal above was not the exec being denied"
    );

    // Control: with the directory executable, the same script runs.
    std::fs::remove_file(&dropped).expect("remove the copy");
    let (ok, stderr) = run(&policy.execute(&f.allowed), &sh, &["-c".to_owned(), script]);
    assert!(ok, "the exec control failed: {stderr}");
}

#[test]
fn an_allocation_past_the_data_limit_fails_and_the_process_carries_on() {
    // Suite 1's 512 MB case (§8.2). The heap flag bounds JavaScript objects
    // and a cgroup bounds the rest only where systemd is reachable, which is
    // not inside a Flatpak; RLIMIT_DATA holds everywhere. The probe reserves
    // 512 MiB with `try_reserve`, so a refusal comes back as an error it
    // prints rather than an abort.
    assert_eq!(
        probe_plain("alloc-512m"),
        "errno=ok",
        "the unconfined control could not reserve 512 MiB"
    );
    assert_eq!(
        probe_mode(&probe_policy(), "alloc-512m"),
        "errno=ok",
        "without a data limit the reservation should succeed, or the refusal below proves nothing"
    );
    assert_eq!(
        probe_mode(&probe_policy().data_limit(256 * 1024 * 1024), "alloc-512m"),
        "errno=ENOMEM",
        "512 MiB was reserved under a 256 MiB data limit"
    );
}

/// A home with the allowlisted paths, a key beside `~/.ssh/config`, other
/// files, and a compositor configuration linked out of the list.
fn a_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("a home");
    let root = home.path();
    std::fs::create_dir_all(root.join(".ssh")).unwrap();
    std::fs::write(root.join(".ssh/config"), "Host box\n").unwrap();
    std::fs::write(root.join(".ssh/id_ed25519"), "PRIVATE\n").unwrap();
    std::fs::create_dir_all(root.join(".password-store")).unwrap();
    std::fs::write(root.join(".password-store/web.gpg"), "sealed\n").unwrap();
    std::fs::create_dir_all(root.join(".config/hypr")).unwrap();
    std::fs::write(root.join(".config/hypr/hyprland.conf"), "bind = x\n").unwrap();
    std::fs::write(root.join("notes.txt"), "mine\n").unwrap();
    std::fs::create_dir_all(root.join("secrets")).unwrap();
    std::fs::write(root.join("secrets/token"), "TOKEN\n").unwrap();
    std::os::unix::fs::symlink(root.join("secrets"), root.join(".config/sway")).unwrap();
    home
}

#[test]
fn an_extension_reads_the_home_allowlist_and_nothing_else_of_home() {
    if !landlock_enforces() {
        return assert_fails_closed();
    }
    let home = a_home();
    let root = home.path();
    let mut policy = runnable();
    for path in compass_sandbox::home::home_reads(root).granted {
        policy = policy.read(path);
    }
    let read = |path: &Path| run(&policy, &cat(), &[path.to_string_lossy().into_owned()]);

    for allowed in [
        root.join(".ssh/config"),
        root.join(".password-store/web.gpg"),
        root.join(".config/hypr/hyprland.conf"),
    ] {
        let (ok, stderr) = read(&allowed);
        assert!(ok, "{} is on the allowlist: {stderr}", allowed.display());
    }
    for denied in [
        root.join(".ssh/id_ed25519"),
        root.join("notes.txt"),
        root.join("secrets/token"),
        // Through the link: its target is not on the list.
        root.join(".config/sway/token"),
    ] {
        // Control: readable without the policy, so a denial is the policy's.
        assert!(std::fs::read(&denied).is_ok(), "{}", denied.display());
        let (ok, _) = read(&denied);
        assert!(!ok, "{} was read through the sandbox", denied.display());
    }

    let sh = ["/usr/bin/sh", "/bin/sh"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .expect("sh");
    let target = root.join(".password-store/new.gpg");
    let (ok, _) = run(
        &policy,
        &sh,
        &[
            "-c".to_owned(),
            format!("echo x > {}", target.to_string_lossy()),
        ],
    );
    assert!(!ok && !target.exists(), "the password store is read-only");
}
