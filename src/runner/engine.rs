use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::core::config::{Config, Mode};
use crate::core::constants::STATS_UPDATE_INTERVAL_MS;
use crate::runner::client::build_client;
use crate::runner::executor::{execute_http, execute_script};
use crate::runner::ramp::current_rps;
use crate::runner::request::PreparedRequest;
use crate::runner::stats::{InflightGuard, RunStats};
use crate::templates::{TemplateContext, TemplateEngine};

/// Interval used while waiting for in-flight work to drain.
const DRAIN_POLL_INTERVAL_MS: u64 = 20;

/// How far the open-loop schedule may lag before it resynchronises.
///
/// Beyond this the generator is not keeping up with the requested rate, and
/// catching up by replaying the backlog would queue work the schedule already
/// committed to sending.
const MAX_SCHEDULE_SLIP: Duration = Duration::from_secs(1);

/// The main load generation engine.
///
/// Owns scheduling only. Request execution lives in [`crate::runner::executor`]
/// and the reusable request plan in [`PreparedRequest`].
pub struct LoadEngine {
    /// Shared so a request task clones an `Arc`, not the headers and auth
    /// tokens inside a `Config`.
    cfg: Arc<Config>,
    client: reqwest::Client,
    prepared: PreparedRequest,
    template_engine: Arc<TemplateEngine>,
    stats: Arc<RunStats>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl LoadEngine {
    /// Create a new load engine bound to a stats channel.
    pub fn new(
        cfg: Config,
        updates: tokio::sync::mpsc::Sender<crate::core::snapshot::StatsSnapshot>,
    ) -> anyhow::Result<Self> {
        // Validation lives here, not in the CLI, so that a library embedder
        // cannot bypass it. Without this call every bound in `Config` is inert
        // and an out-of-range `num_users` reaches `run_users` unbounded.
        if let Err(errors) = cfg.validate() {
            anyhow::bail!("invalid configuration:\n  - {}", errors.join("\n  - "));
        }
        let client = build_client(&cfg)?;
        let permits = Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrency as usize));

        Ok(Self {
            prepared: PreparedRequest::new(&cfg, &TemplateEngine::new())?,
            client,
            template_engine: Arc::new(TemplateEngine::new()),
            stats: Arc::new(RunStats::new(updates)),
            permits,
            cfg: Arc::new(cfg),
        })
    }

    /// Shared stats handle for the UI and post-run reporting.
    pub fn stats(&self) -> &Arc<RunStats> {
        &self.stats
    }

    /// Generate load until the run duration elapses or `cancel` fires.
    pub async fn run(&self, cancel: CancellationToken) {
        let ticker = self.spawn_stats_ticker(cancel.clone());

        match self.cfg.mode {
            Mode::Users => self.run_users(cancel.clone()).await,
            Mode::Rps => self.run_rps(cancel.clone()).await,
        }

        ticker.abort();
    }

    /// Emit a stats snapshot on a fixed cadence for the lifetime of the run.
    fn spawn_stats_ticker(&self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        let stats = Arc::clone(&self.stats);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(STATS_UPDATE_INTERVAL_MS));
            loop {
                tokio::select! {
                    _ = interval.tick() => stats.publish_snapshot(),
                    _ = cancel.cancelled() => {
                        stats.publish_snapshot();
                        break;
                    }
                }
            }
        })
    }

    /// Closed-loop mode: a fixed pool of virtual users, each looping.
    async fn run_users(&self, cancel: CancellationToken) {
        let total_duration = self.cfg.total_duration();
        let start = Instant::now();
        let spawn_interval = self.user_spawn_interval();

        let mut handles = Vec::with_capacity(self.cfg.num_users.min(1_024) as usize);
        for index in 0..self.cfg.num_users {
            if cancel.is_cancelled() {
                break;
            }
            if index > 0 && !spawn_interval.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(spawn_interval) => {}
                    _ = cancel.cancelled() => break,
                }
            }
            handles.push(self.spawn_virtual_user(cancel.clone(), start, total_duration));
        }

        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Delay between virtual user spawns during ramp-up.
    fn user_spawn_interval(&self) -> Duration {
        if self.cfg.ramp_up_secs > 0 && self.cfg.num_users > 1 {
            Duration::from_secs_f64(self.cfg.ramp_up_secs as f64 / self.cfg.num_users as f64)
        } else {
            Duration::ZERO
        }
    }

    /// Spawn one looping virtual user.
    fn spawn_virtual_user(
        &self,
        cancel: CancellationToken,
        start: Instant,
        total_duration: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let cfg = Arc::clone(&self.cfg);
        let client = self.client.clone();
        let engine = Arc::clone(&self.template_engine);
        let stats = Arc::clone(&self.stats);
        let prepared = self.prepared.clone();
        let user_id = Uuid::new_v4().to_string();

        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() || start.elapsed() > total_duration {
                    break;
                }

                let scheduled = Instant::now();
                let ctx = TemplateContext::new(user_id.clone());
                let _admitted = InflightGuard::admit(&stats);

                if cfg.command.is_some() {
                    execute_script(&cfg, &engine, &ctx, scheduled, &stats).await;
                } else {
                    execute_http(&client, &prepared, &engine, &ctx, scheduled, &stats).await;
                }

                if cfg.think_time_ms > 0 {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(cfg.think_time_ms)) => {}
                        _ = cancel.cancelled() => break,
                    }
                }
            }
        })
    }

    /// Open-loop mode: schedule requests against a wall-clock timeline.
    ///
    /// Concurrency is capped by a semaphore permit acquired *before* spawning.
    /// When no permit is available the scheduled request is dropped and counted
    /// rather than queued, which keeps resident memory bounded and keeps the
    /// open-loop rate honest. A non-zero drop count is the signal that the
    /// generator, not the target, was the bottleneck.
    async fn run_rps(&self, cancel: CancellationToken) {
        let total_duration = self.cfg.total_duration();
        let start = Instant::now();
        let mut next_request_time = start;

        loop {
            if cancel.is_cancelled() || start.elapsed() >= total_duration {
                break;
            }

            let target_rps = current_rps(&self.cfg, start.elapsed().as_secs_f64());
            if target_rps <= 0.001 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                next_request_time = Instant::now();
                continue;
            }

            let period = Duration::from_secs_f64(1.0 / target_rps);
            self.absorb_schedule_slip(&mut next_request_time, period);
            self.dispatch_due_requests(&mut next_request_time, period, &cancel);
            sleep_until(next_request_time).await;
        }

        self.await_drain().await;
        self.stats.publish_snapshot();
    }

    /// Resynchronise the schedule after the generator falls behind.
    ///
    /// A backlog is never replayed: replaying it would queue requests the open
    /// loop already committed to sending, converting a generator limit into
    /// apparent target latency. Every slot skipped by the resynchronisation is
    /// counted as a shed request so the run reports what it actually did
    /// instead of silently under-reporting its request count.
    fn absorb_schedule_slip(&self, next_request_time: &mut Instant, period: Duration) {
        let now = Instant::now();
        let behind = now.saturating_duration_since(*next_request_time);
        if behind <= MAX_SCHEDULE_SLIP {
            return;
        }

        let period_ns = period.max(Duration::from_nanos(1)).as_nanos();
        let skipped = (behind.as_nanos() / period_ns).min(u64::MAX as u128) as u64;
        self.stats.record_scheduled_drops(skipped);
        *next_request_time = now;
    }

    /// Spawn every request whose scheduled time has arrived.
    fn dispatch_due_requests(
        &self,
        next_request_time: &mut Instant,
        period: Duration,
        cancel: &CancellationToken,
    ) {
        let now = Instant::now();
        while *next_request_time <= now {
            if cancel.is_cancelled() {
                return;
            }
            match self.permits.clone().try_acquire_owned() {
                Ok(permit) => {
                    self.spawn_scheduled(*next_request_time, permit);
                }
                Err(_) => self.stats.record_scheduled_drop(),
            }
            *next_request_time += period;
        }
    }

    /// Spawn one open-loop request holding an owned concurrency permit.
    fn spawn_scheduled(&self, scheduled: Instant, permit: tokio::sync::OwnedSemaphorePermit) {
        let client = self.client.clone();
        let engine = Arc::clone(&self.template_engine);
        let stats = Arc::clone(&self.stats);
        let prepared = self.prepared.clone();
        let cfg = Arc::clone(&self.cfg);
        let ctx = TemplateContext::new(Uuid::new_v4().to_string());

        // Admitted here, at dispatch, strictly BEFORE spawning. Admitting
        // inside the task body would make a spawned-but-not-yet-polled task
        // invisible to the drain barrier, and the run would abandon it.
        let admitted = InflightGuard::admit(&stats);

        tokio::spawn(async move {
            let _permit = permit;
            let _admitted = admitted;
            if cfg.command.is_some() {
                execute_script(&cfg, &engine, &ctx, scheduled, &stats).await;
            } else {
                execute_http(&client, &prepared, &engine, &ctx, scheduled, &stats).await;
            }
        });
    }

    /// Wait for all in-flight requests to finish after the schedule ends.
    async fn await_drain(&self) {
        while self.stats.inflight_count() > 0 {
            tokio::time::sleep(Duration::from_millis(DRAIN_POLL_INTERVAL_MS)).await;
        }
    }
}

/// Sleep until `deadline`, yielding immediately if it has already passed.
async fn sleep_until(deadline: Instant) {
    let wait = deadline.saturating_duration_since(Instant::now());
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}
