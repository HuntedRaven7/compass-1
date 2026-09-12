//! Unix-domain-socket IPC for the Compass engine.
//!
//! This is the socket the `vicinae` CLI, the browser native host and any other
//! local client use to talk to the running engine. It replaces the C++
//! `src/lib/vicinae-ipc` (glaze JSON-RPC over a hand-rolled 4-byte prefix, with
//! the `figura` code generator on top). Per ADR-0002 in
//! `docs/rust-engine/PLAN.md` there is no code generator here: the messages are
//! ordinary `serde` types and the body encoding is [`postcard`].
//!
//! # Layers
//!
//! * [`codec`] — the wire format: a 4-byte little-endian length prefix followed
//!   by a postcard body, as a [`tokio_util::codec`] `Encoder`/`Decoder`, with a
//!   hard maximum frame size.
//! * [`protocol`] — the versioned [`Request`]/[`Response`] envelopes.
//! * [`path`] — resolving `$XDG_RUNTIME_DIR/vicinae/ipc.sock`, injectable for
//!   tests.
//! * [`transport`] — [`Listener`] (which is also the single-instance lock),
//!   [`Client`], and the serve loop.
//!
//! # Example
//!
//! ```no_run
//! use compass_ipc::{Client, Listener, Request, Response, SocketPath};
//!
//! # async fn run() -> compass_ipc::Result<()> {
//! let socket = SocketPath::from_env();
//!
//! // Server side: binding fails with `Error::AlreadyRunning` if an engine is up.
//! let listener = Listener::bind(socket.as_path()).await?;
//! tokio::spawn(listener.serve(|request| async move {
//!     match request {
//!         Request::Ping => Response::Pong { protocol_version: compass_ipc::PROTOCOL_VERSION, pid: std::process::id() },
//!         _ => Response::Ack,
//!     }
//! }));
//!
//! // Client side.
//! let mut client = Client::connect(socket.as_path()).await?;
//! let response = client.request(Request::Ping).await?;
//! # let _ = response;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![warn(missing_debug_implementations)]

pub mod codec;
pub mod error;
pub mod path;
pub mod protocol;
pub mod transport;

pub use codec::{FrameCodec, LENGTH_PREFIX_LEN, MAX_FRAME_LEN};
pub use error::{Error, Result};
pub use path::SocketPath;
pub use protocol::{
    DoctorCheck, DoctorStatus, ErrorKind, PROTOCOL_VERSION, ProtocolError, QueryHit, Request,
    RequestEnvelope, Response, ResponseEnvelope,
};
pub use transport::{Client, Listener, is_listening, serve_connection};
