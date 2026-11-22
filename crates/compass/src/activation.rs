//! Cold-starting Compass from a command that was bound to a key.
//!
//! [ADR-0015](../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md)
//! makes the window a resident client of the engine, so both have to be running
//! before `compass toggle` can do anything. On a machine with no service and no
//! login autostart, the first press of the hotkey is the first thing that asks
//! for either. The commands that summon the launcher therefore start what they
//! need and let it outlive them: a short-lived CLI process spawns a detached
//! `compass start`, which owns the engine and the window through the ordinary
//! [`crate::session`] and [`crate::supervise`] paths. No service manager, no D-Bus
//! activation and no enabled login switch are involved.
//!
//! A client that already has what it needs is left alone. The single UI lease
//! makes two simultaneous cold starts safe: one wins the lease, and the other
//! shows the window it just started instead of opening a second one.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use compass_ipc::{Request, Response, SocketPath};

use crate::ipc;
use crate::session;

/// How long a cold command waits for what it started to become usable.
const READY_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a poll of the window's own state is allowed to take.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The poll interval while waiting for the engine or the window.
const POLL: Duration = Duration::from_millis(50);

/// What a cold command spawns. Kept separate from the decision so the exact
/// command line can be tested without starting a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spawn {
    /// Engine plus a hidden resident window: enough to answer a headless
    /// command, and ready for the next summon without another cold start.
    SessionHidden,
    /// Engine plus a visible resident window: what `toggle` and `show` want.
    SessionVisible,
    /// The engine alone, for a machine with no graphical session.
    Headless,
}

impl Spawn {
    fn subcommand(self) -> &'static [&'static str] {
        match self {
            Self::SessionHidden => &["start", "--hidden"],
            Self::SessionVisible => &["start"],
            Self::Headless => &["serve", "--no-hotkey"],
        }
    }
}

/// Whether this process has a graphical session to open a window on.
///
/// The rule lives with [`Env`](crate::doctor::Env) so that the doctor and the
/// cold-start path cannot disagree about the same environment — in particular
/// about an empty value, which means "unset" everywhere in the desktop stack.
#[must_use]
pub fn has_display() -> bool {
    crate::doctor::has_display_in(&crate::doctor::Env::from_process())
}

/// Builds the detached process a cold command starts.
///
/// A new process group means a terminal hangup or Ctrl-C aimed at the invoking
/// command does not take the session with it. Null stdio means a daemon that
/// outlives its invoking terminal owns no pipe back to it; the engine's log is
/// its own file, as it is for every other start path.
fn command(exe: &Path, socket: &SocketPath, spawn: Spawn, verbose: u8) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("--engine=rust")
        .arg("--socket")
        .arg(socket.as_path())
        .args(std::iter::repeat_n("-v", usize::from(verbose)))
        .args(spawn.subcommand())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

/// Starts `spawn` and waits until its engine answers a ping.
///
/// Returning the child lets a caller keep watching the process it started, and
/// checking it in the loop reports an engine that exits during startup instead
/// of waiting the full timeout for one that is never coming.
async fn start(socket: &SocketPath, spawn: Spawn, verbose: u8) -> Result<Child> {
    let exe = std::env::current_exe().context("finding the compass executable")?;
    let mut child = command(&exe, socket, spawn, verbose)
        .spawn()
        .context("starting Compass on demand")?;
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("checking the Compass process")? {
            if session::listening(socket).await? {
                // A second cold starter lost the race and exited cleanly; the
                // engine the other one started is the one we care about.
                return Ok(child);
            }
            if !status.success() {
                bail!("Compass exited before its engine was ready: {status}");
            }
            // A duplicate `start` exits cleanly after handing the job to the
            // process holding the UI lease. Keep waiting for that engine.
        }
        if session::listening(socket).await? {
            return Ok(child);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the Compass engine did not become ready within {} seconds",
                READY_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Whether a window is attached to the engine and currently shown.
///
/// A poll, not a query for a person: a window that is starting, hidden, gone or
/// wedged is uniformly "not usable yet", and the caller either waits or starts
/// one. `DescribeWindow` is the engine's own state, so this adds no IPC.
async fn window_open(socket: &SocketPath) -> bool {
    tokio::time::timeout(DESCRIBE_TIMEOUT, ipc::send(socket, Request::DescribeWindow))
        .await
        .ok()
        .and_then(|answer| answer.ok())
        .is_some_and(|answer| matches!(answer, Response::WindowState { open: true }))
}

/// Ensures a window is attached and shown, starting a session if needed.
///
/// Returns whether this call had to start one. When it did, the window is
/// already visible, so a `toggle` or `show` that called this has nothing left
/// to send. An engine with a hidden window gets the same treatment as a cold
/// start: `start` either claims the free lease or asks the existing resident
/// window to show itself.
pub async fn ensure_window(socket: &SocketPath, verbose: u8) -> Result<bool> {
    if session::listening(socket).await? && window_open(socket).await {
        return Ok(false);
    }
    if !has_display() {
        bail!(
            "no Compass engine is listening on {socket}, and there is no graphical session to \
             start a launcher on. Run `compass start --hidden` from your desktop session, or see \
             `compass doctor` for the full picture"
        );
    }
    let _child = start(socket, Spawn::SessionVisible, verbose).await?;
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    while !window_open(socket).await {
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "the Compass launcher window did not open within {} seconds",
                READY_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(true)
}

/// Ensures an engine answers on `socket`, starting one if needed.
///
/// With a display this starts the full hidden session, which leaves the
/// launcher ready for the next hotkey press. Headless it starts only `serve`:
/// opening a window there would fail, and asking for a hotkey would prompt for
/// a consent nobody can give.
pub async fn ensure_engine(socket: &SocketPath, verbose: u8) -> Result<bool> {
    if session::listening(socket).await? {
        return Ok(false);
    }
    let spawn = if has_display() {
        Spawn::SessionHidden
    } else {
        Spawn::Headless
    };
    let _child = start(socket, spawn, verbose).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    use crate::Cli;

    #[test]
    fn every_spawn_keeps_the_global_flags_and_socket_in_front() {
        let socket = SocketPath::exact(Path::new("/run/user/1000/compass/ipc.sock"));
        for (spawn, tail) in [
            (Spawn::SessionHidden, vec!["start", "--hidden"]),
            (Spawn::SessionVisible, vec!["start"]),
            (Spawn::Headless, vec!["serve", "--no-hotkey"]),
        ] {
            let command = command(Path::new("/usr/bin/compass"), &socket, spawn, 2);
            let args: Vec<_> = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                args,
                vec![
                    "--engine=rust",
                    "--socket",
                    "/run/user/1000/compass/ipc.sock",
                    "-v",
                    "-v"
                ]
                .into_iter()
                .chain(tail)
                .map(str::to_owned)
                .collect::<Vec<_>>()
            );
            assert!(
                Cli::try_parse_from(
                    std::iter::once("compass".to_owned()).chain(args.iter().cloned())
                )
                .is_ok(),
                "the spawned argv must parse: {args:?}"
            );
        }
    }
}
