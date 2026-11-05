//! Unix-domain-socket transport: listener, serve loop and client.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::codec::Framed;

use crate::codec::FrameCodec;
use crate::error::{Error, Result};
use crate::protocol::{
    PROTOCOL_VERSION, Request, RequestEnvelope, Response, ResponseEnvelope, WindowCommand,
    WindowOutcome, version_mismatch,
};

/// Permissions for the directory holding the socket: owner only.
const SOCKET_DIR_MODE: u32 = 0o700;

/// Probes whether a live server is listening at `path`.
///
/// Returns `true` only if a connection was accepted. A missing file, a stale
/// socket left behind by a crashed engine, or a regular file all return
/// `false`.
pub async fn is_listening(path: impl AsRef<Path>) -> bool {
    UnixStream::connect(path.as_ref()).await.is_ok()
}

/// A bound Unix listener that owns its socket file.
///
/// Binding doubles as the single-instance lock: a second engine trying to bind
/// the same path gets [`Error::AlreadyRunning`]. Dropping the listener unlinks
/// the socket file.
#[derive(Debug)]
pub struct Listener {
    inner: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Binds a listener at `path`, reclaiming a stale socket if there is one.
    ///
    /// Behaviour when `path` already exists:
    ///
    /// * something accepts a connection there → [`Error::AlreadyRunning`]; this
    ///   is the "Compass is already running" case and the caller should say so
    ///   and exit,
    /// * nothing accepts (a socket left by a crashed engine, or a stray regular
    ///   file) → the path is unlinked and the bind retried once.
    ///
    /// The parent directory is created if missing, with mode `0700`.
    pub async fn bind(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(SOCKET_DIR_MODE);
            builder.create(parent).map_err(Error::Io)?;
        }

        match UnixListener::bind(&path) {
            Ok(inner) => Ok(Self { inner, path }),
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                if is_listening(&path).await {
                    return Err(Error::AlreadyRunning { path });
                }

                // Known narrow race: two engines starting at the same moment can both observe a
                // stale socket, both unlink it, and both bind -- leaving the loser listening on an
                // unlinked inode that no client can reach. Not fixed here because the window is a
                // few microseconds after a crash, the failure is recoverable by restarting, and the
                // real fix (an O_EXCL lock file gating the reclaim) is a design change rather than
                // a patch. Tracked rather than silently tolerated.
                tracing::info!(path = %path.display(), "reclaiming stale ipc socket");
                std::fs::remove_file(&path).map_err(Error::Io)?;
                let inner = UnixListener::bind(&path).map_err(Error::Io)?;
                Ok(Self { inner, path })
            }
            Err(err) => Err(Error::Io(err)),
        }
    }

    /// The path this listener is bound to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accepts one connection.
    pub async fn accept(&self) -> Result<UnixStream> {
        let (stream, _addr) = self.inner.accept().await.map_err(Error::Io)?;
        Ok(stream)
    }

    /// Accepts connections forever, dispatching every request to `handler`.
    ///
    /// Each connection is served on its own task, so slow handlers do not block
    /// other clients.
    pub async fn serve<F, Fut>(self, handler: F) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.serve_with_shutdown(handler, std::future::pending::<()>())
            .await
    }

    /// Like [`Listener::serve`], but stops accepting when `shutdown` resolves.
    ///
    /// Connections still in flight are dropped at that point; a handler that
    /// needs to finish its work should own that responsibility itself.
    pub async fn serve_with_shutdown<F, Fut, S>(self, handler: F, shutdown: S) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
        S: Future<Output = ()>,
    {
        // `|_| async {}` rather than a dedicated no-attach path: a handler that
        // never answers `WindowAttached` never reaches it, so there is nothing
        // for a second code path to do differently.
        self.serve_with_shutdown_attach(handler, |_link| async {}, shutdown)
            .await
    }

    /// Like [`Listener::serve_with_shutdown`], but hands over connections that
    /// become launcher windows.
    ///
    /// When `handler` answers a request with [`Response::WindowAttached`], the
    /// connection **stops being request/response**: it is wrapped in a
    /// [`WindowLink`] and passed to `on_attach`, which owns it for the rest of
    /// its life. `on_attach` runs on the connection's own task, so it may block
    /// for as long as the window lives without holding up other clients.
    ///
    /// The acceptance is **not written yet**: `on_attach` stores the link
    /// where requests will find it and then calls [`WindowLink::confirm`]. A
    /// window that is told it attached is therefore already reachable, so a
    /// request sent the moment its attach returns cannot be refused for want
    /// of a window.
    ///
    /// The handler decides *whether* to attach — a server that already holds a
    /// window can refuse a second one by answering [`Response::Error`] instead,
    /// and never sees `on_attach` called.
    pub async fn serve_with_shutdown_attach<F, Fut, A, AFut, S>(
        self,
        handler: F,
        on_attach: A,
        shutdown: S,
    ) -> Result<()>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
        A: Fn(WindowLink) -> AFut + Send + Sync + 'static,
        AFut: Future<Output = ()> + Send + 'static,
        S: Future<Output = ()>,
    {
        let handler = Arc::new(handler);
        let on_attach = Arc::new(on_attach);
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                () = &mut shutdown => {
                    tracing::debug!(path = %self.path.display(), "ipc serve loop shutting down");
                    return Ok(());
                }
                accepted = self.inner.accept() => {
                    let (stream, _addr) = accepted.map_err(Error::Io)?;
                    let handler = Arc::clone(&handler);
                    let on_attach = Arc::clone(&on_attach);
                    tokio::spawn(async move {
                        match serve_connection_until_attach(stream, |req| handler(req)).await {
                            Ok(Some(link)) => on_attach(link).await,
                            Ok(None) => {}
                            Err(err) => {
                                tracing::debug!(error = %err, "ipc connection ended with an error");
                            }
                        }
                    });
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Best effort: if this fails the next start reclaims the stale socket.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Serves a single accepted connection until the peer closes it.
///
/// Reads [`RequestEnvelope`] frames, rejects protocol-version mismatches,
/// passes the request to `handler`, and writes back a [`ResponseEnvelope`]
/// carrying the same correlation id.
pub async fn serve_connection<F, Fut>(stream: UnixStream, handler: F) -> Result<()>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    // An attached connection is dropped here rather than served: a caller that
    // wants windows uses `serve_connection_until_attach` and is handed the
    // link. Dropping closes the socket, so a window that attaches to a server
    // which cannot hold one finds out immediately instead of waiting forever
    // for a push that will never come.
    serve_connection_until_attach(stream, handler)
        .await
        .map(|_| ())
}

/// Serves a connection until the peer closes it **or** becomes a window.
///
/// Returns `Ok(Some(link))` when `handler` answered with
/// [`Response::WindowAttached`]: the connection now belongs to the returned
/// [`WindowLink`], whose acceptance is sent by [`WindowLink::confirm`] (or by
/// its first push). Returns `Ok(None)` when the peer closed an ordinary
/// request/response connection.
pub async fn serve_connection_until_attach<F, Fut>(
    stream: UnixStream,
    handler: F,
) -> Result<Option<WindowLink>>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    let mut framed = Framed::new(stream, FrameCodec::<RequestEnvelope>::new());

    while let Some(frame) = framed.next().await {
        let envelope = frame?;

        if envelope.version != PROTOCOL_VERSION {
            tracing::warn!(
                peer_version = envelope.version,
                "rejecting ipc peer on version mismatch"
            );
            let response = ResponseEnvelope::new(envelope.id, version_mismatch(envelope.version));
            framed.send(&response).await?;
            // Nothing this peer says afterwards can be trusted to mean what we
            // would read it as, so stop here rather than keep decoding.
            break;
        }

        let response = handler(envelope.request).await;
        if matches!(response, Response::WindowAttached) {
            return Ok(Some(WindowLink {
                framed,
                next_id: envelope.id.wrapping_add(1),
                unconfirmed: Some(envelope.id),
            }));
        }
        framed
            .send(&ResponseEnvelope::new(envelope.id, response))
            .await?;
    }

    Ok(None)
}

/// The engine's end of an attached launcher window.
///
/// Holds the reversed connection: this side sends [`WindowCommand`]s and reads
/// [`WindowOutcome`]s, the mirror image of every other connection the
/// [`Listener`] serves.
///
/// Dropping it closes the socket, which is how the window learns the engine is
/// no longer driving it.
#[derive(Debug)]
pub struct WindowLink {
    framed: Framed<UnixStream, FrameCodec<RequestEnvelope>>,
    next_id: u64,
    /// Id of the attach request whose acceptance has not been written yet.
    unconfirmed: Option<u64>,
}

impl WindowLink {
    /// Tells the window its attach was accepted, if it has not been told.
    ///
    /// Call it once the link is where requests can reach it; see
    /// [`Listener::serve_with_shutdown_attach`]. Every push confirms first, so
    /// the window always reads the acceptance before any command.
    ///
    /// # Errors
    ///
    /// The write failing, typically because the window already went away.
    pub async fn confirm(&mut self) -> Result<()> {
        if let Some(id) = self.unconfirmed.take() {
            self.framed
                .send(&ResponseEnvelope::new(id, Response::WindowAttached))
                .await?;
        }
        Ok(())
    }

    /// Pushes `command` to the window and waits for what it did.
    ///
    /// # Errors
    ///
    /// [`Error::ConnectionClosed`] when the window went away — which is the
    /// signal the engine needs to go back to refusing `show`/`hide`/`toggle`
    /// rather than reporting success into a dead socket. The push is *not*
    /// acknowledged by the write succeeding: a successful write only means the
    /// bytes reached a kernel buffer, and a window that died between the write
    /// and the read would look like a window that showed.
    ///
    /// # Cancellation
    ///
    /// The future may be dropped while it waits, as the engine does when a
    /// window takes too long to answer. The window still answers that command
    /// in order, before the next one, so the next push skips answers to
    /// commands older than its own instead of taking one as its own.
    pub async fn push(&mut self, command: WindowCommand) -> Result<WindowOutcome> {
        self.confirm().await?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        self.framed
            .send(&ResponseEnvelope::new(id, Response::Window(command)))
            .await?;

        let envelope = loop {
            let envelope = match self.framed.next().await {
                Some(frame) => frame?,
                None => return Err(Error::ConnectionClosed),
            };
            if envelope.id >= id {
                break envelope;
            }
            tracing::debug!(
                late = envelope.id,
                waiting_for = id,
                "skipping a late window answer"
            );
        };

        if envelope.version != PROTOCOL_VERSION {
            return Err(Error::VersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: envelope.version,
            });
        }

        if envelope.id != id {
            return Err(Error::MismatchedResponse {
                expected: id,
                got: envelope.id,
            });
        }

        match envelope.request {
            Request::WindowOutcome(outcome) => Ok(outcome),
            other => Err(Error::NotAWindowOutcome {
                got: format!("{other:?}"),
            }),
        }
    }
}

/// A launcher window's end of the link: connect, attach, then take commands.
///
/// The window drives this loop itself rather than being called into, because
/// on every platform the window's own event loop owns the thread that may
/// touch it.
#[derive(Debug)]
pub struct WindowClient {
    framed: Framed<UnixStream, FrameCodec<ResponseEnvelope>>,
    /// Id of the command handed out by the last [`WindowClient::next_command`]
    /// and not yet answered.
    pending: Option<u64>,
}

impl WindowClient {
    /// Connects to the engine at `path` and offers this process as its window.
    ///
    /// # Errors
    ///
    /// [`Error::Remote`] when the engine refuses — most usefully when another
    /// window is already attached, which a second `compass ui` should report
    /// rather than sit silently unused.
    pub async fn attach(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path.as_ref())
            .await
            .map_err(Error::Io)?;
        let mut framed = Framed::new(stream, FrameCodec::<ResponseEnvelope>::new());

        framed
            .send(&RequestEnvelope::new(1, Request::AttachWindow))
            .await?;

        let envelope = match framed.next().await {
            Some(frame) => frame?,
            None => return Err(Error::ConnectionClosed),
        };

        if envelope.version != PROTOCOL_VERSION {
            return Err(Error::VersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: envelope.version,
            });
        }

        match envelope.response {
            Response::WindowAttached => Ok(Self {
                framed,
                pending: None,
            }),
            Response::Error(err) => Err(Error::Remote(err)),
            other => Err(Error::NotAWindowOutcome {
                got: format!("{other:?}"),
            }),
        }
    }

    /// Waits for the next command from the engine.
    ///
    /// Returns `Ok(None)` when the engine closed the link, which is the
    /// window's cue to stop rather than wedge waiting for a push that cannot
    /// come.
    pub async fn next_command(&mut self) -> Result<Option<WindowCommand>> {
        let envelope = match self.framed.next().await {
            Some(frame) => frame?,
            None => return Ok(None),
        };

        if envelope.version != PROTOCOL_VERSION {
            return Err(Error::VersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: envelope.version,
            });
        }

        match envelope.response {
            Response::Window(command) => {
                self.pending = Some(envelope.id);
                Ok(Some(command))
            }
            other => Err(Error::NotAWindowOutcome {
                got: format!("{other:?}"),
            }),
        }
    }

    /// Reports what the window did with the command last handed out.
    ///
    /// # Errors
    ///
    /// [`Error::ConnectionClosed`] when called with no command outstanding.
    /// Replying twice, or before any command, would put a frame on the wire
    /// that the engine correlates against an id it is not waiting for, and the
    /// two sides would be one reply out of step from then on.
    pub async fn reply(&mut self, outcome: WindowOutcome) -> Result<()> {
        let id = self.pending.take().ok_or(Error::ConnectionClosed)?;
        self.framed
            .send(&RequestEnvelope::new(id, Request::WindowOutcome(outcome)))
            .await?;
        Ok(())
    }
}

/// A connected IPC client.
#[derive(Debug)]
pub struct Client {
    framed: Framed<UnixStream, FrameCodec<ResponseEnvelope>>,
    next_id: u64,
}

impl Client {
    /// Connects to the server listening at `path`.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path.as_ref())
            .await
            .map_err(Error::Io)?;
        Ok(Self {
            framed: Framed::new(stream, FrameCodec::new()),
            next_id: 1,
        })
    }

    /// Sends `request` and waits for its response.
    ///
    /// A [`Response::Error`] from the server is returned as `Ok`, not `Err`:
    /// transport failures and application failures are different things. Use
    /// [`Client::call`] when you want both flattened into `Err`.
    pub async fn request(&mut self, request: Request) -> Result<Response> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        self.framed.send(&RequestEnvelope::new(id, request)).await?;

        let envelope = match self.framed.next().await {
            Some(frame) => frame?,
            None => return Err(Error::ConnectionClosed),
        };

        if envelope.version != PROTOCOL_VERSION {
            return Err(Error::VersionMismatch {
                expected: PROTOCOL_VERSION,
                actual: envelope.version,
            });
        }

        if envelope.id != id {
            return Err(Error::MismatchedResponse {
                expected: id,
                got: envelope.id,
            });
        }

        Ok(envelope.response)
    }

    /// Like [`Client::request`], but turns [`Response::Error`] into
    /// [`Error::Remote`].
    pub async fn call(&mut self, request: Request) -> Result<Response> {
        match self.request(request).await? {
            Response::Error(err) => Err(Error::Remote(err)),
            other => Ok(other),
        }
    }

    /// Connects, sends one request, and drops the connection.
    pub async fn oneshot(path: impl AsRef<Path>, request: Request) -> Result<Response> {
        let mut client = Self::connect(path).await?;
        client.request(request).await
    }
}
