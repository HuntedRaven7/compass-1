//! The reversed direction: an engine pushing commands to an attached window.
//!
//! Everything here runs over a real Unix domain socket in its own temporary
//! directory, for the same reason the rest of `transport.rs` does: the framing,
//! the handover and the correlation are exactly the parts a mock would fake.
//!
//! See `docs/rust-engine/adr/0015-the-launcher-window-is-resident.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use compass_ipc::{
    Client, Error, ErrorKind, Listener, PROTOCOL_VERSION, ProtocolError, Request, Response,
    SocketPath, WindowClient, WindowCommand, WindowLink, WindowOutcome,
};
use tokio::sync::{Mutex, mpsc, oneshot};

/// Failure guard for anything that could deadlock. Not a synchronisation
/// device: the tests never rely on it elapsing.
const GUARD: Duration = Duration::from_secs(10);

// --- scaffolding ------------------------------------------------------------

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        // Unix socket paths are capped near 108 bytes, so keep this short.
        let path = std::env::temp_dir().join(format!("cwin-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn socket(&self) -> SocketPath {
        SocketPath::in_dir(self.path())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A stand-in engine that accepts exactly one window and hands the link out.
///
/// Returns the socket to dial and a receiver that yields the [`WindowLink`]
/// once a window attaches. The listener task is detached and dies with the
/// runtime.
async fn engine_accepting_one_window(socket: &SocketPath) -> mpsc::Receiver<WindowLink> {
    engine_with_handler(socket, |request| async move {
        match request {
            Request::AttachWindow => Response::WindowAttached,
            Request::Ping => Response::Pong {
                protocol_version: PROTOCOL_VERSION,
                pid: 4242,
            },
            _ => Response::Ack,
        }
    })
    .await
}

/// Binds **before** returning, then serves on a detached task.
///
/// Binding inside the spawned task instead would leave every caller racing the
/// scheduler to dial a socket that may not exist yet — a race that passes far
/// more often than it fails, which is the worst kind.
async fn engine_with_handler<F, Fut>(socket: &SocketPath, handler: F) -> mpsc::Receiver<WindowLink>
where
    F: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    let (links_tx, links_rx) = mpsc::channel::<WindowLink>(4);
    let listener = Listener::bind(socket.as_path()).await.expect("bind");

    tokio::spawn(async move {
        let _ = listener
            .serve_with_shutdown_attach(
                handler,
                move |mut link| {
                    let links_tx = links_tx.clone();
                    async move {
                        if link.confirm().await.is_err() {
                            return;
                        }
                        // Held until the receiver drops it, which is what keeps
                        // the socket open for the duration of the test.
                        let _ = links_tx.send(link).await;
                        std::future::pending::<()>().await;
                    }
                },
                std::future::pending::<()>(),
            )
            .await;
    });

    links_rx
}

/// Runs a window that answers every command by reporting the state named, and
/// records what it was asked. Ends when the engine closes the link.
fn window_reporting(
    socket: SocketPath,
    outcome: WindowOutcome,
) -> (
    Arc<Mutex<Vec<WindowCommand>>>,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (attached_tx, attached_rx) = oneshot::channel();
    let handle = {
        let seen = Arc::clone(&seen);
        tokio::spawn(async move {
            let mut window = WindowClient::attach(socket.as_path())
                .await
                .expect("attach");
            let _ = attached_tx.send(());
            while let Some(command) = window.next_command().await.expect("next command") {
                seen.lock().await.push(command);
                window.reply(outcome.clone()).await.expect("reply");
            }
        })
    };
    (seen, attached_rx, handle)
}

// --- the happy path ---------------------------------------------------------

#[tokio::test]
async fn a_pushed_command_reaches_the_window_and_its_answer_comes_back() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let (seen, attached, _window) = window_reporting(socket.clone(), WindowOutcome::Shown);
    tokio::time::timeout(GUARD, attached)
        .await
        .expect("window attached in time")
        .expect("attach signal");

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let outcome = tokio::time::timeout(GUARD, link.push(WindowCommand::Show))
        .await
        .expect("push answered in time")
        .expect("push");

    assert_eq!(outcome, WindowOutcome::Shown);
    assert_eq!(&*seen.lock().await, &[WindowCommand::Show]);
}

#[tokio::test]
async fn commands_arrive_in_the_order_they_were_pushed() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let (seen, attached, _window) = window_reporting(socket.clone(), WindowOutcome::Hidden);
    tokio::time::timeout(GUARD, attached)
        .await
        .expect("window attached in time")
        .expect("attach signal");

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    // Three *different* commands, so a transport that dropped or reordered one
    // produces a different vector rather than an identical-looking one. A
    // sequence of three `Show`s would pass even if the link sent the first one
    // three times.
    for command in [
        WindowCommand::Show,
        WindowCommand::Hide,
        WindowCommand::Toggle,
    ] {
        tokio::time::timeout(GUARD, link.push(command))
            .await
            .expect("push answered in time")
            .expect("push");
    }

    assert_eq!(
        &*seen.lock().await,
        &[
            WindowCommand::Show,
            WindowCommand::Hide,
            WindowCommand::Toggle
        ]
    );
}

#[tokio::test]
async fn the_outcome_the_window_reports_is_the_one_the_engine_reads() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    // `Failed` carries a payload, so this also proves the reply body survives
    // the round trip rather than only its variant index.
    let reason = "no compositor";
    let (_seen, attached, _window) =
        window_reporting(socket.clone(), WindowOutcome::Failed(reason.to_owned()));
    tokio::time::timeout(GUARD, attached)
        .await
        .expect("window attached in time")
        .expect("attach signal");

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let outcome = tokio::time::timeout(GUARD, link.push(WindowCommand::Toggle))
        .await
        .expect("push answered in time")
        .expect("push");

    assert_eq!(outcome, WindowOutcome::Failed(reason.to_owned()));
}

// --- the two processes agreeing on lifetime ---------------------------------
//
// ADR-0015: "If the window dies, `serve` must notice and go back to refusing
// rather than reporting success into a closed socket. If `serve` dies, the
// window should not wedge." Both directions get a test that kills one side.

#[tokio::test]
async fn a_push_to_a_dead_window_fails_rather_than_reporting_success() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let (_seen, attached, window) = window_reporting(socket.clone(), WindowOutcome::Shown);
    tokio::time::timeout(GUARD, attached)
        .await
        .expect("window attached in time")
        .expect("attach signal");

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    // Control: the same push succeeds while the window is alive, so the failure
    // below is the death and not the fixture.
    let alive = tokio::time::timeout(GUARD, link.push(WindowCommand::Show))
        .await
        .expect("push answered in time")
        .expect("push");
    assert_eq!(alive, WindowOutcome::Shown);

    window.abort();
    let _ = window.await;

    let after_death = tokio::time::timeout(GUARD, link.push(WindowCommand::Hide))
        .await
        .expect("push resolved in time");

    // What this pins is narrow on purpose: a dead window makes `push` fail
    // rather than succeed. It does *not* establish that the read is what
    // noticed — here the write itself fails with EPIPE, so a write-only `push`
    // would pass this too. That stronger property is
    // `a_push_whose_write_succeeded_is_still_not_an_answer` below, which was
    // added after a control showed this test alone did not catch deleting the
    // read.
    assert!(
        matches!(
            after_death,
            Err(Error::ConnectionClosed) | Err(Error::Io(_))
        ),
        "pushing to a dead window must fail, got {after_death:?}"
    );
}

/// The engine gives up on a slow window by dropping the push. The window
/// still answers that command, late and before the next one, and the next
/// push must not take that late answer for its own.
#[tokio::test]
async fn a_push_given_up_on_does_not_steal_the_next_ones_answer() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let (release_tx, release_rx) = oneshot::channel::<()>();
    let window = tokio::spawn({
        let socket = socket.clone();
        async move {
            let mut window = WindowClient::attach(socket.as_path())
                .await
                .expect("attach");
            let first = window
                .next_command()
                .await
                .expect("first")
                .expect("a command");
            assert_eq!(first, WindowCommand::Show);
            let _ = release_rx.await;
            window
                .reply(WindowOutcome::Shown)
                .await
                .expect("late reply");
            let second = window
                .next_command()
                .await
                .expect("second")
                .expect("a command");
            assert_eq!(second, WindowCommand::Hide);
            window.reply(WindowOutcome::Hidden).await.expect("reply");
            let _ = window.next_command().await;
        }
    });

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let given_up =
        tokio::time::timeout(Duration::from_millis(100), link.push(WindowCommand::Show)).await;
    assert!(
        given_up.is_err(),
        "the window was holding its answer, got {given_up:?}"
    );

    let _ = release_tx.send(());
    let outcome = tokio::time::timeout(GUARD, link.push(WindowCommand::Hide))
        .await
        .expect("push answered in time")
        .expect("push");
    assert_eq!(outcome, WindowOutcome::Hidden);

    drop(link);
    tokio::time::timeout(GUARD, window)
        .await
        .expect("window ended in time")
        .expect("window task");
}

#[tokio::test]
async fn a_window_whose_engine_went_away_is_told_rather_than_left_waiting() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let mut window = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
        .await
        .expect("attached in time")
        .expect("attach");

    let link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    // Dropping the link closes the engine's end of the socket.
    drop(link);

    let next = tokio::time::timeout(GUARD, window.next_command())
        .await
        .expect("next_command resolved rather than hanging")
        .expect("no protocol error");

    assert_eq!(next, None, "a closed link must end the window's loop");
}

/// A window that *receives* a push and then closes without replying.
///
/// The discriminating case. Because the peer read the command frame, the
/// engine's write is known to have succeeded; so an implementation that
/// answered from the write alone would return `Ok` here, and only one that
/// waits for the reply can report the window is gone.
#[tokio::test]
async fn a_push_whose_write_succeeded_is_still_not_an_answer() {
    use compass_ipc::codec::FrameCodec;
    use compass_ipc::{RequestEnvelope, ResponseEnvelope};
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::UnixStream;
    use tokio_util::codec::Framed;

    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    // Hand-rolled rather than `WindowClient`, because the behaviour under test
    // is one `WindowClient` deliberately cannot produce: read a command, then
    // vanish without replying.
    let (got_command_tx, got_command_rx) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let stream = UnixStream::connect(socket.as_path())
            .await
            .expect("connect");
        let mut framed = Framed::new(stream, FrameCodec::<ResponseEnvelope>::new());
        framed
            .send(&RequestEnvelope::new(1, Request::AttachWindow))
            .await
            .expect("send attach");
        let attached = framed.next().await.expect("attach answer").expect("decode");
        assert!(matches!(attached.response, Response::WindowAttached));

        let pushed = framed.next().await.expect("a push").expect("decode");
        assert!(
            matches!(pushed.response, Response::Window(WindowCommand::Show)),
            "expected the pushed command, got {:?}",
            pushed.response
        );
        // The engine's write landed. Now leave without answering.
        let _ = got_command_tx.send(());
        drop(framed);
    });

    let mut link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let outcome = tokio::time::timeout(GUARD, link.push(WindowCommand::Show))
        .await
        .expect("push resolved rather than hanging");

    tokio::time::timeout(GUARD, got_command_rx)
        .await
        .expect("peer read the command in time")
        .expect("peer signal");
    peer.await.expect("peer task");

    assert!(
        matches!(outcome, Err(Error::ConnectionClosed)),
        "a push whose write succeeded but whose reply never came must report the \
         window gone, got {outcome:?}"
    );
}

// --- refusing a second window -----------------------------------------------

#[tokio::test]
async fn an_engine_that_already_holds_a_window_can_refuse_the_next_one() {
    let dir = TempDir::new();
    let socket = dir.socket();

    // Accepts the first `AttachWindow` and refuses every one after it, which is
    // the shape `serve` needs: the decision is the handler's, not the
    // transport's.
    let taken = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut links = {
        let taken = Arc::clone(&taken);
        engine_with_handler(&socket, move |request| {
            let taken = Arc::clone(&taken);
            async move {
                match request {
                    Request::AttachWindow => {
                        if taken.swap(true, Ordering::SeqCst) {
                            Response::Error(ProtocolError::new(
                                ErrorKind::Unsupported,
                                "a launcher window is already attached",
                            ))
                        } else {
                            Response::WindowAttached
                        }
                    }
                    _ => Response::Ack,
                }
            }
        })
        .await
    };

    let first = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
        .await
        .expect("attached in time");
    assert!(first.is_ok(), "the first window must attach: {first:?}");

    let _link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let second = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
        .await
        .expect("second attach resolved in time");

    match second {
        Err(Error::Remote(err)) => {
            assert_eq!(err.kind, ErrorKind::Unsupported);
            assert!(
                err.message.contains("already attached"),
                "the refusal should say why: {}",
                err.message
            );
        }
        other => panic!("a second window must be refused, got {other:?}"),
    }
}

// --- the handover does not disturb ordinary clients -------------------------

#[tokio::test]
async fn ordinary_requests_still_work_on_an_engine_that_holds_a_window() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut links = engine_accepting_one_window(&socket).await;

    let (_seen, attached, _window) = window_reporting(socket.clone(), WindowOutcome::Shown);
    tokio::time::timeout(GUARD, attached)
        .await
        .expect("window attached in time")
        .expect("attach signal");

    let _link = tokio::time::timeout(GUARD, links.recv())
        .await
        .expect("link handed over in time")
        .expect("a link");

    let mut client = Client::connect(socket.as_path()).await.expect("connect");
    let response = tokio::time::timeout(GUARD, client.request(Request::Ping))
        .await
        .expect("ping answered in time")
        .expect("ping");

    assert!(
        matches!(response, Response::Pong { pid: 4242, .. }),
        "an attached window must not take the serve loop with it, got {response:?}"
    );
}

#[tokio::test]
async fn a_reply_with_no_command_outstanding_is_refused() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let _links = engine_accepting_one_window(&socket).await;

    let mut window = tokio::time::timeout(GUARD, WindowClient::attach(socket.as_path()))
        .await
        .expect("attached in time")
        .expect("attach");

    // Replying before any command would put a frame on the wire correlated
    // against an id the engine is not waiting for, leaving the two sides one
    // reply out of step for the rest of the connection.
    let premature = window.reply(WindowOutcome::Shown).await;
    assert!(
        matches!(premature, Err(Error::ConnectionClosed)),
        "a reply with nothing outstanding must be refused, got {premature:?}"
    );
}
