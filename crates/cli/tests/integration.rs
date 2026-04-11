use rustress_core::config::{Config, Mode};
use rustress_core::constants::VERSION;

#[test]
fn test_version_not_empty() {
    assert!(!VERSION.is_empty());
    assert!(VERSION.chars().next().unwrap().is_ascii_digit());
}

#[test]
fn test_config_mode_serialization() {
    let cfg = Config {
        url: "http://test".into(),
        mode: Mode::Users,
        num_users: 50,
        steady_dur_secs: 10,
        ..Default::default()
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let parsed: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.mode, Mode::Users);
    assert_eq!(parsed.num_users, 50);
}

#[test]
fn test_experiment_result_serialization() {
    use rustress_core::result::ExperimentResult;
    use chrono::Utc;
    use std::time::Duration;

    let r = ExperimentResult {
        timestamp: Utc::now(),
        latency: Duration::from_millis(100),
        service_time: Duration::from_millis(90),
        queue_wait: Duration::from_millis(10),
        status: 200,
        success: true,
        bytes: 1024,
        user_id: "user-1".into(),
        query: "test".into(),
        error: None,
        response_body: None,
    };

    let json = serde_json::to_string(&r).unwrap();
    let parsed: ExperimentResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, 200);
    assert!(parsed.success);
}

#[test]
fn test_stats_snapshot_default() {
    use rustress_core::snapshot::StatsSnapshot;
    let snap = StatsSnapshot::default();
    assert_eq!(snap.requests, 0);
    assert!(snap.status_codes.is_empty());
}

#[test]
fn test_metrics_histogram_record_and_query() {
    use rustress_metrics::LatencyHistogram;
    use rustress_metrics::PercentileExt;

    let hist = LatencyHistogram::new();
    for _ in 0..1000 {
        hist.record(500);
    }
    for _ in 0..1000 {
        hist.record(1000);
    }

    // p50 should be between 0.5ms and 1.0ms (equal split)
    assert!(hist.p50_ms() >= 0.4 && hist.p50_ms() <= 1.2, "p50 was {}", hist.p50_ms());
    // p99 should be around 1ms
    assert!(hist.p99_ms() >= 0.5, "p99 was {}", hist.p99_ms());
    assert_eq!(hist.len(), 2000);
}

#[test]
fn test_stats_collector_thread_safety() {
    use rustress_metrics::StatsCollector;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    let collector: Arc<StatsCollector> = Arc::new(StatsCollector::new());
    let mut handles = vec![];

    for _ in 0..50 {
        let c: Arc<StatsCollector> = Arc::clone(&collector);
        handles.push(thread::spawn(move || {
            for _ in 0..200 {
                c.add(
                    true,
                    64,
                    Duration::from_micros(500),
                    Duration::ZERO,
                    Duration::from_micros(500),
                    200,
                    None,
                    None,
                );
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let snap = collector.snapshot();
    assert_eq!(snap.requests, 10_000);
    assert_eq!(snap.success, 10_000);
    assert_eq!(snap.fail, 0);
}

#[test]
fn test_template_engine_basic() {
    use rustress_templates::TemplateContext;
    use rustress_templates::TemplateEngine;

    let engine = TemplateEngine::new();
    let tpl = engine.parse("greet", "Hello {{ user_id }}!").unwrap();
    let ctx = TemplateContext::new("alice".into(), "uuid-1".into());
    let result = engine.execute(&tpl, &ctx).unwrap();
    assert_eq!(result, "Hello alice!");
}

#[test]
fn test_template_uuid_generation() {
    use rustress_templates::TemplateContext;
    use rustress_templates::TemplateEngine;

    let engine = TemplateEngine::new();
    let ctx = TemplateContext::default();

    let mut uuids = std::collections::HashSet::new();
    for _ in 0..100 {
        let result = engine.execute_str("{{ uuid() }}", &ctx).unwrap();
        uuids.insert(result);
    }
    assert_eq!(uuids.len(), 100);
}

#[test]
fn test_ramp_profile_accuracy() {
    use rustress_runner::ramp::current_rps;

    let cfg = Config {
        url: "http://test".into(),
        target_rps: 1000,
        ramp_up_secs: 10,
        steady_dur_secs: 30,
        ramp_down_secs: 10,
        ..Default::default()
    };

    let eps = 5.0;
    assert!((current_rps(&cfg, 5.0) - 500.0).abs() < eps);
    assert!((current_rps(&cfg, 10.0) - 1000.0).abs() < eps);
    assert!((current_rps(&cfg, 25.0) - 1000.0).abs() < eps);
    assert!((current_rps(&cfg, 45.0) - 500.0).abs() < eps);
    assert_eq!(current_rps(&cfg, 50.0), 0.0);
}

#[test]
fn test_runner_view_navigation() {
    use rustress_core::config::Config;
    use rustress_tui::views::runner::RunnerView;

    let cfg = Config {
        url: "http://test".into(),
        ..Default::default()
    };
    let mut view = RunnerView::new(cfg);

    view.focus_next();
    assert_eq!(view.focused_field, 1);

    view.toggle_load_mode();
    assert_eq!(view.load_mode, "users");

    let config = view.get_config();
    assert_eq!(config.mode, Mode::Users);
}

#[test]
fn test_theme_dark_and_light() {
    use rustress_tui::Theme;

    let dark = Theme::dark();
    let light = Theme::light();

    assert!(dark.bg != light.bg);
    assert!(dark.text != light.text);
}

#[test]
fn test_cli_config_from_flags() {
    use rustress_core::config::{Config, Mode};
    use clap::Parser;

    #[derive(clap::Parser, Debug)]
    struct TestCli {
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "GET")]
        method: String,
        #[arg(short, long)]
        rate: Option<u32>,
        #[arg(short, long)]
        users: Option<u32>,
        #[arg(short, long)]
        duration: Option<u64>,
        #[arg(short = 'H', long, action = clap::ArgAction::Append)]
        header: Vec<String>,
    }

    impl TestCli {
        fn into_config(&self) -> Config {
            let mut cfg = Config::default();
            if let Some(ref url) = self.url { cfg.url = url.clone(); }
            cfg.method = self.method.clone();
            if let Some(r) = self.rate { cfg.target_rps = r; }
            if let Some(u) = self.users { cfg.num_users = u; cfg.mode = Mode::Users; }
            if let Some(d) = self.duration { cfg.steady_dur_secs = d; }
            for h in &self.header {
                if let Some((k, v)) = h.split_once(':') {
                    cfg.headers.insert(k.trim().into(), v.trim().into());
                }
            }
            cfg
        }
    }

    let cli = TestCli::try_parse_from([
        "test",
        "--url", "http://example.com",
        "-r", "500",
        "-d", "60",
        "--method", "POST",
        "-H", "Authorization: Bearer token",
    ]).unwrap();

    let cfg = cli.into_config();
    assert_eq!(cfg.url, "http://example.com");
    assert_eq!(cfg.target_rps, 500);
    assert_eq!(cfg.steady_dur_secs, 60);
    assert_eq!(cfg.method, "POST");
    assert_eq!(cfg.headers.get("Authorization"), Some(&"Bearer token".to_string()));
}
