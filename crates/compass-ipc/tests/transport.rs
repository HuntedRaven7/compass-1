//! End-to-end tests over real Unix domain sockets.
//!
//! Every test binds inside its own temporary directory, so the developer's
//! live `$XDG_RUNTIME_DIR/vicinae/ipc.sock` is never touched.
//!
//! Synchronisation is done with `tokio::sync` primitives, never sleeps. The
//! only timeouts present are failure guards: if the code under test deadlocks,
//! the test should fail loudly instead of hanging CI forever.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use compass_ipc::codec::FrameCodec;
use compass_ipc::{
    Client, DoctorCheck, DoctorStatus, Error, ErrorKind, Listener, PROTOCOL_VERSION, ProtocolError,
    QueryHit, Request, RequestEnvelope, Response, ResponseEnvelope, SocketPath, is_listening,
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Barrier, oneshot};
use tokio_util::bytes::BufMut;
use tokio_util::codec::Framed;

/// Failure guard for anything that could deadlock. Not a synchronisation
/// device: the tests never rely on it elapsing.
const GUARD: Duration = Duration::from_secs(10);

// --- test scaffolding -------------------------------------------------------

/// A self-cleaning temporary directory.
///
/// Hand-rolled rather than pulled from `tempfile` so this crate's dependency
/// set stays exactly what the workspace declares.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        // Unix socket paths are capped near 108 bytes, so keep this short.
        let path = std::env::temp_dir().join(format!("cipc-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// The canonical `<dir>/vicinae/ipc.sock` location inside this tempdir.
    fn socket(&self) -> SocketPath {
        SocketPath::in_dir(&self.path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The handler used by most tests: a stand-in engine.
async fn echo_handler(request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: 4242,
        },
        Request::Toggle | Request::Show | Request::Hide => Response::Ack,
        Request::Query { text } => {
            if text.is_empty() {
                Response::Error(ProtocolError::new(ErrorKind::BadRequest, "empty query"))
            } else {
                Response::QueryResults {
                    hits: vec![QueryHit {
                        id: format!("hit:{text}"),
                        title: text,
                        subtitle: None,
                        score: 100,
                    }],
                }
            }
        }
        Request::Doctor => Response::DoctorReport {
            checks: vec![DoctorCheck {
                name: "ipc.socket".into(),
                status: DoctorStatus::Ok,
                detail: None,
            }],
        },
        Request::Shutdown => Response::ShuttingDown,
    }
}

/// Binds a listener in `dir` and serves `echo_handler` until the returned
/// sender fires. Returns once the listener is bound, so a client may connect
/// immediately with no race.
async fn spawn_echo_server(
    socket: &SocketPath,
) -> (tokio::task::JoinHandle<()>, oneshot::Sender<()>) {
    let listener = Listener::bind(socket.as_path()).await.expect("bind");
    let (stop_tx, stop_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let shutdown = async {
            let _ = stop_rx.await;
        };
        listener
            .serve_with_shutdown(echo_handler, shutdown)
            .await
            .expect("serve");
    });

    (handle, stop_tx)
}

// --- end to end -------------------------------------------------------------

#[tokio::test]
async fn client_connects_sends_and_receives() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    let response = tokio::time::timeout(GUARD, client.request(Request::Ping))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        response,
        Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: 4242
        }
    );

    drop(client);
    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn the_socket_lands_at_the_documented_path_inside_the_injected_dir() {
    let dir = TempDir::new();
    let socket = dir.socket();
    assert_eq!(socket.as_path(), dir.path().join("vicinae/ipc.sock"));

    let (server, stop) = spawn_echo_server(&socket).await;
    assert!(socket.as_path().exists());
    // The parent directory is created for us, owner-only.
    assert!(dir.path().join("vicinae").is_dir());

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn every_request_variant_survives_the_round_trip() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let mut client = Client::connect(socket.as_path()).await.unwrap();

    let expectations = vec![
        (
            Request::Ping,
            Response::Pong {
                protocol_version: PROTOCOL_VERSION,
                pid: 4242,
            },
        ),
        (Request::Toggle, Response::Ack),
        (Request::Show, Response::Ack),
        (Request::Hide, Response::Ack),
        (
            Request::Query {
                text: "term".into(),
            },
            Response::QueryResults {
                hits: vec![QueryHit {
                    id: "hit:term".into(),
                    title: "term".into(),
                    subtitle: None,
                    score: 100,
                }],
            },
        ),
        (
            Request::Doctor,
            Response::DoctorReport {
                checks: vec![DoctorCheck {
                    name: "ipc.socket".into(),
                    status: DoctorStatus::Ok,
                    detail: None,
                }],
            },
        ),
        (Request::Shutdown, Response::ShuttingDown),
    ];

    for (request, expected) in expectations {
        let got = tokio::time::timeout(GUARD, client.request(request.clone()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, expected, "wrong response for {request:?}");
    }

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn many_requests_reuse_one_connection() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    for i in 0..64u32 {
        let text = format!("q{i}");
        let response = client
            .request(Request::Query { text: text.clone() })
            .await
            .unwrap();
        match response {
            Response::QueryResults { hits } => assert_eq!(hits[0].title, text),
            other => panic!("unexpected {other:?}"),
        }
    }

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn oneshot_connects_sends_and_drops() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let response = Client::oneshot(socket.as_path(), Request::Toggle)
        .await
        .unwrap();
    assert_eq!(response, Response::Ack);

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn call_turns_a_server_error_into_an_err() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let mut client = Client::connect(socket.as_path()).await.unwrap();

    // `request` surfaces the error as a value...
    let response = client
        .request(Request::Query {
            text: String::new(),
        })
        .await
        .unwrap();
    assert!(matches!(&response, Response::Error(e) if e.kind == ErrorKind::BadRequest));

    // ...while `call` flattens it into the `Result`.
    let err = client
        .call(Request::Query {
            text: String::new(),
        })
        .await
        .unwrap_err();
    match err {
        Error::Remote(e) => {
            assert_eq!(e.kind, ErrorKind::BadRequest);
            assert_eq!(e.message, "empty query");
        }
        other => panic!("unexpected {other:?}"),
    }

    stop.send(()).unwrap();
    server.await.unwrap();
}

// --- concurrency ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_clients_each_get_their_own_response() {
    const CLIENTS: usize = 16;

    let dir = TempDir::new();
    let socket = dir.socket();
    let listener = Listener::bind(socket.as_path()).await.unwrap();

    // The barrier is the assertion: every handler blocks until all of them have
    // arrived. If the server served connections one at a time this test could
    // not finish, and the guard timeout would fail it.
    let barrier = Arc::new(Barrier::new(CLIENTS));
    let (stop_tx, stop_rx) = oneshot::channel();

    let server = {
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            let handler = move |request: Request| {
                let barrier = Arc::clone(&barrier);
                async move {
                    let Request::Query { text } = request else {
                        return Response::Error(ProtocolError::new(ErrorKind::Unsupported, "nope"));
                    };
                    barrier.wait().await;
                    Response::QueryResults {
                        hits: vec![QueryHit {
                            id: format!("hit:{text}"),
                            title: text,
                            subtitle: None,
                            score: 7,
                        }],
                    }
                }
            };
            let shutdown = async {
                let _ = stop_rx.await;
            };
            listener
                .serve_with_shutdown(handler, shutdown)
                .await
                .unwrap();
        })
    };

    let mut tasks = Vec::with_capacity(CLIENTS);
    for i in 0..CLIENTS {
        let path = socket.as_path().to_path_buf();
        tasks.push(tokio::spawn(async move {
            let mut client = Client::connect(&path).await.unwrap();
            let text = format!("client-{i}");
            let response = client
                .request(Request::Query { text: text.clone() })
                .await
                .unwrap();
            (text, response)
        }));
    }

    let results = tokio::time::timeout(GUARD, futures_util::future::join_all(tasks))
        .await
        .expect("concurrent clients deadlocked: the server is not serving them in parallel");

    for result in results {
        let (text, response) = result.unwrap();
        match response {
            Response::QueryResults { hits } => {
                assert_eq!(hits.len(), 1);
                assert_eq!(
                    hits[0].title, text,
                    "a client received another client's response"
                );
                assert_eq!(hits[0].id, format!("hit:{text}"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    stop_tx.send(()).unwrap();
    server.await.unwrap();
}

// --- single instance / stale sockets ---------------------------------------

#[tokio::test]
async fn a_live_socket_reports_already_running() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    let err = Listener::bind(socket.as_path()).await.unwrap_err();
    match err {
        Error::AlreadyRunning { ref path } => {
            assert_eq!(path, socket.as_path());
            assert!(err.to_string().contains("already running"), "{err}");
        }
        other => panic!("expected AlreadyRunning, got {other:?}"),
    }

    // The live server is untouched by the failed bind.
    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert!(matches!(
        client.request(Request::Ping).await.unwrap(),
        Response::Pong { .. }
    ));

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn a_stale_socket_is_reclaimed() {
    let dir = TempDir::new();
    let socket = dir.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();

    // A socket left behind by a crashed engine: the inode exists, nobody is
    // listening. `tokio::net::UnixListener` does not unlink on drop, which is
    // exactly the situation we need to reproduce.
    let stale = UnixListener::bind(socket.as_path()).unwrap();
    drop(stale);
    assert!(socket.as_path().exists());
    assert!(!is_listening(socket.as_path()).await);

    let (server, stop) = spawn_echo_server(&socket).await;
    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert!(matches!(
        client.request(Request::Ping).await.unwrap(),
        Response::Pong { .. }
    ));

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn a_stray_regular_file_at_the_socket_path_is_reclaimed() {
    let dir = TempDir::new();
    let socket = dir.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::write(socket.as_path(), b"not a socket").unwrap();

    let (server, stop) = spawn_echo_server(&socket).await;
    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert_eq!(
        client.request(Request::Toggle).await.unwrap(),
        Response::Ack
    );

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn dropping_the_listener_unlinks_the_socket() {
    let dir = TempDir::new();
    let socket = dir.socket();

    let listener = Listener::bind(socket.as_path()).await.unwrap();
    assert_eq!(listener.path(), socket.as_path());
    assert!(socket.as_path().exists());

    drop(listener);
    assert!(!socket.as_path().exists());
}

#[tokio::test]
async fn is_listening_distinguishes_live_stale_and_missing() {
    let dir = TempDir::new();
    let socket = dir.socket();

    assert!(!is_listening(socket.as_path()).await, "missing path");

    let (server, stop) = spawn_echo_server(&socket).await;
    assert!(is_listening(socket.as_path()).await, "live socket");

    stop.send(()).unwrap();
    server.await.unwrap();
    assert!(!is_listening(socket.as_path()).await, "after shutdown");
}

#[tokio::test]
async fn rebinding_after_a_clean_shutdown_works() {
    let dir = TempDir::new();
    let socket = dir.socket();

    let (server, stop) = spawn_echo_server(&socket).await;
    stop.send(()).unwrap();
    server.await.unwrap();

    let (server, stop) = spawn_echo_server(&socket).await;
    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert!(matches!(
        client.request(Request::Ping).await.unwrap(),
        Response::Pong { .. }
    ));

    stop.send(()).unwrap();
    server.await.unwrap();
}

// --- protocol version -------------------------------------------------------

#[tokio::test]
async fn the_server_rejects_a_peer_on_a_different_protocol_version() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    // Hand-built client speaking a future protocol version.
    let stream = UnixStream::connect(socket.as_path()).await.unwrap();
    let mut framed = Framed::new(stream, FrameCodec::<ResponseEnvelope>::new());
    let bogus = RequestEnvelope {
        version: PROTOCOL_VERSION + 7,
        id: 99,
        request: Request::Ping,
    };
    framed.send(&bogus).await.unwrap();

    let envelope = tokio::time::timeout(GUARD, framed.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(envelope.version, PROTOCOL_VERSION);
    assert_eq!(
        envelope.id, 99,
        "the error must be correlated with the request"
    );

    match envelope.response {
        Response::Error(err) => {
            assert_eq!(err.kind, ErrorKind::VersionMismatch);
            assert!(
                err.message.contains(&format!("v{}", PROTOCOL_VERSION + 7)),
                "message should name the peer's version: {}",
                err.message
            );
            assert!(
                err.message.contains(&format!("v{PROTOCOL_VERSION}")),
                "message should name our version: {}",
                err.message
            );
        }
        other => panic!("expected an error response, got {other:?}"),
    }

    // The server hangs up on a peer it cannot understand.
    assert!(
        framed.next().await.is_none(),
        "server should close after a version mismatch"
    );

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn the_client_rejects_a_server_on_a_different_protocol_version() {
    let dir = TempDir::new();
    let socket = dir.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(socket.as_path()).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut framed = Framed::new(stream, FrameCodec::<RequestEnvelope>::new());
        let request = framed.next().await.unwrap().unwrap();
        let reply = ResponseEnvelope {
            version: PROTOCOL_VERSION + 1,
            id: request.id,
            response: Response::Ack,
        };
        framed.send(&reply).await.unwrap();
    });

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    let err = client.request(Request::Toggle).await.unwrap_err();

    match err {
        Error::VersionMismatch { expected, actual } => {
            assert_eq!(expected, PROTOCOL_VERSION);
            assert_eq!(actual, PROTOCOL_VERSION + 1);
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }

    server.await.unwrap();
}

// --- hostile and broken peers ----------------------------------------------

#[tokio::test]
async fn the_client_errors_when_the_server_drops_mid_request() {
    let dir = TempDir::new();
    let socket = dir.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(socket.as_path()).unwrap();

    let (accepted_tx, accepted_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        // Read nothing, answer nothing, hang up.
        drop(stream);
        let _ = accepted_tx.send(());
    });

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    accepted_rx.await.unwrap();

    let err = tokio::time::timeout(GUARD, client.request(Request::Ping))
        .await
        .unwrap()
        .unwrap_err();
    match err {
        // Either the write lands before the peer's close is observed (then the
        // read hits EOF), or it does not (then the write fails with EPIPE).
        // Both are "the server went away"; neither may hang or panic.
        Error::ConnectionClosed => {}
        Error::Io(io) => assert!(
            matches!(
                io.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ),
            "unexpected io error: {io:?}"
        ),
        other => panic!("unexpected {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn the_server_survives_a_client_that_hangs_up_mid_frame() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    {
        // Announce a frame, send half of it, disappear.
        let mut stream = UnixStream::connect(socket.as_path()).await.unwrap();
        let mut buf = tokio_util::bytes::BytesMut::new();
        buf.put_u32_le(64);
        buf.put_slice(&[0u8; 8]);
        tokio::io::AsyncWriteExt::write_all(&mut stream, &buf)
            .await
            .unwrap();
    }

    // The server is still healthy for everybody else.
    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(GUARD, client.request(Request::Ping))
            .await
            .unwrap()
            .unwrap(),
        Response::Pong { .. }
    ));

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn the_server_survives_a_client_announcing_an_oversized_frame() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    {
        let mut stream = UnixStream::connect(socket.as_path()).await.unwrap();
        let mut buf = tokio_util::bytes::BytesMut::new();
        buf.put_u32_le(u32::MAX);
        tokio::io::AsyncWriteExt::write_all(&mut stream, &buf)
            .await
            .unwrap();
        // The connection task must error out rather than reserve 4 GiB.
        let mut sink = Vec::new();
        let _ = tokio::time::timeout(
            GUARD,
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut sink),
        )
        .await
        .expect("the server hung on an oversized frame");
    }

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(GUARD, client.request(Request::Ping))
            .await
            .unwrap()
            .unwrap(),
        Response::Pong { .. }
    ));

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn the_server_survives_a_client_sending_garbage() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    {
        let mut stream = UnixStream::connect(socket.as_path()).await.unwrap();
        let mut buf = tokio_util::bytes::BytesMut::new();
        buf.put_u32_le(6);
        buf.put_slice(&[0xff; 6]);
        tokio::io::AsyncWriteExt::write_all(&mut stream, &buf)
            .await
            .unwrap();
        let mut sink = Vec::new();
        let _ = tokio::time::timeout(
            GUARD,
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut sink),
        )
        .await
        .expect("the server hung on a garbage frame");
    }

    let mut client = Client::connect(socket.as_path()).await.unwrap();
    assert_eq!(client.request(Request::Show).await.unwrap(), Response::Ack);

    stop.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn connecting_to_a_missing_socket_fails_cleanly() {
    let dir = TempDir::new();
    let socket = dir.socket();

    let err = Client::connect(socket.as_path()).await.unwrap_err();
    assert!(matches!(err, Error::Io(_)), "unexpected {err:?}");
}

#[tokio::test]
async fn serve_with_shutdown_returns_when_asked() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let (server, stop) = spawn_echo_server(&socket).await;

    stop.send(()).unwrap();
    tokio::time::timeout(GUARD, server)
        .await
        .expect("serve loop did not stop")
        .unwrap();

    assert!(
        !socket.as_path().exists(),
        "the socket should be unlinked when the loop ends"
    );
}
