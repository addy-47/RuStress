//! ============================================================================
//! custom_endpoint.rs — load test a target you define, not the built-in one
//! ============================================================================
//! Category     : Utility Tool
//! Component    : core::Config, runner::LoadEngine
//! Prerequisites: none
//! Execution    : cargo run --release --example custom_endpoint
//! Metrics      : status-code distribution and service-time percentiles
//!
//! The built-in target is convenient but not representative. This stands up an
//! in-process `axum` route with the shape you actually want to measure — here a
//! mixed workload where one request in eight returns 503 — and points the
//! generator at it.
//!
//! The point of the mix is that a load test where everything succeeds mostly
//! measures the generator. Failures are the interesting part, and a status-code
//! distribution is the thing that tells you whether the failures are spread
//! evenly or concentrated in a tail.
//!
//! Point `url` at a real service to use this against production: everything
//! except the in-process route is identical.
//! ============================================================================

#[path = "common/mod.rs"]
mod shared;

use axum::Router;
use axum::extract::State;
use axum::routing::get;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustress::core::Config;
use shared::report;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A deterministic 1-in-8 failure rate, so the run is reproducible.
    let served = Arc::new(AtomicU64::new(0));
    let router = Router::new()
        .route(
            "/mixed",
            get({
                let served = Arc::clone(&served);
                move |State(_): State<()>| {
                    let n = served.fetch_add(1, Ordering::Relaxed);
                    async move {
                        if n % 8 == 7 {
                            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "upstream busy")
                        } else {
                            (axum::http::StatusCode::OK, "ok")
                        }
                    }
                }
            }),
        )
        .with_state(());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    println!("target: http://{addr}/mixed  (1 in 8 returns 503)");

    let cfg = Config {
        url: format!("http://{addr}/mixed"),
        target_rps: 400,
        steady_dur_secs: 5,
        timeout_secs: 10,
        ..Default::default()
    };

    let snap = shared::run_to_completion(cfg).await?;
    report("400 RPS against a 12.5% failure rate", &snap);

    println!("\n  status codes");
    let mut codes: Vec<_> = snap.status_codes.iter().collect();
    codes.sort();
    for (code, count) in codes {
        println!("    {code}: {count}");
    }
    Ok(())
}
