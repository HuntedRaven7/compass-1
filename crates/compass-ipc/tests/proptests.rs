//! Property tests for the framing layer.
//!
//! Two properties matter here:
//!
//! 1. anything we can encode, we can decode back to the same value;
//! 2. anything a *peer* can send — arbitrary bytes, in arbitrary chunk sizes —
//!    produces a message or an error, never a panic and never an unbounded
//!    allocation.

use compass_ipc::codec::FrameCodec;
use compass_ipc::{
    DoctorCheck, DoctorStatus, ErrorKind, ProtocolError, QueryHit, Request, RequestEnvelope,
    Response, ResponseEnvelope,
};
use proptest::prelude::*;
use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Where a counterexample gets written so it can be replayed.
///
/// proptest's default `SourceParallel` persistence looks for `lib.rs` or `main.rs` beside
/// the test file. An integration test under `tests/` has neither, so proptest prints
/// "failed to find lib.rs or main.rs" and *discards the seed*. A property that fails on
/// one seed in a few hundred is then unreplayable -- and that is precisely the failure
/// worth keeping, since it will not reproduce on the next run. Writing the seed to a
/// checked-in file turns each such find into a permanent regression case.
fn regressions(cases: u32, file: &'static str) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::Direct(file),
        )),
        ..ProptestConfig::default()
    }
}

/// Small enough that a hostile length prefix in the fuzz tests would be caught
/// immediately, large enough that generated envelopes still fit.
const TEST_MAX_FRAME_LEN: usize = 64 * 1024;

fn request_strategy() -> impl Strategy<Value = Request> {
    prop_oneof![
        Just(Request::Ping),
        Just(Request::Toggle),
        Just(Request::Show),
        Just(Request::Hide),
        any::<String>().prop_map(|text| Request::Query { text }),
        Just(Request::Doctor),
        Just(Request::Shutdown),
    ]
}

fn query_hit_strategy() -> impl Strategy<Value = QueryHit> {
    (
        any::<String>(),
        any::<String>(),
        any::<Option<String>>(),
        0u32..=100,
    )
        .prop_map(|(id, title, subtitle, score)| QueryHit {
            id,
            title,
            subtitle,
            score,
        })
}

fn doctor_status_strategy() -> impl Strategy<Value = DoctorStatus> {
    prop_oneof![
        Just(DoctorStatus::Ok),
        Just(DoctorStatus::Warn),
        Just(DoctorStatus::Fail)
    ]
}

fn doctor_check_strategy() -> impl Strategy<Value = DoctorCheck> {
    (
        any::<String>(),
        doctor_status_strategy(),
        any::<Option<String>>(),
    )
        .prop_map(|(name, status, detail)| DoctorCheck {
            name,
            status,
            detail,
        })
}

fn error_kind_strategy() -> impl Strategy<Value = ErrorKind> {
    prop_oneof![
        Just(ErrorKind::VersionMismatch),
        Just(ErrorKind::Unsupported),
        Just(ErrorKind::BadRequest),
        Just(ErrorKind::Internal),
    ]
}

fn response_strategy() -> impl Strategy<Value = Response> {
    prop_oneof![
        (any::<u16>(), any::<u32>()).prop_map(|(protocol_version, pid)| Response::Pong {
            protocol_version,
            pid
        }),
        Just(Response::Ack),
        proptest::collection::vec(query_hit_strategy(), 0..16)
            .prop_map(|hits| Response::QueryResults { hits }),
        proptest::collection::vec(doctor_check_strategy(), 0..16)
            .prop_map(|checks| Response::DoctorReport { checks }),
        Just(Response::ShuttingDown),
        (error_kind_strategy(), any::<String>())
            .prop_map(|(kind, message)| Response::Error(ProtocolError { kind, message })),
    ]
}

fn request_envelope_strategy() -> impl Strategy<Value = RequestEnvelope> {
    (any::<u16>(), any::<u64>(), request_strategy()).prop_map(|(version, id, request)| {
        RequestEnvelope {
            version,
            id,
            request,
        }
    })
}

fn response_envelope_strategy() -> impl Strategy<Value = ResponseEnvelope> {
    (any::<u16>(), any::<u64>(), response_strategy()).prop_map(|(version, id, response)| {
        ResponseEnvelope {
            version,
            id,
            response,
        }
    })
}

proptest! {
    #![proptest_config(regressions(256, "tests/regressions/proptests.txt"))]
    #[test]
    fn request_envelopes_round_trip(envelope in request_envelope_strategy()) {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();

        codec.encode(&envelope, &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap();

        prop_assert_eq!(decoded, Some(envelope));
        prop_assert!(buf.is_empty());
    }

    #[test]
    fn response_envelopes_round_trip(envelope in response_envelope_strategy()) {
        let mut codec = FrameCodec::<ResponseEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();

        codec.encode(&envelope, &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap();

        prop_assert_eq!(decoded, Some(envelope));
        prop_assert!(buf.is_empty());
    }

    /// A batch of envelopes written back to back comes out in the same order.
    #[test]
    fn a_stream_of_envelopes_round_trips_in_order(
        envelopes in proptest::collection::vec(request_envelope_strategy(), 0..12),
    ) {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();
        for envelope in &envelopes {
            codec.encode(envelope, &mut buf).unwrap();
        }

        let mut decoded = Vec::new();
        while let Some(item) = codec.decode(&mut buf).unwrap() {
            decoded.push(item);
        }

        prop_assert_eq!(decoded, envelopes);
        prop_assert!(buf.is_empty());
    }

    /// Split an encoded stream at an arbitrary point and feed it in two writes:
    /// the decoder must still yield the same messages and nothing extra.
    #[test]
    fn arbitrary_chunk_boundaries_do_not_change_the_result(
        envelopes in proptest::collection::vec(request_envelope_strategy(), 1..6),
        split in 0usize..4096,
    ) {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut wire = BytesMut::new();
        for envelope in &envelopes {
            codec.encode(envelope, &mut wire).unwrap();
        }
        let wire = wire.freeze();
        let split = split.min(wire.len());

        let mut buf = BytesMut::new();
        let mut decoded = Vec::new();

        for chunk in [&wire[..split], &wire[split..]] {
            buf.put_slice(chunk);
            while let Some(item) = codec.decode(&mut buf).unwrap() {
                decoded.push(item);
            }
        }

        prop_assert_eq!(decoded, envelopes);
        prop_assert!(buf.is_empty());
    }

    /// The load-bearing fuzz property: arbitrary bytes must never panic the
    /// decoder. They may decode, they may error; they may not abort.
    #[test]
    fn arbitrary_bytes_never_panic_the_decoder(
        data in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::from(&data[..]);

        while let Ok(Some(_)) = codec.decode(&mut buf) {}
    }

    /// Same, but with an attacker-chosen length prefix in front, which is the
    /// shape that actually reaches a listening socket.
    #[test]
    fn arbitrary_length_prefixes_never_panic_or_over_allocate(
        announced in any::<u32>(),
        body in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let mut codec = FrameCodec::<RequestEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();
        buf.put_u32_le(announced);
        buf.put_slice(&body);

        let result = codec.decode(&mut buf);

        if announced as usize > TEST_MAX_FRAME_LEN {
            let too_large = matches!(result, Err(compass_ipc::Error::FrameTooLarge { .. }));
            prop_assert!(too_large, "an oversized prefix must be rejected");
        }
        // Whatever happened, the buffer must not have been grown towards the
        // announced size when that size is absurd.
        prop_assert!(buf.capacity() <= TEST_MAX_FRAME_LEN + 1024);
    }

    /// Arbitrary bytes delivered in arbitrary chunks, the way a socket would.
    #[test]
    fn arbitrary_bytes_in_arbitrary_chunks_never_panic(
        chunks in proptest::collection::vec(
            proptest::collection::vec(any::<u8>(), 0..64),
            0..32,
        ),
    ) {
        let mut codec = FrameCodec::<ResponseEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();

        for chunk in chunks {
            buf.put_slice(&chunk);
            loop {
                match codec.decode(&mut buf) {
                    Ok(Some(_)) => continue,
                    Ok(None) => break,
                    Err(_) => {
                        // A hard framing error kills the connection in the real
                        // transport; here we just stop feeding this stream.
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Encoding never produces a frame whose prefix disagrees with its body.
    #[test]
    fn the_prefix_always_matches_the_body_length(envelope in response_envelope_strategy()) {
        let mut codec = FrameCodec::<ResponseEnvelope>::with_max_frame_len(TEST_MAX_FRAME_LEN);
        let mut buf = BytesMut::new();
        codec.encode(&envelope, &mut buf).unwrap();

        let mut prefix = [0u8; 4];
        prefix.copy_from_slice(&buf[..4]);
        prop_assert_eq!(u32::from_le_bytes(prefix) as usize, buf.len() - 4);
    }
}
