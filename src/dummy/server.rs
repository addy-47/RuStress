use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use rand::Rng;
use std::time::Duration;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Built-in test HTTP server with varied endpoints for load testing.
pub struct DummyServer {
    port: u16,
}

impl DummyServer {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/fast", get(handler_fast))
            .route("/medium", get(handler_medium))
            .route("/slow", get(handler_slow))
            .route("/spike", get(handler_spike))
            .route("/error", get(handler_error))
            .layer(TraceLayer::new_for_http())
            .with_state(());

        let addr = format!("0.0.0.0:{}", self.port);
        info!("Dummy server listening on {}", addr);
        info!("  /fast    — 10-50ms jitter");
        info!("  /medium  — 100-300ms jitter");
        info!("  /slow    — 1000-2000ms jitter");
        info!("  /spike   — 95% 20ms, 5% 2000ms");
        info!("  /error   — 20% 500, 20% 429, 60% 200");

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
