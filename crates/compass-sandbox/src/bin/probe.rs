//! Makes one call the sandbox should refuse and says what the kernel answered.
//!
//! The boundary tests need a program that issues a denied call on purpose.
//! Nothing on a stock system does that reliably: `unshare` needs namespace
//! privileges the machine may not give (a GitHub runner does not), and
//! `strace` is not installed everywhere. So this is that program.
//!
//! Each call is chosen so that the answer *without* a filter is an errno the
//! filter never gives, which is what makes "denied" distinguishable from
//! "failed":
//!
//! - `ptrace` (the default): `ptrace(PTRACE_ATTACH, 0)`. Process 0 does not
//!   exist, so unfiltered the kernel answers `ESRCH` having looked at the
//!   argument; behind the filter, `EPERM` before it looks.
//! - `raw-socket`: `socket(AF_INET, SOCK_RAW, 0)`. Unfiltered the kernel
//!   finds no raw protocol 0 and answers `EPROTONOSUPPORT` before it checks
//!   `CAP_NET_RAW`, root or not; behind the filter, `EPERM`.
//! - `packet-socket`: `socket(AF_PACKET, SOCK_DGRAM, 0)`. Unfiltered this is
//!   `EPERM` for an unprivileged process and a socket for root, so only a
//!   privileged control can tell the two apart; the tests say so.
//! - `stream-socket`: `socket(AF_INET, SOCK_STREAM, 0)`, which must work
//!   behind the filter: the control that the socket rules are not a ban on
//!   sockets.
//! - `unix-connect=<path>`: `socket(AF_UNIX, SOCK_STREAM, 0)` and `connect`
//!   to `path`. Unfiltered, with something listening there, this succeeds;
//!   behind the filter the `socket` call answers `EPERM`.
//! - `unix-socketpair`: `socketpair(AF_UNIX, SOCK_STREAM, 0)`, which must work
//!   behind the filter: it is how Node makes a child's stdio pipes, and the
//!   pair it returns can reach nothing but itself.
//! - `alloc-512m`: reserves 512 MiB, answering `ENOMEM` when the data limit
//!   refuses it.
//!
//! It exits 0 either way and prints `errno=<name>`, or `errno=ok` when the
//! call succeeded: the test reads the name, because "the program failed" is
//! exactly the ambiguity being avoided.

use nix::sys::ptrace;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket, socketpair};
use nix::unistd::Pid;

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ptrace".to_owned());
    let answer = match mode.as_str() {
        // `Pid::from_raw(0)` is the "attach to pid 0" that cannot succeed.
        "ptrace" => ptrace::attach(Pid::from_raw(0)).map(drop),
        "raw-socket" => {
            socket(AddressFamily::Inet, SockType::Raw, SockFlag::empty(), None).map(drop)
        }
        "packet-socket" => socket(
            AddressFamily::Packet,
            SockType::Datagram,
            SockFlag::empty(),
            None,
        )
        .map(drop),
        "stream-socket" => socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .map(drop),
        "unix-socketpair" => socketpair(
            AddressFamily::Unix,
            SockType::Stream,
            None,
            SockFlag::empty(),
        )
        .map(drop),
        unix if unix.starts_with("unix-connect=") => {
            let path = &unix["unix-connect=".len()..];
            UnixAddr::new(path).and_then(|address| {
                let fd = socket(
                    AddressFamily::Unix,
                    SockType::Stream,
                    SockFlag::empty(),
                    None,
                )?;
                connect(std::os::fd::AsRawFd::as_raw_fd(&fd), &address)
            })
        }
        // `try_reserve` rather than an allocation, so a refusal is an error
        // to print rather than an abort; the reservation is a private
        // writable mapping, which is what RLIMIT_DATA counts.
        "alloc-512m" => {
            let mut block: Vec<u8> = Vec::new();
            block
                .try_reserve_exact(512 * 1024 * 1024)
                .map_err(|_| nix::errno::Errno::ENOMEM)
        }
        other => {
            eprintln!("compass-sandbox-probe: unknown mode {other}");
            std::process::exit(64);
        }
    };
    match answer {
        Ok(()) => println!("errno=ok"),
        Err(errno) => println!("errno={errno:?}"),
    }
}
