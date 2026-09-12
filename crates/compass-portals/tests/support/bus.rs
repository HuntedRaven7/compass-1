//! A private D-Bus session bus, one per test.
//!
//! Tests must not depend on an ambient bus: `DBUS_SESSION_BUS_ADDRESS` may be
//! absent in CI, and sharing a bus between tests makes them order-dependent.
//! Each [`TestBus`] spawns its own `dbus-daemon --session` and hands out its
//! address, which is passed explicitly through
//! [`PortalConfig::with_address`](compass_portals::PortalConfig::with_address).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

/// A running private `dbus-daemon`.
pub struct TestBus {
    child: Child,
    address: String,
}

impl TestBus {
    /// Start a private session bus.
    ///
    /// Returns `None` when there is no `dbus-daemon` on this machine, so the
    /// caller can skip loudly rather than pass silently.
    pub fn start() -> Option<Self> {
        let mut child = match Command::new("dbus-daemon")
            .args(["--session", "--print-address", "--nofork"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => panic!("failed to spawn dbus-daemon: {err}"),
        };

        let stdout = child.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("dbus-daemon should print its address");
        let address = line.trim().to_owned();
        assert!(
            address.starts_with("unix:"),
            "unexpected dbus-daemon address: {address:?}"
        );

        Some(Self { child, address })
    }

    /// The bus address to connect to.
    pub fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for TestBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start a private bus, or return `None` after a loud message.
pub fn start_or_skip(test_name: &str) -> Option<TestBus> {
    match TestBus::start() {
        Some(bus) => Some(bus),
        None => {
            eprintln!(
                "\n\
                 ############################################################\n\
                 # SKIPPED {test_name}: no `dbus-daemon` binary on PATH.\n\
                 # compass-portals' portal tests exercise a REAL session bus\n\
                 # and cannot be meaningfully faked. Install dbus to run them\n\
                 # (Fedora: dbus-daemon, Debian/Ubuntu: dbus-bin).\n\
                 ############################################################\n"
            );
            None
        }
    }
}
