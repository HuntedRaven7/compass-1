//! The crate's error type.

use std::path::PathBuf;

use crate::protocol::ProtocolError;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Anything that can go wrong framing, transporting or dispatching a message.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Underlying socket or filesystem failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A value could not be postcard-encoded.
    #[error("failed to encode frame: {0}")]
    Encode(#[source] postcard::Error),

    /// A frame body was not valid postcard for the expected type.
    #[error("failed to decode frame: {0}")]
    Decode(#[source] postcard::Error),

    /// A peer announced a frame larger than the configured limit.
    ///
    /// The frame is rejected on the strength of its 4-byte length prefix
    /// alone, so no buffer is grown to `len`.
    #[error("frame of {len} bytes exceeds the {max} byte limit")]
    FrameTooLarge {
        /// Length the peer announced.
        len: usize,
        /// Configured maximum.
        max: usize,
    },

    /// The fallback socket directory is not one we exclusively own.
    ///
    /// Only ever raised for the `/tmp` fallback: `$XDG_RUNTIME_DIR` is the
    /// session manager's to get right, and an explicit
    /// [`crate::SocketPath::in_dir`] is the caller's.
    #[error(
        "refusing to use {} for the ipc socket: {reason}. \
         This is the fallback used when $XDG_RUNTIME_DIR is unset; a directory \
         there that is not exclusively yours may belong to another user",
        .path.display()
    )]
    UnsafeSocketDir {
        /// The directory that was refused.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },

    /// A connection came from a process running as another user.
    ///
    /// The socket's directory is the owner's alone, so this means its mode
    /// was loosened by someone else; the connection is closed unanswered.
    #[error("refusing an ipc connection from uid {peer}: the engine runs as uid {owner}")]
    ForeignPeer {
        /// The connecting process's user.
        peer: u32,
        /// The user the socket belongs to.
        owner: u32,
    },

    /// Something is already listening on the socket path.
    #[error("Compass is already running (socket {} is live)", .path.display())]
    AlreadyRunning {
        /// The socket path that answered a connection attempt.
        path: PathBuf,
    },

    /// A peer's envelope carried an unsupported protocol version.
    #[error("protocol version mismatch: peer speaks v{actual}, this build speaks v{expected}")]
    VersionMismatch {
        /// Version this build speaks.
        expected: u16,
        /// Version the peer announced.
        actual: u16,
    },

    /// The peer closed the connection before sending a complete response.
    #[error("connection closed before a response arrived")]
    ConnectionClosed,

    /// The server answered with [`crate::protocol::Response::Error`].
    #[error("server error: {0}")]
    Remote(ProtocolError),

    /// An attached window sent something other than a
    /// [`crate::protocol::Request::WindowOutcome`] in reply to a pushed
    /// command.
    ///
    /// The connection is reversed after
    /// [`crate::protocol::Request::AttachWindow`], so an ordinary request
    /// arriving on it is a peer that does not understand the handover rather
    /// than a request to serve.
    #[error("attached window replied with {got} instead of a window outcome")]
    NotAWindowOutcome {
        /// Debug rendering of what arrived instead.
        got: String,
    },

    /// The server answered a request we did not send.
    #[error("response id {got} does not match request id {expected}")]
    MismatchedResponse {
        /// Id we sent.
        expected: u64,
        /// Id we got back.
        got: u64,
    },
}
