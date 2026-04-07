use rustress_core::config::{Config, Mode};
use rustress_core::constants::STATS_UPDATE_INTERVAL_MS;
use rustress_templates::{TemplateContext, TemplateEngine};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::client::build_client;
use crate::executor::{execute_http, execute_script};
use crate::ramp::current_rps;
use crate::stats::RunStats;

/// The main load generation engine.
pub struct LoadEngine {
    cfg: Config,
    client: reqwest::Client,
    template_engine: Arc<TemplateEngine>,
    stats: Arc<RunStats>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl LoadEngine {
    /// Create a new load engine.
    pub fn new(cfg: Config, updates: mpsc::UnboundedSender<rustress_core::snapshot::StatsSnapshot>) -> Self {
        let client = build_client(&cfg);
        let template_engine = Arc::new(TemplateEngine::new());
        let semaphore = Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrency as usize));

        Self {
            cfg,
            client,
            template_engine,
            stats: Arc::new(RunStats::new(updates)),
            semaphore,
        }
    }

    /// Get shared stats reference.
    pub fn stats(&self) -> &Arc<RunStats> {
        &self.stats
    }

    /// Run the load test until cancellation.
    pub async fn run(&self, cancel: CancellationToken) {
        // Start stats tick loop
        let stats = Arc::clone(&self.stats);
        let tick_cancel = cancel.clone();
        let tick_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(STATS_UPDATE_INTERVAL_MS));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let snap = stats.snapshot();
                        let _ = stats.updates.send(snap);
                    }
                    _ = tick_cancel.cancelled() => {
                        let snap = stats.snapshot();
                        let _ = stats.updates.send(snap);
                        break;
                    }
                }
            }
        });

        match self.cfg.mode {
            Mode::Users => self.run_users(cancel).await,
            Mode::Rps => self.run_rps(cancel).await,
        }

        tick_handle.abort();
    }

    /// Users mode (closed loop): spawn N virtual users, each looping.
    async fn run_users(&self, cancel: CancellationToken) {
        let total_dur = self.cfg.total_duration();
        let start = Instant::now();

        let spawn_interval = if self.cfg.ramp_up_secs > 0 && self.cfg.num_users > 1 {
            Duration::from_secs_f64(
                self.cfg.ramp_up_secs as f64 / self.cfg.num_users as f64,
            )
        } else {
            Duration::ZERO
        };

        let mut handles = Vec::new();
        let cfg = Arc::new(self.cfg.clone());
        let client = self.client.clone();
        let engine = Arc::clone(&self.template_engine);
        let stats = Arc::clone(&self.stats);

        for i in 0..self.cfg.num_users {
            if cancel.is_cancelled() {
                break;
            }

            if i > 0 && !spawn_interval.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(spawn_interval) => {}
                    _ = cancel.cancelled() => break,
                }
            }

            let user_id = Uuid::new_v4().to_string();
            let user_cancel = cancel.clone();
            let user_start = start;
            let user_cfg = Arc::clone(&cfg);
            let user_client = client.clone();
            let user_engine = Arc::clone(&engine);
            let user_stats = Arc::clone(&stats);

            handles.push(tokio::spawn(async move {
                loop {
                    if user_cancel.is_cancelled() {
                        break;
                    }
                    if user_start.elapsed() > total_dur {
                        break;
                    }

                    let scheduled = Instant::now();
                    let req_id = Uuid::new_v4().to_string();
                    let req_ctx = TemplateContext::new(user_id.clone(), req_id);

                    if user_cfg.command.is_some() {
                        execute_script(&user_cfg, &user_engine, &req_ctx, scheduled, &user_stats).await;
                    } else {
                        execute_http(&user_client, &user_cfg, &user_engine, &req_ctx, scheduled, &user_stats).await;
                    }

                    if user_cfg.think_time_ms > 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(user_cfg.think_time_ms)) => {}
                            _ = user_cancel.cancelled() => break,
                        }
                    }
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }
    }

    /// RPS mode (open loop): time-based request scheduling.
    async fn run_rps(&self, cancel: CancellationToken) {
        let total_dur = self.cfg.total_duration();
        let start = Instant::now();

        let mut next_request_time = start;
        let mut handles = Vec::new();
        let drain_flag = Arc::new(AtomicBool::new(false));

        let cfg = Arc::new(self.cfg.clone());
        let client = self.client.clone();
        let engine = Arc::clone(&self.template_engine);
        let stats = Arc::clone(&self.stats);

        loop {
            if cancel.is_cancelled() {
                drain_flag.store(true, Ordering::SeqCst);
                break;
            }

            let elapsed = start.elapsed().as_secs_f64();

            if elapsed >= total_dur.as_secs_f64() {
                drain_flag.store(true, Ordering::SeqCst);
                break;
            }

            let target_rps = current_rps(&cfg, elapsed);

            if target_rps <= 0.001 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                next_request_time = Instant::now();
                continue;
            }

            let period = Duration::from_secs_f64(1.0 / target_rps);
            let now = Instant::now();

            // Catch-up protection
            if now.saturating_duration_since(next_request_time) > Duration::from_secs(1) {
                next_request_time = now;
            }

            // Spawn requests while behind schedule
            while next_request_time <= now || next_request_time == now {
                let scheduled = next_request_time;
                let req_id = Uuid::new_v4().to_string();
                let user_id = Uuid::new_v4().to_string();
                let ctx = TemplateContext::new(user_id, req_id);
                let r_cfg = Arc::clone(&cfg);
                let r_client = client.clone();
                let r_engine = Arc::clone(&engine);
                let r_stats = Arc::clone(&stats);
                let sem = Arc::clone(&self.semaphore);

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire_owned().await.ok();
                    if r_cfg.command.is_some() {
                        execute_script(&r_cfg, &r_engine, &ctx, scheduled, &r_stats).await;
                    } else {
                        execute_http(&r_client, &r_cfg, &r_engine, &ctx, scheduled, &r_stats).await;
                    }
                }));

                next_request_time = next_request_time + period;
            }

            let sleep_dur = next_request_time.saturating_duration_since(Instant::now());
            if !sleep_dur.is_zero() {
                tokio::time::sleep(sleep_dur).await;
            }
        }

        // Drain: wait for all in-flight requests
        for h in handles {
            let _ = h.await;
        }

        // Final stats update
        let snap = self.stats.snapshot();
        let _ = self.stats.updates.send(snap);
    }
}
