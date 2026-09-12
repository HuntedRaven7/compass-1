//! Length-prefixed postcard framing.
//!
//! # Wire format
//!
//! A stream is a sequence of frames laid end to end with nothing between them.
//! One frame is:
//!
//! ```text
//! ┌───────────────────────────┬────────────────────────────────┐
//! │ length: u32, little-endian│ body: `length` bytes, postcard │
//! └───────────────────────────┴────────────────────────────────┘
//!   4 bytes                     0..=MAX_FRAME_LEN bytes
//! ```
//!
//! * `length` counts the body only; the 4 prefix bytes are not included.
//! * `length` is little-endian regardless of host endianness. Every target we
//!   support is little-endian anyway, but the format is fixed, not native.
//! * The body is [`postcard`]'s non-self-describing encoding of one value —
//!   for this crate, a [`RequestEnvelope`](crate::protocol::RequestEnvelope) or
//!   a [`ResponseEnvelope`](crate::protocol::ResponseEnvelope). Since the
//!   encoding carries no schema, both peers must agree on the type; that is
//!   what the version field inside the envelope is for.
//! * A `length` greater than the codec's maximum (default
//!   [`MAX_FRAME_LEN`]) is a hard error. It is detected from the prefix alone,
//!   *before* any buffer is reserved, so a hostile or buggy peer cannot make us
//!   allocate 4 GiB by writing four bytes.
//! * A zero `length` is legal in the format but never produced here: postcard
//!   encodes every envelope to at least one byte.

use std::marker::PhantomData;

use serde::{Serialize, de::DeserializeOwned};
use tokio_util::bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::error::Error;

/// Number of bytes in the length prefix.
pub const LENGTH_PREFIX_LEN: usize = 4;

/// Default maximum body size, in bytes.
///
/// One mebibyte is far beyond anything the Phase 2 message set needs (the
/// largest realistic frame is a `Query` response with a few hundred hits) while
/// staying small enough that a malicious peer cannot exhaust memory by opening
/// connections.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

/// A [`tokio_util::codec`] codec for the frame format documented at the
/// [module level](self).
///
/// It decodes `T` and encodes any [`Serialize`] value, so a server uses
/// `FrameCodec<RequestEnvelope>` (decoding requests, encoding responses) and a
/// client uses `FrameCodec<ResponseEnvelope>`.
#[derive(Debug)]
pub struct FrameCodec<T> {
    max_frame_len: usize,
    _item: PhantomData<fn() -> T>,
}

impl<T> FrameCodec<T> {
    /// A codec with the default [`MAX_FRAME_LEN`] limit.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_frame_len(MAX_FRAME_LEN)
    }

    /// A codec with a custom maximum body size.
    #[must_use]
    pub fn with_max_frame_len(max_frame_len: usize) -> Self {
        Self {
            max_frame_len,
            _item: PhantomData,
        }
    }

    /// The maximum body size this codec accepts, in bytes.
    #[must_use]
    pub fn max_frame_len(&self) -> usize {
        self.max_frame_len
    }
}

impl<T> Default for FrameCodec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for FrameCodec<T> {
    fn clone(&self) -> Self {
        Self {
            max_frame_len: self.max_frame_len,
            _item: PhantomData,
        }
    }
}

impl<T, I: Serialize> Encoder<I> for FrameCodec<T> {
    type Error = Error;

    fn encode(&mut self, item: I, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let body = postcard::to_stdvec(&item).map_err(Error::Encode)?;

        if body.len() > self.max_frame_len {
            return Err(Error::FrameTooLarge {
                len: body.len(),
                max: self.max_frame_len,
            });
        }

        // `body.len() <= max_frame_len <= usize::MAX`, and the limit is far
        // below `u32::MAX`, so the cast cannot truncate.
        let len = u32::try_from(body.len()).map_err(|_| Error::FrameTooLarge {
            len: body.len(),
            max: self.max_frame_len,
        })?;

        dst.reserve(LENGTH_PREFIX_LEN + body.len());
        dst.put_u32_le(len);
        dst.put_slice(&body);

        Ok(())
    }
}

impl<T: DeserializeOwned> Decoder for FrameCodec<T> {
    type Item = T;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < LENGTH_PREFIX_LEN {
            return Ok(None);
        }

        let mut prefix = [0u8; LENGTH_PREFIX_LEN];
        prefix.copy_from_slice(&src[..LENGTH_PREFIX_LEN]);
        let len = u32::from_le_bytes(prefix) as usize;

        // Checked before any `reserve`: an unbounded length prefix from a peer
        // must never turn into an allocation.
        if len > self.max_frame_len {
            return Err(Error::FrameTooLarge {
                len,
                max: self.max_frame_len,
            });
        }

        let frame_len = LENGTH_PREFIX_LEN + len;

        if src.len() < frame_len {
            src.reserve(frame_len - src.len());
            return Ok(None);
        }

        src.advance(LENGTH_PREFIX_LEN);
        let body = src.split_to(len);

        postcard::from_bytes(&body).map(Some).map_err(Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Request, RequestEnvelope, Response, ResponseEnvelope};

    fn encoded(env: &RequestEnvelope) -> BytesMut {
        let mut buf = BytesMut::new();
        FrameCodec::<RequestEnvelope>::new()
            .encode(env, &mut buf)
            .unwrap();
        buf
    }

    #[test]
    fn prefix_is_little_endian_body_length() {
        let env = RequestEnvelope::new(7, Request::Ping);
        let buf = encoded(&env);
        let body = postcard::to_stdvec(&env).unwrap();

        assert_eq!(&buf[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&buf[4..], &body[..]);
        assert_eq!(buf.len(), LENGTH_PREFIX_LEN + body.len());
    }

    #[test]
    fn round_trips_a_frame() {
        let env = RequestEnvelope::new(
            1,
            Request::Query {
                text: "kitty".into(),
            },
        );
        let mut buf = encoded(&env);
        let mut codec = FrameCodec::<RequestEnvelope>::new();

        assert_eq!(codec.decode(&mut buf).unwrap(), Some(env));
        assert_eq!(codec.decode(&mut buf).unwrap(), None);
        assert!(buf.is_empty());
    }

    #[test]
    fn decodes_two_frames_from_one_buffer() {
        let a = RequestEnvelope::new(1, Request::Show);
        let b = RequestEnvelope::new(2, Request::Hide);
        let mut buf = BytesMut::new();
        let mut codec = FrameCodec::<RequestEnvelope>::new();
        codec.encode(&a, &mut buf).unwrap();
        codec.encode(&b, &mut buf).unwrap();

        assert_eq!(codec.decode(&mut buf).unwrap(), Some(a));
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(b));
        assert_eq!(codec.decode(&mut buf).unwrap(), None);
    }

    #[test]
    fn empty_buffer_yields_nothing() {
        let mut buf = BytesMut::new();
        assert_eq!(
            FrameCodec::<RequestEnvelope>::new()
                .decode(&mut buf)
                .unwrap(),
            None
        );
    }

    #[test]
    fn partial_prefix_yields_nothing() {
        let mut buf = BytesMut::from(&[0xffu8, 0xff, 0xff][..]);
        // Only three prefix bytes: not even the length is known yet, so an
        // absurd fourth byte must not be guessed at.
        assert_eq!(
            FrameCodec::<RequestEnvelope>::new()
                .decode(&mut buf)
                .unwrap(),
            None
        );
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn oversized_prefix_errors_without_reserving() {
        let mut buf = BytesMut::new();
        buf.put_u32_le(u32::MAX);
        let before = buf.capacity();

        let err = FrameCodec::<RequestEnvelope>::new()
            .decode(&mut buf)
            .unwrap_err();

        assert!(
            matches!(err, Error::FrameTooLarge { len, max } if len == u32::MAX as usize && max == MAX_FRAME_LEN),
            "unexpected error: {err:?}"
        );
        // The buffer must not have grown towards the announced 4 GiB.
        assert_eq!(buf.capacity(), before);
        assert!(buf.capacity() < MAX_FRAME_LEN);
    }

    #[test]
    fn encoding_over_the_limit_errors() {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(16);
        let mut buf = BytesMut::new();
        let env = RequestEnvelope::new(
            1,
            Request::Query {
                text: "x".repeat(64),
            },
        );

        let err = codec.encode(&env, &mut buf).unwrap_err();

        assert!(
            matches!(err, Error::FrameTooLarge { max: 16, .. }),
            "unexpected error: {err:?}"
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn a_frame_at_the_limit_is_accepted() {
        // Round-trip a frame whose body is exactly as large as the limit.
        let env = RequestEnvelope::new(
            1,
            Request::Query {
                text: "y".repeat(4096),
            },
        );
        let body_len = postcard::to_stdvec(&env).unwrap().len();
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(body_len);
        let mut buf = BytesMut::new();

        codec.encode(&env, &mut buf).unwrap();
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(env));
    }

    #[test]
    fn garbage_body_is_a_decode_error_not_a_panic() {
        let mut buf = BytesMut::new();
        buf.put_u32_le(3);
        buf.put_slice(&[0xff, 0xff, 0xff]);

        let err = FrameCodec::<RequestEnvelope>::new()
            .decode(&mut buf)
            .unwrap_err();
        assert!(matches!(err, Error::Decode(_)), "unexpected error: {err:?}");
    }

    #[test]
    fn responses_round_trip_too() {
        let env = ResponseEnvelope::new(
            9,
            Response::Pong {
                protocol_version: 1,
                pid: 4242,
            },
        );
        let mut buf = BytesMut::new();
        let mut codec = FrameCodec::<ResponseEnvelope>::new();
        codec.encode(&env, &mut buf).unwrap();
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(env));
    }
}
