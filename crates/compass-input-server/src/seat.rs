//! Who may use the helper's privilege: the person at the keyboard.
//!
//! The capability is a file capability, so it is granted to whoever runs the
//! file, and the helper speaks to whoever holds its stdin. Without a check,
//! another account on the machine could run it to read every keyboard and to
//! type into the focused window. So a privileged helper serves only a user
//! whose logind session is the active one on a seat, and asks again while it
//! runs: after a switch to another user it stops reading keys and refuses to
//! inject, and starts again when its own user is back.
//!
//! Unprivileged, the helper can open nothing its caller could not, and none
//! of this applies; that is also how the tests run it.
//!
//! The system bus is reached at its fixed address, never through
//! `DBUS_SYSTEM_BUS_ADDRESS`: whoever runs the helper chooses its
//! environment, and could point it at a bus that answers "active". Where no
//! logind answers there (a seatd-only system) the helper cannot tell, says
//! so, and relies on who the install lets run it (`packaging/README.md`).

use std::time::{Duration, Instant};

use zbus::zvariant::OwnedObjectPath;

/// `CAP_DAC_OVERRIDE`'s bit in `CapEff`.
const CAP_DAC_OVERRIDE: u32 = 1;

/// The system bus's socket, where the D-Bus specification fixes it.
const SYSTEM_BUS_SOCKET: &str = "/var/run/dbus/system_bus_socket";

/// How long an answer is trusted before logind is asked again.
pub const RECHECK: Duration = Duration::from_secs(1);

/// What `/proc/self/status` says about this process: its real user, and
/// whether it holds `CAP_DAC_OVERRIDE` (or is root, who needs no capability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    /// The real user id: who ran the helper, whatever the file grants.
    pub uid: u32,
    /// Whether the process can open what its user could not.
    pub privileged: bool,
}

/// Reads [`Credentials`] from the text of `/proc/self/status`.
#[must_use]
pub fn credentials(status: &str) -> Option<Credentials> {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
    };
    let mut ids = field("Uid:")?.split_whitespace();
    let uid: u32 = ids.next()?.parse().ok()?;
    let euid: u32 = ids.next()?.parse().ok()?;
    let effective = u64::from_str_radix(field("CapEff:")?, 16).ok()?;
    Some(Credentials {
        uid,
        privileged: euid == 0 || effective & (1 << CAP_DAC_OVERRIDE) != 0,
    })
}

/// A session as logind lists it: its user, its seat (empty for a remote
/// one), and whether it is the one in front on that seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The session's user.
    pub uid: u32,
    /// The seat, or empty for an SSH or other seatless session.
    pub seat: String,
    /// Whether it is the active session of its seat.
    pub active: bool,
}

/// Whether `uid` is at a keyboard: it has the active session on some seat.
#[must_use]
pub fn at_the_keyboard(uid: u32, sessions: &[Session]) -> bool {
    sessions
        .iter()
        .any(|session| session.uid == uid && !session.seat.is_empty() && session.active)
}

/// What the gate decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Serve: unprivileged, or the caller has the active session.
    Allowed,
    /// Do not read keys or inject: the caller is not at the keyboard.
    Refused,
    /// Privileged, and no logind to ask; the install's permissions decide.
    Unknown,
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LoginSession {
    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;
}

/// Asks logind, and remembers the answer for [`RECHECK`].
#[derive(Debug)]
pub struct Gate {
    credentials: Option<Credentials>,
    runtime: Option<tokio::runtime::Runtime>,
    connection: Option<zbus::Connection>,
    last: Option<(Instant, Verdict)>,
}

impl Gate {
    /// A gate for this process.
    #[must_use]
    pub fn new() -> Self {
        let credentials = std::fs::read_to_string("/proc/self/status")
            .ok()
            .as_deref()
            .and_then(credentials);
        Self {
            credentials,
            runtime: None,
            connection: None,
            last: None,
        }
    }

    /// Whether to serve now, asking logind when the last answer is stale.
    pub fn verdict(&mut self) -> Verdict {
        if let Some((at, verdict)) = self.last
            && at.elapsed() < RECHECK
        {
            return verdict;
        }
        let verdict = self.ask();
        self.last = Some((Instant::now(), verdict));
        verdict
    }

    /// Whether keys may be read and injected now.
    pub fn allows(&mut self) -> bool {
        self.verdict() != Verdict::Refused
    }

    fn ask(&mut self) -> Verdict {
        let credentials = match self.credentials {
            Some(credentials) if !credentials.privileged => return Verdict::Allowed,
            Some(credentials) => credentials,
            // Unreadable status: assume the worst about the privilege, and
            // there is no user to look for.
            None => return Verdict::Refused,
        };
        if self.runtime.is_none() {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => self.runtime = Some(runtime),
                Err(error) => {
                    eprintln!("could not start the logind client: {error}");
                    return Verdict::Refused;
                }
            }
        }
        let Some(runtime) = &self.runtime else {
            return Verdict::Refused;
        };
        let connection = &mut self.connection;
        runtime.block_on(async {
            if connection.is_none() {
                match connect().await {
                    Ok(made) => *connection = Some(made),
                    // No system bus at all: nothing to ask. One that is there
                    // and will not answer is not taken as absent, since a
                    // caller can starve the helper of descriptors.
                    Err(_) if !std::path::Path::new(SYSTEM_BUS_SOCKET).exists() => {
                        return Verdict::Unknown;
                    }
                    Err(error) => {
                        eprintln!("could not reach the system bus: {error}");
                        return Verdict::Refused;
                    }
                }
            }
            let Some(bus) = connection.as_ref() else {
                return Verdict::Unknown;
            };
            match sessions(bus).await {
                Ok(sessions) if at_the_keyboard(credentials.uid, &sessions) => Verdict::Allowed,
                Ok(_) => Verdict::Refused,
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
                        || name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
                {
                    Verdict::Unknown
                }
                Err(error) => {
                    eprintln!("could not ask logind who is at the keyboard: {error}");
                    *connection = None;
                    Verdict::Refused
                }
            }
        })
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

async fn connect() -> zbus::Result<zbus::Connection> {
    zbus::connection::Builder::address(format!("unix:path={SYSTEM_BUS_SOCKET}").as_str())?
        .build()
        .await
}

async fn sessions(bus: &zbus::Connection) -> zbus::Result<Vec<Session>> {
    let listed = ManagerProxy::new(bus).await?.list_sessions().await?;
    let mut sessions = Vec::with_capacity(listed.len());
    for (_, uid, _, seat, path) in listed {
        let active = LoginSessionProxy::builder(bus)
            .path(path)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?
            .active()
            .await?;
        sessions.push(Session { uid, seat, active });
    }
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tcompass-input-s\nUid:\t1000\t1000\t1000\t1000\n\
                          Gid:\t1000\t1000\t1000\t1000\nCapInh:\t0000000000000000\n\
                          CapPrm:\t0000000000000002\nCapEff:\t0000000000000002\n";

    #[test]
    fn the_capability_and_the_real_user_are_read_from_status() {
        assert_eq!(
            credentials(STATUS),
            Some(Credentials {
                uid: 1000,
                privileged: true
            })
        );
        let plain = STATUS.replace("CapEff:\t0000000000000002", "CapEff:\t0000000000000000");
        assert_eq!(
            credentials(&plain).map(|c| c.privileged),
            Some(false),
            "a helper without the capability is not privileged"
        );
        let root = plain.replace("Uid:\t1000\t1000", "Uid:\t1000\t0");
        assert_eq!(credentials(&root).map(|c| c.privileged), Some(true));
        assert_eq!(credentials("Name:\tx\n"), None);
    }

    fn session(uid: u32, seat: &str, active: bool) -> Session {
        Session {
            uid,
            seat: seat.to_owned(),
            active,
        }
    }

    #[test]
    fn only_the_active_session_on_a_seat_is_at_the_keyboard() {
        let sessions = [
            session(1000, "seat0", true),
            session(1001, "seat0", false),
            session(1002, "", true),
        ];
        assert!(at_the_keyboard(1000, &sessions));
        assert!(
            !at_the_keyboard(1001, &sessions),
            "a user switched away from reads the next user's keys"
        );
        assert!(
            !at_the_keyboard(1002, &sessions),
            "an SSH login is not at the keyboard"
        );
        assert!(!at_the_keyboard(1003, &sessions));
    }

    #[test]
    fn an_unprivileged_helper_is_never_refused() {
        let mut gate = Gate {
            credentials: Some(Credentials {
                uid: 1000,
                privileged: false,
            }),
            runtime: None,
            connection: None,
            last: None,
        };
        assert_eq!(gate.verdict(), Verdict::Allowed);
        assert!(
            gate.runtime.is_none(),
            "no bus is asked without a privilege"
        );
    }
}
