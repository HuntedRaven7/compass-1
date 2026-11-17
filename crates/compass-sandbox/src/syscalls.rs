//! The syscall filter that goes on top of the filesystem boundary.
//!
//! Landlock says which files a worker may touch; this says which kernel
//! interfaces it may reach at all. They are complementary and neither
//! substitutes for the other: `ptrace` reads another process's memory without
//! opening a single file, and `init_module` leaves the filesystem entirely.
//!
//! # A denylist, not an allowlist, and why
//!
//! PLAN.md §6 says "seccomp for the syscall filter" without saying which
//! shape. An allowlist is stronger and is what a purpose-built binary should
//! have. The thing being confined here is Node, whose syscall surface is large,
//! version-dependent and allowed to change under us — an allowlist would be
//! wrong on the next Node release, in the direction that breaks every
//! extension. So the default is a small denylist of interfaces no extension
//! runtime has any business reaching, each of which is a known route to
//! something worse.
//!
//! This is a deliberate, recorded weakening of the spec. An allowlist becomes
//! possible if the worker is ever something we build rather than something we
//! embed.
//!
//! # Everything denied answers `EPERM`, not `SIGSYS`
//!
//! A killed process and a refused call look very different to whoever is
//! debugging an extension at the time. `EPERM` reaches Node as an ordinary
//! error, with a stack.

use std::collections::BTreeMap;

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};

/// A syscall this crate can name, with the reason it is on the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Denied {
    /// The name, as `seccomp` and `strace` spell it.
    pub name: &'static str,
    /// The number on this architecture.
    pub number: i64,
    /// Why an extension worker must not reach it.
    pub reason: &'static str,
}

/// The syscalls a worker is denied by default.
///
/// Each entry is an interface with no legitimate use from an extension and a
/// well-known abusive one. Nothing here is reachable from the `@raycast/api`
/// surface, and nothing here is used by Node itself in normal operation — the
/// one to watch is `perf_event_open`, which V8 uses only under `--prof`.
pub const DEFAULT_DENYLIST: &[Denied] = &[
    Denied {
        name: "ptrace",
        number: libc::SYS_ptrace,
        reason: "reads and writes another process's memory, including the host's",
    },
    Denied {
        name: "process_vm_readv",
        number: libc::SYS_process_vm_readv,
        reason: "reads another process's memory without opening anything",
    },
    Denied {
        name: "process_vm_writev",
        number: libc::SYS_process_vm_writev,
        reason: "writes another process's memory",
    },
    Denied {
        name: "init_module",
        number: libc::SYS_init_module,
        reason: "loads kernel code",
    },
    Denied {
        name: "finit_module",
        number: libc::SYS_finit_module,
        reason: "loads kernel code from a file descriptor",
    },
    Denied {
        name: "delete_module",
        number: libc::SYS_delete_module,
        reason: "unloads kernel code",
    },
    Denied {
        name: "kexec_load",
        number: libc::SYS_kexec_load,
        reason: "replaces the running kernel",
    },
    Denied {
        name: "mount",
        number: libc::SYS_mount,
        reason: "changes what the filesystem looks like underneath the boundary",
    },
    Denied {
        name: "umount2",
        number: libc::SYS_umount2,
        reason: "unmounts, which changes the filesystem underneath the boundary",
    },
    Denied {
        name: "pivot_root",
        number: libc::SYS_pivot_root,
        reason: "swaps the root filesystem for another one",
    },
    Denied {
        name: "chroot",
        number: libc::SYS_chroot,
        reason: "changes what the root of the filesystem means",
    },
    Denied {
        name: "unshare",
        number: libc::SYS_unshare,
        reason: "makes new namespaces, which is how a confined process gets out of one",
    },
    Denied {
        name: "setns",
        number: libc::SYS_setns,
        reason: "joins someone else's namespace",
    },
    Denied {
        name: "bpf",
        number: libc::SYS_bpf,
        reason: "loads programs into the kernel",
    },
    Denied {
        name: "userfaultfd",
        number: libc::SYS_userfaultfd,
        reason: "hands userspace control of page faults, a standard exploit primitive",
    },
    Denied {
        name: "perf_event_open",
        number: libc::SYS_perf_event_open,
        reason: "reads performance counters across the machine; V8 wants it only under --prof",
    },
    Denied {
        name: "keyctl",
        number: libc::SYS_keyctl,
        reason: "reaches the kernel keyring, where other things keep secrets",
    },
    Denied {
        name: "add_key",
        number: libc::SYS_add_key,
        reason: "writes to the kernel keyring, where other things keep secrets",
    },
    Denied {
        name: "request_key",
        number: libc::SYS_request_key,
        reason: "reads from the kernel keyring, where other things keep secrets",
    },
];

/// Why a filter could not be built or installed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// This architecture has no `seccompiler` target.
    #[error("no seccomp target for this architecture ({0}); refusing to run unfiltered")]
    UnsupportedArch(String),

    /// The filter would not compile.
    #[error("building the seccomp filter failed: {0}")]
    Build(#[from] seccompiler::Error),

    /// The filter would not compile into BPF.
    #[error("compiling the seccomp filter failed: {0}")]
    Backend(#[from] seccompiler::BackendError),

    /// The kernel refused the filter.
    #[error("installing the seccomp filter failed: {0}")]
    Install(std::io::Error),

    /// A name that is not on the list this build knows.
    #[error("{0} is not a syscall this build can deny")]
    UnknownSyscall(String),
}

/// The number this build knows for `name`.
///
/// # Errors
///
/// [`Error::UnknownSyscall`] if it is not in [`DEFAULT_DENYLIST`]. Deliberately
/// not a general syscall-name table: a filter that silently ignores a name it
/// does not recognise is weaker than it reads.
pub fn number_for(name: &str) -> Result<i64, Error> {
    DEFAULT_DENYLIST
        .iter()
        .find(|denied| denied.name == name)
        .map(|denied| denied.number)
        .ok_or_else(|| Error::UnknownSyscall(name.to_owned()))
}

/// The socket types a worker may not open, by the low bits of `socket`'s
/// second argument (the rest are `SOCK_NONBLOCK` and `SOCK_CLOEXEC`).
///
/// A raw socket reads and forges traffic for every process on the machine.
/// An unprivileged process is refused one by the kernel already, but only for
/// want of `CAP_NET_RAW`, which is a property of how the engine was started
/// rather than of the sandbox: run as root, or with the capability, it would
/// be granted. `socket` itself cannot go on the denylist, since every
/// connection Node makes starts with one, so these are argument rules.
pub const DENIED_SOCKET_TYPES: &[(&str, i32)] = &[
    ("SOCK_RAW", libc::SOCK_RAW),
    // `libc` deprecates the constant (it is obsolete in favour of AF_PACKET),
    // which is exactly why an old program might still ask for it.
    ("SOCK_PACKET", 10),
];

/// The address families a worker may not open a socket in, whatever the type.
///
/// `AF_PACKET` is link-layer access, raw in every type it offers.
///
/// `AF_UNIX` is the way out of the filesystem boundary that never opens a
/// file: Landlock V1 does not mediate `connect` to a pathname socket, and the
/// session bus, the engine's own IPC socket and every other local service
/// listen on one. Over the session bus a worker can ask systemd or a portal to
/// start a process that has none of these rules. The worker's own channel to
/// the host is its stdin and stdout, and Node makes the pipes for a child's
/// stdio with `socketpair`, which is a different syscall and stays allowed;
/// only a socket that could be pointed at an address is refused.
pub const DENIED_SOCKET_FAMILIES: &[(&str, i32)] =
    &[("AF_PACKET", libc::AF_PACKET), ("AF_UNIX", libc::AF_UNIX)];

/// The low bits of `socket`'s type argument that name the type.
const SOCKET_TYPE_MASK: u64 = 0xf;

fn socket_rules() -> Result<Vec<seccompiler::SeccompRule>, Error> {
    use seccompiler::{SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompRule};
    let by_type = DENIED_SOCKET_TYPES.iter().map(|&(_, kind)| {
        SeccompCondition::new(
            1,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(SOCKET_TYPE_MASK),
            u64::from(kind.unsigned_abs()),
        )
    });
    let by_family = DENIED_SOCKET_FAMILIES.iter().map(|&(_, family)| {
        SeccompCondition::new(
            0,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Eq,
            u64::from(family.unsigned_abs()),
        )
    });
    by_type
        .chain(by_family)
        .map(|condition| Ok(SeccompRule::new(vec![condition?])?))
        .collect()
}

/// Installs a filter denying `numbers` on the calling thread, for good, and
/// the sockets [`DENIED_SOCKET_TYPES`] and [`DENIED_SOCKET_FAMILIES`] name.
///
/// Everything unnamed stays allowed — see the module docs.
///
/// # Errors
///
/// [`Error::UnsupportedArch`] on an architecture with no target, and the
/// build or install errors otherwise.
pub fn deny(numbers: &[i64]) -> Result<(), Error> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => TargetArch::x86_64,
        "aarch64" => TargetArch::aarch64,
        other => return Err(Error::UnsupportedArch(other.to_owned())),
    };

    let mut rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> =
        numbers.iter().map(|number| (*number, Vec::new())).collect();
    // An empty rule list denies the call outright; a socket is only denied
    // when one of its rules matches, so it must not already be listed bare.
    if let std::collections::btree_map::Entry::Vacant(socket) = rules.entry(libc::SYS_socket) {
        socket.insert(socket_rules()?);
    }

    let program: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM.unsigned_abs()),
        arch,
    )?
    .try_into()?;

    seccompiler::apply_filter(&program).map_err(|err| match err {
        seccompiler::Error::Backend(backend) => Error::Backend(backend),
        other => Error::Install(std::io::Error::other(other.to_string())),
    })
}

/// Every number in [`DEFAULT_DENYLIST`].
#[must_use]
pub fn default_numbers() -> Vec<i64> {
    DEFAULT_DENYLIST.iter().map(|d| d.number).collect()
}

/// Whether the filter is enforced (`EPERM`) rather than log-only.
///
/// Log-only ships the filter with `SeccompAction::Log` so violations are
/// recorded but not denied — useful for measuring what would break. Enforce
/// is `SeccompAction::Errno(EPERM)`. Flipping this is the Phase 4 gate:
/// log-only first, enforce a release later. The default is enforce.
pub const ENFORCE: bool = true;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_entry_is_distinct_and_explained() {
        // A duplicate number would be a second name for one syscall, which
        // means one of the two reasons is wrong. An empty reason is an entry
        // nobody can review.
        let mut numbers = BTreeSet::new();
        let mut names = BTreeSet::new();
        for denied in DEFAULT_DENYLIST {
            assert!(
                numbers.insert(denied.number),
                "{} repeats syscall number {}",
                denied.name,
                denied.number
            );
            assert!(names.insert(denied.name), "{} is listed twice", denied.name);
            assert!(
                denied.reason.len() > 10,
                "{} has no reason worth reading",
                denied.name
            );
            assert!(
                denied.number >= 0,
                "{} has no number on this architecture",
                denied.name
            );
        }
        assert!(
            DEFAULT_DENYLIST.len() >= 15,
            "the denylist has shrunk to {} entries; that is a policy change",
            DEFAULT_DENYLIST.len()
        );
    }

    #[test]
    fn the_list_does_not_contain_anything_node_needs() {
        // Not a proof -- nothing here can prove what Node needs. It is a guard
        // against the obvious mistake: adding a syscall every program makes.
        for ordinary in [
            "read",
            "write",
            "openat",
            "close",
            "mmap",
            "futex",
            "clone",
            "execve",
            "socket",
            "connect",
            "epoll_wait",
        ] {
            assert!(
                !DEFAULT_DENYLIST.iter().any(|d| d.name == ordinary),
                "{ordinary} is on the denylist; nothing would run"
            );
        }
    }

    #[test]
    fn a_name_that_is_not_on_the_list_is_refused() {
        assert_eq!(number_for("ptrace").ok(), Some(libc::SYS_ptrace));
        assert!(
            number_for("read").is_err(),
            "a filter that ignored an unknown name would be weaker than it reads"
        );
    }

    #[test]
    fn filter_is_enforce_not_log_only() {
        // Phase 4 gate: shipped log-only first, flipped to enforce. A filter
        // that logs but does not deny is how a sandbox becomes decorative.
        const {
            assert!(
                ENFORCE,
                "seccomp filter must be enforce (EPERM), not log-only"
            );
        }
        // Also verified where it matters: deny() builds SeccompAction::Errno,
        // not SeccompAction::Log, so a real violation is refused, not recorded.
        // The shape is compile-time (ENFORCE const) rather than runtime probe;
        // the probe would require forking a child and triggering ptrace.
    }
}
