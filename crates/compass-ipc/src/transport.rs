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
    PROTOCOL_VERSION, Request, RequestEnvelope, Response, ResponseEnvelope, version_mismatch,
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
    ///   is the "vicinae is already running" case and the caller should say so
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
        let handler = Arc::new(handler);
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
                    tokio::spawn(async move {
                        if let Err(err) = serve_connection(stream, |req| handler(req)).await {
                            tracing::debug!(error = %err, "ipc connection ended with an error");
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
        framed
            .send(&ResponseEnvelope::new(envelope.id, response))
            .await?;
    }

    Ok(())
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
