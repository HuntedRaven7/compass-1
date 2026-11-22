//! Chrome and Firefox native messaging framing.
//!
//! A message is a four-byte little-endian length followed by exactly that many
//! bytes of UTF-8 JSON. The browser spawns us by absolute path and speaks this
//! on our stdin/stdout; nothing else is on those streams, so logs go to stderr.
//! Chrome caps a single message at 1 MiB, and we refuse more rather than
//! buffering a hostile length.

use std::io::{Read, Write};

use thiserror::Error;

/// Chrome's documented ceiling for one native message.
pub const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;

/// A framing failure.
#[derive(Debug, Error)]
pub enum FramingError {
    /// The peer closed the stream where a frame was expected.
    #[error("the native messaging stream ended between frames")]
    UnexpectedEof,
    /// The length prefix was zero or larger than [`MAX_MESSAGE_BYTES`].
    #[error("a native message of {0} bytes is outside the allowed range")]
    InvalidLength(u32),
    /// Reading or writing the stream failed.
    #[error("native messaging I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads one frame, or returns [`FramingError::UnexpectedEof`] at a clean end
/// of stream (the browser closing the host).
///
/// # Errors
///
/// A short or oversized frame, or a failed read.
pub fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, FramingError> {
    let mut prefix = [0_u8; 4];
    match reader.read_exact(&mut prefix) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FramingError::UnexpectedEof);
        }
        Err(error) => return Err(FramingError::Io(error)),
    }
    let length = u32::from_le_bytes(prefix);
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(FramingError::InvalidLength(length));
    }
    let mut payload =
        vec![0_u8; usize::try_from(length).map_err(|_| FramingError::InvalidLength(length))?];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// Writes one frame and flushes, so the browser does not wait for a buffer.
///
/// # Errors
///
/// An oversized payload or a failed write.
pub fn write_frame(writer: &mut impl Write, payload: &[u8]) -> Result<(), FramingError> {
    let length = u32::try_from(payload.len()).map_err(|_| FramingError::InvalidLength(u32::MAX))?;
    if length > MAX_MESSAGE_BYTES {
        return Err(FramingError::InvalidLength(length));
    }
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_through_a_byte_stream() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, br#"{"type":"getTabs"}"#).unwrap();
        assert_eq!(bytes[..4], 18_u32.to_le_bytes());
        let mut cursor = std::io::Cursor::new(bytes);
        assert_eq!(read_frame(&mut cursor).unwrap(), br#"{"type":"getTabs"}"#);
        assert!(matches!(
            read_frame(&mut cursor),
            Err(FramingError::UnexpectedEof)
        ));
    }

    #[test]
    fn a_hostile_length_is_refused_without_allocating_it() {
        let bytes = (MAX_MESSAGE_BYTES + 1).to_le_bytes();
        let mut cursor = std::io::Cursor::new(bytes);
        assert!(matches!(
            read_frame(&mut cursor),
            Err(FramingError::InvalidLength(_))
        ));
        let mut sink = Vec::new();
        assert!(matches!(
            write_frame(&mut sink, &vec![0_u8; MAX_MESSAGE_BYTES as usize + 1]),
            Err(FramingError::InvalidLength(_))
        ));
    }

    #[test]
    fn a_zero_length_frame_is_refused() {
        let mut cursor = std::io::Cursor::new([0_u8; 4]);
        assert!(matches!(
            read_frame(&mut cursor),
            Err(FramingError::InvalidLength(0))
        ));
    }
}
