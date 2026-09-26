use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use rand::Rng;
use std::time::Duration;
use tracing::info;

/// Built-in test HTTP server with varied endpoints for load testing.
pub struct DummyServer {
    port: u16,
}

impl DummyServer {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    /// Build the reference target's route table without binding a socket.
    ///
    /// Exposed so a caller can bind an ephemeral port and read back the
    /// address the OS actually assigned. [`DummyServer::run`] takes a port and
    /// reports none, so a caller that needs a known-free port — every
    /// integration test, and any CI runner running targets in parallel — would
    /// otherwise have to guess one and race for it.
    pub fn router() -> Router {
        Router::new()
            .route("/fast", get(handler_fast))
            .route("/medium", get(handler_medium))
            .route("/slow", get(handler_slow))
            .route("/spike", get(handler_spike))
            .route("/error", get(handler_error))
            .route("/big", get(handler_big))
            .route("/big-ok", get(handler_big_ok))
            .route("/sized", get(handler_sized))
            .with_state(())
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let app = Self::router();

        let addr = format!("0.0.0.0:{}", self.port);
        info!("Dummy server listening on {}", addr);
        info!("  /fast    — 10-50ms jitter");
        info!("  /medium  — 100-300ms jitter");
        info!("  /slow    — 1000-2000ms jitter");
        info!("  /spike   — 95% 20ms, 5% 2000ms");
        info!("  /error   — 20% 500, 20% 429, 60% 200");
        info!("  /big     — 8 MB 500 body (memory-bound regression fixture)");
        info!("  /big-ok  — 8 MB 200 body (throughput byte-counting fixture)");
        info!("  /sized?bytes=N — 500 with a body of exactly N bytes (capture-cap fixture)");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

async fn handler_fast() -> impl IntoResponse {
    let ms = rand::thread_rng().gen_range(10..50);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    (StatusCode::OK, format!("fast: {}ms", ms))
}

async fn handler_medium() -> impl IntoResponse {
    let ms = rand::thread_rng().gen_range(100..300);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    (StatusCode::OK, format!("medium: {}ms", ms))
}

async fn handler_slow() -> impl IntoResponse {
    let ms = rand::thread_rng().gen_range(1000..2000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
    (StatusCode::OK, format!("slow: {}ms", ms))
}

async fn handler_spike() -> impl IntoResponse {
    let r: u32 = rand::thread_rng().gen_range(0..100);
    if r < 95 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        (StatusCode::OK, "spike: 20ms")
    } else {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        (StatusCode::OK, "spike: 2000ms")
    }
}

async fn handler_error() -> impl IntoResponse {
    let r: u32 = rand::thread_rng().gen_range(0..100);
    if r < 20 {
        (StatusCode::INTERNAL_SERVER_ERROR, "error: 500")
    } else if r < 40 {
        (StatusCode::TOO_MANY_REQUESTS, "error: 429")
    } else {
        (StatusCode::OK, "ok: 200")
    }
}

/// Serve an 8 MB error body.
///
/// This is the regression fixture for the original OOM crash: a load generator
/// that buffers one of these per in-flight request retains
/// `8 MB x concurrency` and exhausts host memory. Used by
/// `make verify-crash-safe` and by the memory-bound integration test.
async fn handler_big() -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        vec![b'x'; BIG_BODY_BYTES],
    )
}

/// Serve an 8 MB success body, for verifying throughput byte counting.
async fn handler_big_ok() -> impl IntoResponse {
    (StatusCode::OK, vec![b'x'; BIG_BODY_BYTES])
}

/// Query parameters for the exact-size fixture.
#[derive(serde::Deserialize)]
struct SizedQuery {
    bytes: usize,
}

/// Serve a 500 whose body is exactly `bytes` long.
///
/// [`crate::core::constants::MAX_CAPTURED_BODY_BYTES`] is a boundary, not a
/// range. A body of exactly that many bytes is a *complete* capture and must
/// not be marked truncated; one byte more must be. Only a server that can
/// produce an exact size can prove that difference, and a fixture that offers
/// nothing but "large" leaves the off-by-one invisible.
///
/// The size arrives as a query parameter rather than a path parameter so the
/// fixture depends on no route-pattern syntax.
///
/// The size is capped at [`crate::core::constants::MAX_DRAINED_BODY_BYTES`] so
/// the fixture itself cannot become the memory hazard it exists to detect.
async fn handler_sized(
    axum::extract::Query(query): axum::extract::Query<SizedQuery>,
) -> axum::response::Response {
    use crate::core::constants::MAX_DRAINED_BODY_BYTES;

    if query.bytes > MAX_DRAINED_BODY_BYTES as usize {
        return (
            StatusCode::BAD_REQUEST,
            format!("bytes must not exceed {MAX_DRAINED_BODY_BYTES}"),
        )
            .into_response();
    }
    (StatusCode::INTERNAL_SERVER_ERROR, vec![b'x'; query.bytes]).into_response()
}

/// Size of the large-body regression fixture.
pub const BIG_BODY_BYTES: usize = 8 * 1024 * 1024;
