//! Bridging the resident launcher window to the engine.
//!
//! `compass-ui` deliberately knows nothing about sockets: it takes commands on
//! a channel and reports outcomes on another. `compass-ipc` deliberately knows
//! nothing about windows: it carries frames. This module is the one place that
//! knows both, which is why the two enum conversions live here and are pinned
//! by a test rather than trusted to stay in step.
//!
//! See [ADR-0015](../../../docs/rust-engine/adr/0015-the-launcher-window-is-resident.md).

use std::path::Path;

use compass_ipc::{WindowClient, WindowCommand, WindowOutcome};
use compass_ui::{EngineLink, UiCommand, UiOutcome};
use tokio::sync::mpsc;

/// Translates a pushed command into the UI's vocabulary.
///
/// Exhaustive on purpose: adding a `WindowCommand` variant must stop this
/// compiling rather than silently fall into a default.
fn to_ui(command: WindowCommand) -> UiCommand {
    match command {
        WindowCommand::Show => UiCommand::Show,
        WindowCommand::Hide => UiCommand::Hide,
        WindowCommand::Toggle => UiCommand::Toggle,
        WindowCommand::Dmenu(token) => UiCommand::Dmenu(token),
        WindowCommand::Launch(token) => UiCommand::Launch(token),
        WindowCommand::Deeplink(url) => UiCommand::Deeplink(url),
        WindowCommand::Describe => UiCommand::Describe,
        WindowCommand::Hud { text, icon } => UiCommand::Hud { text, icon },
        WindowCommand::Failure { title, message } => UiCommand::Failure { title, message },
    }
}

/// Translates the UI's answer back onto the wire.
fn from_ui(outcome: UiOutcome) -> WindowOutcome {
    match outcome {
        UiOutcome::Shown => WindowOutcome::Shown,
        UiOutcome::Hidden => WindowOutcome::Hidden,
        UiOutcome::Failed(reason) => WindowOutcome::Failed(reason),
    }
}

/// Attaches to the engine at `socket` and returns the link to hand the UI.
///
/// Returns `Ok(None)` when no engine is listening. That is not a failure: a
/// `compass ui` started by hand on a machine with no daemon should still open
/// a launcher, just an undriven one.
///
/// The bridge runs on its own thread with its own current-thread runtime,
/// because the caller's thread is about to be taken over by Iced's event loop
/// for the life of the process.
///
/// # Errors
///
/// Only for failures that are not "no engine there": a socket that answers but
/// refuses the attach, or a protocol mismatch.
pub fn attach(socket: &Path) -> anyhow::Result<Option<EngineLink>> {
    let socket = socket.to_path_buf();

    // Attached on this thread, before the bridge thread starts, so a refusal is
    // reported to the caller as an error instead of disappearing into a thread
    // nobody joins.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let client = match runtime.block_on(WindowClient::attach(&socket)) {
        Ok(client) => client,
        Err(compass_ipc::Error::Io(err)) => {
            tracing::info!(
                socket = %socket.display(), error = %err,
                "no engine to attach to; running the launcher undriven"
            );
            return Ok(None);
        }
        Err(err) => return Err(err.into()),
    };

    let (commands_tx, commands_rx) = mpsc::unbounded_channel::<UiCommand>();
    let (outcomes_tx, outcomes_rx) = mpsc::unbounded_channel::<UiOutcome>();

    std::thread::Builder::new()
        .name("compass-window-link".to_owned())
        .spawn(move || {
            runtime.block_on(bridge(client, commands_tx, outcomes_rx));
        })?;

    Ok(Some(EngineLink::new(commands_rx, outcomes_tx)))
}

/// Pumps commands into the UI and answers each with what the UI did.
///
/// Strictly one at a time, which mirrors the wire: the engine does not push
/// again until it has its answer, so a second command cannot arrive while one
/// is outstanding.
pub(crate) async fn bridge(
    mut client: WindowClient,
    commands: mpsc::UnboundedSender<UiCommand>,
    mut outcomes: mpsc::UnboundedReceiver<UiOutcome>,
) {
    loop {
        let command = match client.next_command().await {
            Ok(Some(command)) => command,
            Ok(None) => {
                tracing::info!("the engine closed the window link");
                return;
            }
            Err(err) => {
                tracing::warn!(error = %err, "the window link failed");
                return;
            }
        };

        if commands.send(to_ui(command)).is_err() {
            tracing::info!("the launcher stopped listening for commands");
            return;
        }

        let Some(outcome) = outcomes.recv().await else {
            tracing::info!("the launcher stopped reporting outcomes");
            return;
        };

        if let Err(err) = client.reply(from_ui(outcome)).await {
            tracing::warn!(error = %err, "could not report back to the engine");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use compass_ipc::{Listener, ProtocolError, Request, Response, SocketPath, WindowLink};

    /// Failure guard. Not a synchronisation device: nothing waits on it.
    const GUARD: Duration = Duration::from_secs(10);

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            // Unix socket paths are capped near 108 bytes, so keep this short.
            let path = std::env::temp_dir().join(format!("vwin-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn socket(&self) -> SocketPath {
            SocketPath::in_dir(&self.0)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A stand-in engine that accepts one window and hands its link back.
    ///
    /// Binds before returning: binding inside the spawned task would leave the
    /// caller racing the scheduler to dial a socket that may not exist yet.
    async fn engine(socket: &SocketPath) -> mpsc::Receiver<WindowLink> {
        let (links_tx, links_rx) = mpsc::channel::<WindowLink>(4);
        let listener = Listener::bind(socket.as_path()).await.expect("bind");

        tokio::spawn(async move {
            let _ = listener
                .serve_with_shutdown_attach(
                    |request| async move {
                        match request {
                            Request::AttachWindow => Response::WindowAttached,
                            _ => Response::Ack,
                        }
                    },
                    move |mut link| {
                        let links_tx = links_tx.clone();
                        async move {
                            if link.confirm().await.is_err() {
                                return;
                            }
                            let _ = links_tx.send(link).await;
                            // Held so the link outlives this task and the
                            // socket stays open for the test.
                            std::future::pending::<()>().await;
                        }
                    },
                    std::future::pending::<()>(),
                )
                .await;
        });

        links_rx
    }

    /// Drives `bridge` against a real socket, with the UI side as channels.
    ///
    /// This is the whole pump: a command pushed by the engine has to cross the
    /// socket, become a `UiCommand`, and the `UiOutcome` sent back has to reach
    /// the engine as the answer to that push.
    #[tokio::test]
    async fn a_pushed_command_crosses_to_the_ui_and_its_answer_comes_back() {
        let dir = TempDir::new();
        let socket = dir.socket();
        let mut links = engine(&socket).await;

        let client = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
            .await
            .expect("attached in time")
            .expect("attach");

        let (commands_tx, mut commands_rx) = mpsc::unbounded_channel::<UiCommand>();
        let (outcomes_tx, outcomes_rx) = mpsc::unbounded_channel::<UiOutcome>();
        tokio::spawn(bridge(client, commands_tx, outcomes_rx));

        let mut link = tokio::time::timeout(GUARD, links.recv())
            .await
            .expect("link in time")
            .expect("a link");

        // A fake UI: takes each command and answers with a distinguishable
        // outcome, so a bridge that invented an answer would be caught.
        tokio::spawn(async move {
            while let Some(command) = commands_rx.recv().await {
                let outcome = match command {
                    UiCommand::Show => UiOutcome::Shown,
                    UiCommand::Hide => UiOutcome::Hidden,
                    UiCommand::Toggle => UiOutcome::Failed("toggled".to_owned()),
                    UiCommand::Dmenu(_) | UiCommand::Launch(_) | UiCommand::Deeplink(_) => {
                        UiOutcome::Shown
                    }
                    UiCommand::Describe => UiOutcome::Hidden,
                    UiCommand::Hud { .. } => UiOutcome::Failed("no HUD".to_owned()),
                    UiCommand::Failure { .. } => UiOutcome::Hidden,
                };
                if outcomes_tx.send(outcome).is_err() {
                    return;
                }
            }
        });

        for (command, expected) in [
            (WindowCommand::Show, WindowOutcome::Shown),
            (WindowCommand::Hide, WindowOutcome::Hidden),
            (
                WindowCommand::Toggle,
                WindowOutcome::Failed("toggled".to_owned()),
            ),
        ] {
            let got = tokio::time::timeout(GUARD, link.push(command.clone()))
                .await
                .expect("push answered in time")
                .expect("push");
            assert_eq!(got, expected, "{command:?} came back wrong");
        }
    }

    #[tokio::test]
    async fn a_ui_that_stops_answering_ends_the_bridge_rather_than_wedging_the_engine() {
        let dir = TempDir::new();
        let socket = dir.socket();
        let mut links = engine(&socket).await;

        let client = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
            .await
            .expect("attached in time")
            .expect("attach");

        let (commands_tx, mut commands_rx) = mpsc::unbounded_channel::<UiCommand>();
        let (outcomes_tx, outcomes_rx) = mpsc::unbounded_channel::<UiOutcome>();

        // A UI that *takes* commands and never answers. Keeping the command
        // receiver alive matters: dropping it too would end the bridge one step
        // earlier, on the send, and a bridge that invented an answer when the
        // outcome channel closed would pass anyway. A control confirmed that --
        // this test did exactly that before the receiver was kept.
        drop(outcomes_tx);
        tokio::spawn(async move { while commands_rx.recv().await.is_some() {} });
        tokio::spawn(bridge(client, commands_tx, outcomes_rx));

        let mut link = tokio::time::timeout(GUARD, links.recv())
            .await
            .expect("link in time")
            .expect("a link");

        let outcome = tokio::time::timeout(GUARD, link.push(WindowCommand::Show))
            .await
            .expect("the push resolved rather than hanging forever");

        // The engine must learn the window is gone. Blocking here instead would
        // hang `compass toggle` until someone killed the daemon.
        assert!(
            outcome.is_err(),
            "a bridge with no UI behind it must not answer, got {outcome:?}"
        );
    }

    /// A window that is slow to answer must not hold the engine's slot, and
    /// every request queued behind it, for as long as it takes.
    #[tokio::test]
    async fn the_engine_stops_waiting_on_a_slow_window_and_keeps_it() {
        let dir = TempDir::new();
        let socket = dir.socket();
        let mut links = engine(&socket).await;

        let client = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
            .await
            .expect("attached in time")
            .expect("attach");
        let (commands_tx, mut commands_rx) = mpsc::unbounded_channel::<UiCommand>();
        let (outcomes_tx, outcomes_rx) = mpsc::unbounded_channel::<UiOutcome>();
        tokio::spawn(bridge(client, commands_tx, outcomes_rx));

        let link = tokio::time::timeout(GUARD, links.recv())
            .await
            .expect("link in time")
            .expect("a link");
        let slot = std::sync::Arc::new(tokio::sync::Mutex::new(Some(link)));

        let slow = crate::serve::forward_within(
            &slot,
            WindowCommand::Show,
            "show the launcher",
            Duration::from_millis(100),
        )
        .await;
        let Response::Error(err) = slow else {
            panic!("a window that has not answered must not be acknowledged, got {slow:?}");
        };
        assert!(err.message.contains("did not answer"), "{}", err.message);
        assert!(slot.lock().await.is_some(), "a slow window keeps its link");

        // The window catches up: the late answer is skipped, the next is read.
        assert_eq!(commands_rx.recv().await, Some(UiCommand::Show));
        outcomes_tx.send(UiOutcome::Shown).expect("late answer");
        tokio::spawn(async move {
            while let Some(command) = commands_rx.recv().await {
                let outcome = match command {
                    UiCommand::Hide => UiOutcome::Hidden,
                    _ => UiOutcome::Failed("unexpected".to_owned()),
                };
                if outcomes_tx.send(outcome).is_err() {
                    return;
                }
            }
        });
        let next = tokio::time::timeout(
            GUARD,
            crate::serve::forward(&slot, WindowCommand::Hide, "hide the launcher"),
        )
        .await
        .expect("answered in time");
        assert!(matches!(next, Response::Ack), "got {next:?}");
    }

    /// Only used to build a refusing engine in the test below.
    fn refusal() -> Response {
        Response::Error(ProtocolError::new(
            compass_ipc::ErrorKind::Unsupported,
            "not taking windows",
        ))
    }

    #[tokio::test]
    async fn attaching_to_nothing_is_not_an_error() {
        // `compass ui` with no daemon should open an undriven launcher, not
        // refuse to start. `attach` blocks, so it runs off the runtime thread.
        let dir = TempDir::new();
        let socket = dir.socket();
        let path = socket.as_path().to_path_buf();

        let link = tokio::task::spawn_blocking(move || attach(&path))
            .await
            .expect("join")
            .expect("attaching to nothing should not be an error");

        assert!(link.is_none(), "there was no engine to attach to");
    }

    #[tokio::test]
    async fn an_engine_that_refuses_the_attach_is_an_error() {
        let dir = TempDir::new();
        let socket = dir.socket();
        let listener = Listener::bind(socket.as_path()).await.expect("bind");
        tokio::spawn(listener.serve(|request| async move {
            match request {
                Request::AttachWindow => refusal(),
                _ => Response::Ack,
            }
        }));

        let path = socket.as_path().to_path_buf();
        let result = tokio::task::spawn_blocking(move || attach(&path))
            .await
            .expect("join");

        // Distinguished from "no engine": a daemon that is there and said no is
        // something the user needs told, not something to paper over.
        assert!(
            result.is_err(),
            "a refused attach must surface, got {result:?}"
        );
    }

    /// The two enums are separate types by design, so nothing but this keeps
    /// them in step. Written as a round trip rather than two tables because a
    /// table can be wrong in the same way twice.
    #[test]
    fn commands_and_outcomes_survive_the_translation() {
        for command in [
            WindowCommand::Show,
            WindowCommand::Hide,
            WindowCommand::Toggle,
            WindowCommand::Dmenu(42),
            WindowCommand::Launch(7),
            WindowCommand::Deeplink("vicinae://extensions/a/b".into()),
            WindowCommand::Describe,
            WindowCommand::Hud {
                text: "Quit Files".into(),
                icon: Some("copy-clipboard".into()),
            },
            WindowCommand::Failure {
                title: "Could not complete New Window".into(),
                message: "AppleScript is only supported on macOS".into(),
            },
        ] {
            let ui = to_ui(command.clone());
            let back = match ui {
                UiCommand::Show => WindowCommand::Show,
                UiCommand::Hide => WindowCommand::Hide,
                UiCommand::Toggle => WindowCommand::Toggle,
                UiCommand::Dmenu(token) => WindowCommand::Dmenu(token),
                UiCommand::Launch(token) => WindowCommand::Launch(token),
                UiCommand::Deeplink(url) => WindowCommand::Deeplink(url),
                UiCommand::Describe => WindowCommand::Describe,
                UiCommand::Hud { text, icon } => WindowCommand::Hud { text, icon },
                UiCommand::Failure { title, message } => WindowCommand::Failure { title, message },
            };
            assert_eq!(back, command, "{command:?} did not survive the round trip");
        }

        for outcome in [
            UiOutcome::Shown,
            UiOutcome::Hidden,
            UiOutcome::Failed("a reason".to_owned()),
        ] {
            let wire = from_ui(outcome.clone());
            let back = match wire {
                WindowOutcome::Shown => UiOutcome::Shown,
                WindowOutcome::Hidden => UiOutcome::Hidden,
                WindowOutcome::Failed(reason) => UiOutcome::Failed(reason),
            };
            assert_eq!(back, outcome, "{outcome:?} did not survive the round trip");
        }
    }

    #[test]
    fn a_failure_reason_reaches_the_wire_intact() {
        // The one variant that carries a payload. A translation that dropped
        // it would still pass a variant-only check.
        let reason = "no compositor to present on";
        assert_eq!(
            from_ui(UiOutcome::Failed(reason.to_owned())),
            WindowOutcome::Failed(reason.to_owned())
        );
    }
}
