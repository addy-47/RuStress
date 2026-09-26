//! ============================================================================
//! error_path_test — transport failures reach the counters
//! ============================================================================
//! Category     : Integration Test
//! Component    : `runner::executor` (stream error capture) + `metrics::collector`
//! Prerequisites: none (raw `TcpListener` writing real bytes, plus a real
//!                unreachable port)
//! Execution    : cargo test --test error_path_test
//! Metrics      : `StatsSnapshot::error_counts`, `status_codes`, `success`, `fail`
//! ============================================================================
//!
//! These are the tests the Zero-Mock Rule is written for. The failures are
//! produced by a raw socket that writes a malformed response and closes — no
//! fake client, no stubbed transport, no `#[cfg(test)]` production branch. The
//! path exercised is the real one: a real `reqwest` request, a real truncated
//! body, real accounting.
//!
//! # Known failing test
//!
//! `a_truncated_response_is_counted_as_a_failure_not_a_success` fails against
//! current `main`. It is a reported production defect at
//! `src/runner/executor.rs:62` — see the test's own documentation. It is left
//! red deliberately: weakening it would hide the defect, and this role does not
//! fix production code.

mod common;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use common::{Harness, cfg_for};

/// A real socket that answers every connection with `response`, then closes.
struct RawTarget {
    port: u16,
    handle: JoinHandle<()>,
}

impl Drop for RawTarget {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Accept connections and write `response` to each, then hang up.
async fn raw_target(response: &'static [u8]) -> RawTarget {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("read bound port").port();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let _ = stream.write_all(response).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
        }
    });
    RawTarget { port, handle }
}

/// A real port that nothing is listening on.
///
/// Bound then released, so a connect to it is refused by the kernel — a genuine
/// transport failure, not a synthesised one.
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    listener.local_addr().expect("read bound port").port()
}

/// Run against `url` long enough to make several attempts, then return the
/// snapshot.
async fn run_against(url: &str) -> rustress::core::snapshot::StatsSnapshot {
    let mut cfg = cfg_for(url);
    cfg.target_rps = 10;
    cfg.steady_dur_secs = 1;
    cfg.timeout_secs = 2;
    let harness = Harness::new(cfg).expect("config should construct an engine");
    harness.run().await;
    harness.snapshot()
}

/// A body that stops mid-transfer is a failed request, not a success.
///
/// **This test currently fails. It is a reported production defect, not a flaky
/// test.** `runner/executor.rs:62` derives `success` from the status code
/// alone:
///
/// ```text
/// let success = (200..300).contains(&outcome.status);
/// ```
///
/// `outcome.error` is discarded. A `200` whose body errors mid-transfer is
/// therefore recorded as a success while its error message is filed under
/// `error_counts`, so a run can report 100% success against a target that is
/// returning unusable responses and no counter disagrees.
///
/// Do not weaken the assertions below to make this pass. Fix the executor.
///
/// Catches an executor that derives `success` from the status code alone. The
/// status was a perfectly good `200`; the response never finished arriving.
#[tokio::test]
async fn a_truncated_response_is_counted_as_a_failure_not_a_success() {
    // 200 OK, promise 100 bytes, deliver 10, then close.
    let target = raw_target(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Type: text/plain\r\n\r\n0123456789",
    )
    .await;

    let snapshot = run_against(&format!("http://127.0.0.1:{}/truncated", target.port)).await;

    assert!(
        snapshot.requests > 0,
        "the engine must have attempted requests, otherwise this proves nothing"
    );
    assert!(
        !snapshot.error_counts.is_empty(),
        "a body that stopped mid-transfer must be recorded as an error: {:?}",
        snapshot.error_counts
    );
    assert_eq!(
        snapshot.fail, snapshot.requests,
        "DEFECT (src/runner/executor.rs:62): every request that failed to complete \
         must be counted as a failure, not a success. Got success={} fail={} \
         requests={}. `success` is derived from the status code alone and ignores \
         `outcome.error`.",
        snapshot.success, snapshot.fail, snapshot.requests
    );
    assert_eq!(
        snapshot.success, 0,
        "DEFECT (src/runner/executor.rs:62): a 200 that never delivered its body is \
         not a success. Got success={}",
        snapshot.success
    );
    assert_eq!(
        snapshot.fail + snapshot.success,
        snapshot.requests,
        "success and failure counts must partition the request count"
    );
}

/// A refused connection is recorded as a transport error with no status code.
///
/// Catches a connection failure being folded into the status-code map, which
/// would report `0` as a response code the target never sent.
#[tokio::test]
async fn a_refused_connection_is_a_transport_error_with_no_status_code() {
    let port = closed_port().await;
    let snapshot = run_against(&format!("http://127.0.0.1:{port}/refused")).await;

    assert!(snapshot.requests > 0, "no requests attempted");
    assert_eq!(
        snapshot.success, 0,
        "a refused connection cannot be a success"
    );
    assert_eq!(snapshot.fail, snapshot.requests);
    assert!(
        !snapshot.error_counts.is_empty(),
        "the connection error must be recorded: {:?}",
        snapshot.error_counts
    );
    assert!(
        snapshot.status_codes.is_empty(),
        "a request that never received a response has no status code: {:?}",
        snapshot.status_codes
    );
    assert_eq!(snapshot.bytes, 0, "nothing was transferred");
}

/// The error key space stays bounded even when every failure looks different.
///
/// Catches a collector that keys on the full error text including the target's
/// host and port, which would still be a small space here — but the assertion
/// that matters is that failures are counted, and the collector must not
/// silently drop them once the budget is spent.
#[tokio::test]
async fn repeated_transport_failures_accumulate_under_one_key() {
    let port = closed_port().await;
    let snapshot = run_against(&format!("http://127.0.0.1:{port}/refused")).await;

    let total_keyed: u64 = snapshot.error_counts.values().sum();
    assert_eq!(
        total_keyed, snapshot.requests,
        "every failed request must be counted under some error key; \
         requests={} keyed={total_keyed}",
        snapshot.requests
    );
    assert!(
        snapshot.error_counts.len() <= rustress::core::constants::MAX_TRACKED_ERROR_KEYS + 1,
        "the error key space must stay bounded: {:?}",
        snapshot.error_counts.keys().collect::<Vec<_>>()
    );
}

/// A server that accepts and hangs up without a byte must not be reported as a
/// successful empty response.
///
/// Catches a client that treats an immediately-closed connection as a valid
/// zero-length `200`.
#[tokio::test]
async fn a_connection_closed_before_any_response_is_a_failure() {
    let target = raw_target(b"").await;
    let snapshot = run_against(&format!("http://127.0.0.1:{}/empty", target.port)).await;

    assert!(snapshot.requests > 0, "no requests attempted");
    assert_eq!(snapshot.success, 0, "no response was ever received");
    assert_eq!(snapshot.fail, snapshot.requests);
    assert!(!snapshot.error_counts.is_empty());
}

/// A response that is cut off must still contribute the bytes it did deliver.
///
/// Catches a counter that discards partial transfers, which under-reports
/// throughput against any target that resets connections.
#[tokio::test]
async fn a_truncated_response_still_counts_the_bytes_it_delivered() {
    let target = raw_target(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Type: text/plain\r\n\r\n0123456789",
    )
    .await;

    let mut cfg = cfg_for(&format!("http://127.0.0.1:{}/truncated", target.port));
    cfg.target_rps = 5;
    cfg.steady_dur_secs = 1;
    cfg.timeout_secs = 2;
    let harness = Harness::new(cfg).expect("config should construct an engine");
    harness.run().await;

    let results = harness.results();
    assert!(!results.is_empty(), "no requests attempted");
    for r in &results {
        assert_eq!(
            r.bytes, 10,
            "the 10 bytes the target did deliver must be counted, even though \
             the transfer as a whole failed"
        );
    }
}

/// The raw target is a real socket speaking real HTTP.
///
/// Guards the fixture itself. Every assertion above is only as trustworthy as
/// the target producing the failure; if it stopped speaking HTTP, the tests
/// would be measuring a different thing without failing.
#[tokio::test]
async fn the_raw_target_is_a_real_socket_speaking_real_http() {
    let target = raw_target(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n").await;
    let mut stream = TcpStream::connect(("127.0.0.1", target.port))
        .await
        .expect("the raw target must accept a real connection");
    let mut buf = [0u8; 64];
    let read = stream
        .read(&mut buf)
        .await
        .expect("the raw target must send its bytes");
    let text = String::from_utf8_lossy(&buf[..read]);
    assert!(
        text.starts_with("HTTP/1.1 204 No Content"),
        "the fixture must speak real HTTP, got {text:?}"
    );
}
