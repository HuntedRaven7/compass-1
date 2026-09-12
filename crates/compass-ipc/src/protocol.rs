//! Versioned request/response types carried over the socket.
//!
//! Every frame on the wire is one [`RequestEnvelope`] (client to server) or one
//! [`ResponseEnvelope`] (server to client). Both carry the [`PROTOCOL_VERSION`]
//! they were produced with, so an old `vicinae` CLI meeting a new engine — or
//! the reverse — gets a clear [`ErrorKind::VersionMismatch`] instead of a
//! postcard decode failure or, worse, a silent misinterpretation.
//!
//! Postcard is **not** self-describing: field order and variant order *are* the
//! schema. Adding a variant in the middle of [`Request`] or [`Response`], or
//! reordering fields, is a breaking change. Append new variants at the end and
//! bump [`PROTOCOL_VERSION`] whenever the meaning of existing bytes changes.

use serde::{Deserialize, Serialize};

/// Version of the request/response schema understood by this build.
///
/// Bumped whenever the postcard encoding of [`Request`] or [`Response`]
/// changes in a way that an older peer would misread.
pub const PROTOCOL_VERSION: u16 = 1;

/// A client-to-server frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id, echoed in the matching [`ResponseEnvelope`].
    ///
    /// The current transport is strictly request/response per connection, but
    /// the id is on the wire from day one so pipelining does not need a
    /// protocol bump.
    pub id: u64,
    /// The request itself.
    pub request: Request,
}

impl RequestEnvelope {
    /// Wraps `request` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            request,
        }
    }
}

/// A server-to-client frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Protocol version the sender speaks. See [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Correlation id copied from the request this answers.
    ///
    /// Zero when the server could not decode a request far enough to know it.
    pub id: u64,
    /// The response itself.
    pub response: Response,
}

impl ResponseEnvelope {
    /// Wraps `response` at the current [`PROTOCOL_VERSION`].
    #[must_use]
    pub fn new(id: u64, response: Response) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id,
            response,
        }
    }
}

/// What a client can ask the engine to do.
///
/// Deliberately small: this is the Phase 2 surface (socket, CLI, doctor) and
/// nothing more. New variants go at the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Liveness probe. Answered with [`Response::Pong`].
    Ping,
    /// Toggle the launcher window between shown and hidden.
    Toggle,
    /// Show the launcher window.
    Show,
    /// Hide the launcher window.
    Hide,
    /// Run a root search and return the ranked hits.
    Query {
        /// Raw search text, exactly as typed.
        text: String,
    },
    /// Run the self-diagnostic checks behind `vicinae doctor`.
    Doctor,
    /// Ask the engine to shut down cleanly.
    Shutdown,
}

/// What the engine answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// Answer to [`Request::Ping`].
    Pong {
        /// Protocol version the server speaks.
        protocol_version: u16,
        /// Process id of the engine, for `doctor` and stale-socket reporting.
        pid: u32,
    },
    /// The requested side effect was performed; there is nothing to report.
    Ack,
    /// Ranked results for a [`Request::Query`].
    QueryResults {
        /// Hits in presentation order: best first.
        hits: Vec<QueryHit>,
    },
    /// Diagnostic report for a [`Request::Doctor`].
    DoctorReport {
        /// One entry per check, in the order they were run.
        checks: Vec<DoctorCheck>,
    },
    /// The engine accepted [`Request::Shutdown`] and is on its way down.
    ShuttingDown,
    /// The request could not be served.
    Error(ProtocolError),
}

/// One ranked search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryHit {
    /// Stable identifier of the underlying root item.
    pub id: String,
    /// Primary display text.
    pub title: String,
    /// Secondary display text, when the item has one.
    pub subtitle: Option<String>,
    /// Match score in `0..=100`, matching `compass-search`'s scale.
    pub score: u32,
}

/// One diagnostic check performed by `vicinae doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Short machine-ish name, e.g. `"portal.global-shortcuts"`.
    pub name: String,
    /// Outcome of the check.
    pub status: DoctorStatus,
    /// Human-readable detail, when there is something to say.
    pub detail: Option<String>,
}

/// Outcome of a single [`DoctorCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DoctorStatus {
    /// Working as intended.
    Ok,
    /// Degraded but usable.
    Warn,
    /// Broken.
    Fail,
}

/// A failure reported by the server in [`Response::Error`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    /// Machine-readable category.
    pub kind: ErrorKind,
    /// Human-readable explanation, safe to print to a terminal.
    pub message: String,
}

impl ProtocolError {
    /// Builds an error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

/// Category of a [`ProtocolError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Peer speaks a protocol version this build cannot serve.
    VersionMismatch,
    /// The request was understood but this build does not implement it.
    Unsupported,
    /// The request was malformed.
    BadRequest,
    /// The handler failed.
    Internal,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::VersionMismatch => "version mismatch",
            Self::Unsupported => "unsupported",
            Self::BadRequest => "bad request",
            Self::Internal => "internal error",
        };
        f.write_str(s)
    }
}

/// Builds the [`Response::Error`] the server sends when a peer's envelope
/// carries a version this build does not speak.
#[must_use]
pub fn version_mismatch(peer: u16) -> Response {
    Response::Error(ProtocolError::new(
        ErrorKind::VersionMismatch,
        format!(
            "peer speaks compass-ipc protocol v{peer}, this build speaks v{PROTOCOL_VERSION}; \
             restart the client and the engine from the same build"
        ),
    ))
}
