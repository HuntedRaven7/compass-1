//! Frame-level tests: exhaustive variant round-trips, partial frames, and the
//! oversized-frame guard.

use compass_ipc::codec::{FrameCodec, LENGTH_PREFIX_LEN, MAX_FRAME_LEN};
use compass_ipc::{
    DoctorCheck, DoctorStatus, Error, ErrorKind, PROTOCOL_VERSION, ProtocolError, QueryHit,
    Request, RequestEnvelope, Response, ResponseEnvelope,
};
use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Every `Request` variant. Kept exhaustive by the `match` in
/// [`request_variants_are_exhaustive`].
fn all_requests() -> Vec<Request> {
    vec![
        Request::Ping,
        Request::Toggle,
        Request::Show,
        Request::Hide,
        Request::Query {
            text: String::new(),
        },
        Request::Query {
            text: "firefox --private".into(),
        },
        Request::Query {
            text: "unicode: é 漢字 🚀".into(),
        },
        Request::Doctor,
        Request::Shutdown,
    ]
}

/// Every `Response` variant.
fn all_responses() -> Vec<Response> {
    vec![
        Response::Pong {
            protocol_version: PROTOCOL_VERSION,
            pid: 1,
        },
        Response::Pong {
            protocol_version: u16::MAX,
            pid: u32::MAX,
        },
        Response::Ack,
        Response::QueryResults { hits: vec![] },
        Response::QueryResults {
            hits: vec![
                QueryHit {
                    id: "app:firefox.desktop".into(),
                    title: "Firefox".into(),
                    subtitle: Some("Web Browser".into()),
                    score: 100,
                },
                QueryHit {
                    id: "x".into(),
                    title: "y".into(),
                    subtitle: None,
                    score: 0,
                },
            ],
        },
        Response::DoctorReport { checks: vec![] },
        Response::DoctorReport {
            checks: vec![
                DoctorCheck {
                    name: "portal.global-shortcuts".into(),
                    status: DoctorStatus::Ok,
                    detail: None,
                },
                DoctorCheck {
                    name: "shell-extension".into(),
                    status: DoctorStatus::Warn,
                    detail: Some("not installed; window management degraded".into()),
                },
                DoctorCheck {
                    name: "ipc.socket".into(),
                    status: DoctorStatus::Fail,
                    detail: Some("permission denied".into()),
                },
            ],
        },
        Response::ShuttingDown,
        Response::Error(ProtocolError::new(ErrorKind::VersionMismatch, "v2 vs v1")),
        Response::Error(ProtocolError::new(ErrorKind::Unsupported, "")),
        Response::Error(ProtocolError::new(ErrorKind::BadRequest, "empty query")),
        Response::Error(ProtocolError::new(ErrorKind::Internal, "handler panicked")),
    ]
}

#[test]
fn request_variants_are_exhaustive() {
    // If a variant is added to `Request`, this stops compiling and whoever adds
    // it has to extend `all_requests`.
    for request in all_requests() {
        match request {
            Request::Ping
            | Request::Toggle
            | Request::Show
            | Request::Hide
            | Request::Query { .. }
            | Request::Doctor
            | Request::Shutdown => {}
        }
    }
}

#[test]
fn response_variants_are_exhaustive() {
    for response in all_responses() {
        match response {
            Response::Pong { .. }
            | Response::Ack
            | Response::QueryResults { .. }
            | Response::DoctorReport { .. }
            | Response::ShuttingDown
            | Response::Error(_) => {}
        }
    }
}

#[test]
fn every_request_variant_round_trips() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    for (id, request) in all_requests().into_iter().enumerate() {
        let envelope = RequestEnvelope::new(id as u64, request);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        let decoded = codec.decode(&mut buf).unwrap();
        assert_eq!(decoded, Some(envelope));
        assert!(buf.is_empty(), "codec left trailing bytes");
    }
}

#[test]
fn every_response_variant_round_trips() {
    let mut codec = FrameCodec::<ResponseEnvelope>::new();

    for (id, response) in all_responses().into_iter().enumerate() {
        let envelope = ResponseEnvelope::new(id as u64, response);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        assert_eq!(codec.decode(&mut buf).unwrap(), Some(envelope));
        assert!(buf.is_empty());
    }
}

#[test]
fn all_variants_round_trip_back_to_back_in_one_stream() {
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();

    let envelopes: Vec<_> = all_requests()
        .into_iter()
        .enumerate()
        .map(|(i, r)| RequestEnvelope::new(i as u64, r))
        .collect();

    for envelope in &envelopes {
        codec.encode(envelope, &mut buf).unwrap();
    }

    for expected in &envelopes {
        assert_eq!(codec.decode(&mut buf).unwrap().as_ref(), Some(expected));
    }
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn a_frame_fed_one_byte_at_a_time_yields_exactly_one_message_at_the_end() {
    let envelope = RequestEnvelope::new(
        42,
        Request::Query {
            text: "a moderately long query".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();
    let wire = wire.freeze();
    assert!(wire.len() > LENGTH_PREFIX_LEN);

    let mut codec = FrameCodec::<RequestEnvelope>::new();
    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();

    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        let decoded = codec.decode(&mut buf).unwrap();

        if index + 1 < wire.len() {
            assert!(
                decoded.is_none(),
                "decoder produced a message after only {} bytes",
                index + 1
            );
        }
        if let Some(item) = decoded {
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![envelope]);
    assert!(buf.is_empty());
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
}

#[test]
fn two_frames_fed_one_byte_at_a_time_yield_two_messages_in_order() {
    let a = RequestEnvelope::new(1, Request::Toggle);
    let b = RequestEnvelope::new(
        2,
        Request::Query {
            text: "second".into(),
        },
    );

    let mut wire = BytesMut::new();
    let mut codec = FrameCodec::<RequestEnvelope>::new();
    codec.encode(&a, &mut wire).unwrap();
    let boundary = wire.len();
    codec.encode(&b, &mut wire).unwrap();
    let wire = wire.freeze();

    let mut buf = BytesMut::new();
    let mut yielded = Vec::new();
    for (index, byte) in wire.iter().enumerate() {
        buf.put_u8(*byte);
        if let Some(item) = codec.decode(&mut buf).unwrap() {
            // The only two byte offsets that may complete a message are the
            // last byte of each frame.
            assert!(index + 1 == boundary || index + 1 == wire.len());
            yielded.push(item);
        }
    }

    assert_eq!(yielded, vec![a, b]);
}

#[test]
fn a_truncated_frame_never_yields_a_message() {
    let envelope = RequestEnvelope::new(
        7,
        Request::Query {
            text: "truncated".into(),
        },
    );
    let mut wire = BytesMut::new();
    FrameCodec::<RequestEnvelope>::new()
        .encode(&envelope, &mut wire)
        .unwrap();

    // Drop the final byte: the decoder must wait forever rather than guess.
    let truncated = &wire[..wire.len() - 1];
    let mut buf = BytesMut::from(truncated);
    let mut codec = FrameCodec::<RequestEnvelope>::new();

    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    assert_eq!(codec.decode(&mut buf).unwrap(), None);
    assert_eq!(
        buf.len(),
        truncated.len(),
        "a partial frame must stay buffered"
    );
}

#[test]
fn an_oversized_length_prefix_is_rejected_without_allocating() {
    for announced in [MAX_FRAME_LEN as u32 + 1, u32::MAX, u32::MAX - 1, 1 << 30] {
        let mut buf = BytesMut::with_capacity(LENGTH_PREFIX_LEN);
        buf.put_u32_le(announced);
        let capacity_before = buf.capacity();

        let err = FrameCodec::<RequestEnvelope>::new()
            .decode(&mut buf)
            .unwrap_err();

        match err {
            Error::FrameTooLarge { len, max } => {
                assert_eq!(len, announced as usize);
                assert_eq!(max, MAX_FRAME_LEN);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }

        // The whole point: four bytes from a peer must not turn into a
        // multi-gigabyte reservation.
        assert_eq!(buf.capacity(), capacity_before);
        assert!(buf.capacity() <= LENGTH_PREFIX_LEN * 2);
    }
}

#[test]
fn the_size_limit_is_configurable_and_enforced_on_both_sides() {
    let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(32);
    assert_eq!(codec.max_frame_len(), 32);

    let big = RequestEnvelope::new(
        1,
        Request::Query {
            text: "z".repeat(1000),
        },
    );
    let mut buf = BytesMut::new();
    assert!(matches!(
        codec.encode(&big, &mut buf).unwrap_err(),
        Error::FrameTooLarge { max: 32, .. }
    ));

    let mut wire = BytesMut::new();
    wire.put_u32_le(33);
    assert!(matches!(
        codec.decode(&mut wire).unwrap_err(),
        Error::FrameTooLarge { len: 33, max: 32 }
    ));
}

#[test]
fn a_well_framed_but_nonsense_body_is_an_error_not_a_panic() {
    for body in [vec![0xff, 0xff, 0xff, 0xff], vec![0x00], vec![0x7f; 12]] {
        let mut buf = BytesMut::new();
        buf.put_u32_le(body.len() as u32);
        buf.put_slice(&body);

        // Either it decodes to something (postcard is not self-describing, so
        // some byte strings are valid) or it errors. It must never panic and it
        // must always consume the frame.
        let result = FrameCodec::<RequestEnvelope>::new().decode(&mut buf);
        assert!(buf.is_empty(), "the frame body should have been consumed");
        let _ = result;
    }
}

#[test]
fn error_messages_are_legible() {
    let err = Error::FrameTooLarge {
        len: 5_000_000,
        max: MAX_FRAME_LEN,
    };
    assert_eq!(
        err.to_string(),
        "frame of 5000000 bytes exceeds the 1048576 byte limit"
    );

    let err = Error::VersionMismatch {
        expected: 1,
        actual: 9,
    };
    assert!(err.to_string().contains("peer speaks v9"));
    assert!(err.to_string().contains("this build speaks v1"));
}
