//! ============================================================================
//! throughput_bytes_test — byte accounting is measured, not declared
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::executor` response draining; `metrics` byte counter
//! Prerequisites: none (real axum targets on OS-assigned ephemeral ports)
//! Execution    : cargo test --test throughput_bytes_test
//! Metrics      : per-request `bytes`, aggregate `StatsSnapshot::bytes`
//! ============================================================================
//!
//! `Response::content_length()` is `None` for a chunked response. A client that
//! trusts it reports zero throughput against a target that streamed eight
//! megabytes — a number that looks like a result and is not. Two fixtures are
//! needed to tell those worlds apart: one with a `content-length` and one
//! without. The chunked fixture is built in-test because the reference target
//! only had the sized form.

mod common;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;

use rustress::core::snapshot::StatsSnapshot;
use rustress::dummy::server::BIG_BODY_BYTES;

use common::{Harness, TestServer, cfg_for, dummy_server, serve};

/// Wire size of each streamed chunk.
const CHUNK_BYTES: usize = 8 * 1024;

/// Number of chunks streamed, for `CHUNKED_TOTAL_BYTES` overall.
const CHUNK_COUNT: usize = 1024;

/// Total bytes the chunked fixture puts on the wire, with no `content-length`.
const CHUNKED_TOTAL_BYTES: u64 = (CHUNK_BYTES * CHUNK_COUNT) as u64;

/// One immutable chunk, shared by every streamed frame.
///
/// Reusing one static buffer keeps the fixture's own footprint at 8 KB so it
/// cannot be mistaken for the client-side memory under test.
static CHUNK: [u8; CHUNK_BYTES] = [b'x'; CHUNK_BYTES];

/// A real `200` whose body arrives as a chunked transfer with no
/// `content-length` header.
///
/// `stream::unfold` is deliberate: `stream::iter` over a `Vec` reports an exact
/// size hint, hyper would set `content-length` from it, and the fixture would
/// silently stop testing the thing it exists to test.
async fn handler_chunked() -> Response {
    let stream = futures_util::stream::unfold(0usize, |sent| async move {
        if sent >= CHUNK_COUNT {
            return None;
        }
        Some((
            Ok::<Bytes, std::io::Error>(Bytes::from_static(&CHUNK)),
            sent + 1,
        ))
    });
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(stream))
        .expect("a statically-built response cannot fail to build")
}

async fn chunked_server() -> TestServer {
    serve(Router::new().route("/chunked", get(handler_chunked))).await
}

/// A config whose run transfers a bounded amount of large-body traffic.
fn big_body_config(url: &str) -> rustress::core::config::Config {
    let mut cfg = cfg_for(url);
    cfg.target_rps = 20;
    cfg.steady_dur_secs = 1;
    cfg.max_concurrency = 8;
    cfg.timeout_secs = 10;
    cfg
}

/// Assert every retained result reported exactly `expected` bytes, and that
/// the aggregate is the exact sum of the per-request counts.
fn assert_bytes_measured(
    results: &[rustress::core::result::ExperimentResult],
    snap: &StatsSnapshot,
    expected: u64,
    label: &str,
) {
    assert!(
        !results.is_empty(),
        "{label}: the engine executed no requests, so nothing was proven about byte counting"
    );
    for (i, r) in results.iter().enumerate() {
        assert_eq!(
            r.bytes as u64, expected,
            "{label}: request {i} counted {} bytes, expected {expected}",
            r.bytes
        );
    }
    assert_eq!(
        snap.bytes,
        results.len() as u64 * expected,
        "{label}: the aggregate must be the exact sum of the per-request counts, \
         not a re-derived or clamped figure"
    );
}

/// A chunked body with no `content-length` is still measured in full.
///
/// Catches an executor that reads `content_length()` instead of the stream:
/// every request would report `0` bytes and the run would claim zero throughput
/// against a target that actually sent eight megabytes per request.
#[tokio::test]
async fn chunked_bytes_are_counted_from_the_stream_not_from_content_length() {
    let server = chunked_server().await;

    // Fixture validation: prove the response really has no content-length,
    // otherwise the assertions below would pass even against a client that
    // trusted the header.
    let probe = reqwest::Client::new()
        .get(server.url("/chunked"))
        .send()
        .await
        .expect("the chunked fixture must answer a direct GET");
    assert_eq!(probe.status(), StatusCode::OK);
    assert_eq!(
        probe.content_length(),
        None,
        "the chunked fixture must not advertise a content-length, or it is not \
         testing chunked byte counting"
    );
    let mut probed = 0u64;
    let mut probe_body = probe;
    while let Some(chunk) = probe_body
        .chunk()
        .await
        .expect("draining the fixture must succeed")
    {
        probed += chunk.len() as u64;
    }
    assert_eq!(
        probed, CHUNKED_TOTAL_BYTES,
        "the fixture must actually put {CHUNKED_TOTAL_BYTES} bytes on the wire"
    );

    let harness = Harness::new(big_body_config(&server.url("/chunked")))
        .expect("config should construct an engine");
    harness.run().await;

    let results = harness.results();
    let snapshot = harness.snapshot();
    assert_bytes_measured(&results, &snapshot, CHUNKED_TOTAL_BYTES, "chunked");
}

/// A `content-length` body is measured identically, so the two paths agree.
///
/// Catches a drain that double-counts when a length *is* advertised, or that
/// charges the length header plus the streamed bytes.
#[tokio::test]
async fn content_length_bytes_are_counted_exactly_once() {
    let server = dummy_server().await;
    let probe = reqwest::Client::new()
        .get(server.url("/big-ok"))
        .send()
        .await
        .expect("the sized fixture must answer a direct GET");
    assert_eq!(
        probe.content_length(),
        Some(BIG_BODY_BYTES as u64),
        "this fixture must advertise a content-length, or it is not the control"
    );

    let harness = Harness::new(big_body_config(&server.url("/big-ok")))
        .expect("config should construct an engine");
    harness.run().await;

    let results = harness.results();
    let snapshot = harness.snapshot();
    assert_bytes_measured(&results, &snapshot, BIG_BODY_BYTES as u64, "content-length");
}

/// A failed response's body is still counted.
///
/// Catches a counter that only accumulates on success, which would report zero
/// throughput for exactly the responses an operator is investigating.
#[tokio::test]
async fn error_response_bytes_are_counted_too() {
    let server = dummy_server().await;
    let harness = Harness::new(big_body_config(&server.url("/big")))
        .expect("config should construct an engine");
    harness.run().await;

    let results = harness.results();
    let snapshot = harness.snapshot();
    assert!(snapshot.fail > 0, "the /big fixture always returns 500");
    assert_bytes_measured(&results, &snapshot, BIG_BODY_BYTES as u64, "500");
}
