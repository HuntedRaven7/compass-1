//! Native messaging host prototype for the browser tab extension (#315).
//!
//! ADR-0008 took browser control out of launcher core; ADR-0022 chose option 1
//! — a small host shipped with the launcher, spawned by the browser, relaying
//! to the launcher. This crate is the narrow proof of that transport: the
//! framing, the request/reply set and a binary the browser can spawn by
//! absolute path. It does not yet carry tabs; [`EmptyTabs`] is the stand-in
//! until #316 connects the host to the engine's IPC socket.
//!
//! The proof that matters is a process boundary: `tests/round_trip.rs` spawns
//! the real binary and drives connect/list/focus over its stdin and stdout.
//! That is the same boundary the browser crosses, minus the Flatpak sandbox.

#![deny(missing_docs)]

pub mod framing;
pub mod manifest;
pub mod protocol;

use std::io::{Read, Write};

use thiserror::Error;

pub use framing::{FramingError, MAX_MESSAGE_BYTES, read_frame, write_frame};
pub use protocol::{HostRequest, HostResponse, PROTOCOL_VERSION, Tab, TabSource};

/// Anything that can go wrong while serving one connection.
#[derive(Debug, Error)]
pub enum HostError {
    /// A frame was malformed or the stream failed.
    #[error(transparent)]
    Framing(#[from] FramingError),
    /// A frame was not valid JSON for this protocol.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// A tab source with nothing connected, matching `NoBrowsers` in the engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyTabs;

impl TabSource for EmptyTabs {
    fn tabs(&self) -> Vec<Tab> {
        Vec::new()
    }

    fn focus_tab(&self, _browser_id: &str, _tab_id: i64) -> bool {
        false
    }
}

/// Serves native messages until the peer closes the stream.
///
/// Each request gets exactly one reply. An unknown version or a malformed
/// request is answered with [`HostResponse::Error`] and the loop continues:
/// one bad message from an extension must not tear down the host for the
/// browser's other extensions.
///
/// # Errors
///
/// A framing or stream failure that makes further replies impossible.
pub fn serve(
    reader: &mut impl Read,
    writer: &mut impl Write,
    source: &impl TabSource,
) -> Result<(), HostError> {
    loop {
        let bytes = match read_frame(reader) {
            Ok(bytes) => bytes,
            Err(FramingError::UnexpectedEof) => return Ok(()),
            Err(error) => return Err(HostError::Framing(error)),
        };
        let request = match serde_json::from_slice::<HostRequest>(&bytes) {
            Ok(request) => request,
            Err(error) => {
                let reply = HostResponse::Error {
                    message: format!("not a valid request: {error}"),
                };
                write_frame(writer, &serde_json::to_vec(&reply)?)?;
                continue;
            }
        };
        let reply = match request {
            HostRequest::Connect { version } if version == PROTOCOL_VERSION => {
                HostResponse::Connected {
                    version: PROTOCOL_VERSION,
                    browser_id: "org.tunaos.compass".to_owned(),
                }
            }
            HostRequest::Connect { version } => HostResponse::Error {
                message: format!(
                    "protocol version {version} is not supported; this host speaks {PROTOCOL_VERSION}"
                ),
            },
            HostRequest::GetTabs => HostResponse::Tabs {
                tabs: source.tabs(),
            },
            HostRequest::FocusTab { browser_id, tab_id }
                if source.focus_tab(&browser_id, tab_id) =>
            {
                HostResponse::Ok
            }
            HostRequest::FocusTab { .. } => HostResponse::Error {
                message: "no connected browser owns that tab".to_owned(),
            },
        };
        write_frame(writer, &serde_json::to_vec(&reply)?)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_can_connect_list_and_focus_without_a_tab_source() {
        let mut client = Vec::new();
        for request in [
            HostRequest::Connect {
                version: PROTOCOL_VERSION,
            },
            HostRequest::GetTabs,
            HostRequest::FocusTab {
                browser_id: "firefox".into(),
                tab_id: 7,
            },
        ] {
            write_frame(&mut client, &serde_json::to_vec(&request).unwrap()).unwrap();
        }
        let mut cursor = std::io::Cursor::new(client);
        let mut replies = Vec::new();
        serve(&mut cursor, &mut replies, &EmptyTabs).unwrap();

        let mut out = std::io::Cursor::new(replies);
        let connected: HostResponse =
            serde_json::from_slice(&read_frame(&mut out).unwrap()).unwrap();
        assert_eq!(
            connected,
            HostResponse::Connected {
                version: PROTOCOL_VERSION,
                browser_id: "org.tunaos.compass".to_owned()
            }
        );
        let tabs: HostResponse = serde_json::from_slice(&read_frame(&mut out).unwrap()).unwrap();
        assert_eq!(tabs, HostResponse::Tabs { tabs: Vec::new() });
        let denied: HostResponse = serde_json::from_slice(&read_frame(&mut out).unwrap()).unwrap();
        assert!(matches!(denied, HostResponse::Error { .. }));
    }

    #[test]
    fn a_wrong_version_is_answered_not_fatal() {
        let mut client = Vec::new();
        write_frame(&mut client, br#"{"type":"connect","version":99}"#).unwrap();
        write_frame(&mut client, br#"{"type":"connect","version":1}"#).unwrap();
        let mut cursor = std::io::Cursor::new(client);
        let mut replies = Vec::new();
        serve(&mut cursor, &mut replies, &EmptyTabs).unwrap();
        let mut out = std::io::Cursor::new(replies);
        let first: HostResponse = serde_json::from_slice(&read_frame(&mut out).unwrap()).unwrap();
        assert!(matches!(first, HostResponse::Error { .. }));
        let second: HostResponse = serde_json::from_slice(&read_frame(&mut out).unwrap()).unwrap();
        assert!(matches!(second, HostResponse::Connected { .. }));
    }
}
