use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Sparkline};
use ratatui::Frame;

use crate::theme::Theme;
use crate::views::helpers::{
    render_metric_card, status_badge_text, vertical_chunks, horizontal_chunks,
};
use rustress_core::snapshot::StatsSnapshot;
use std::time::{Duration, Instant};

/// Phase labels for the ramp profile.
fn phase_label(elapsed: Duration, ramp_up: u64, steady: u64, ramp_down: u64) -> &'static str {
    let secs = elapsed.as_secs_f64();
    if secs < ramp_up as f64 {
        "Ramp Up"
    } else if secs < (ramp_up + steady) as f64 {
        "Steady State"
    } else if secs < (ramp_up + steady + ramp_down) as f64 {
        "Ramp Down"
    } else {
        "Complete"
    }
}

fn progress_pct(elapsed: Duration, total: Duration) -> f64 {
    if total.is_zero() {
        return 0.0;
    }
    (elapsed.as_secs_f64() / total.as_secs_f64() * 100.0).min(100.0)
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Latency history for sparkline.
#[derive(Debug, Clone)]
pub struct LatencyHistory {
    pub p50: Vec<u64>,
    pub p90: Vec<u64>,
    pub p99: Vec<u64>,
    pub rps: Vec<u64>,
    pub max_len: usize,
}

impl LatencyHistory {
    pub fn new(max_len: usize) -> Self {
        Self {
            p50: Vec::with_capacity(max_len),
            p90: Vec::with_capacity(max_len),
            p99: Vec::with_capacity(max_len),
            rps: Vec::with_capacity(max_len),
            max_len,
        }
    }

    pub fn push(&mut self, snap: &StatsSnapshot, elapsed_secs: f64) {
        // Sample every ~1 second
        if elapsed_secs < 1.0 {
            return;
        }
        let rps = if elapsed_secs > 0.0 {
            (snap.requests as f64 / elapsed_secs) as u64
        } else {
            0
        };
        self.p50.push(snap.p50_service_ms as u64);
        self.p90.push(snap.p90_service_ms as u64);
        self.p99.push(snap.p99_service_ms as u64);
        self.rps.push(rps);

        // Trim old entries
        while self.p50.len() > self.max_len {
            self.p50.remove(0);
        }
        while self.p90.len() > self.max_len {
            self.p90.remove(0);
        }
        while self.p99.len() > self.max_len {
            self.p99.remove(0);
        }
        while self.rps.len() > self.max_len {
            self.rps.remove(0);
        }
    }
}

/// Dashboard view state.
pub struct DashboardView {
    pub stats: StatsSnapshot,
    pub start_time: Instant,
    pub last_sample_time: Instant,
    pub duration: Duration,
    pub ramp_up: u64,
    pub steady: u64,
    pub ramp_down: u64,
    pub target_value: f64,
    pub mode: String,
    pub status: String,
    pub latency_history: LatencyHistory,
    pub width: u16,
    pub height: u16,
}

impl DashboardView {
    pub fn new(ramp_up: u64, steady: u64, ramp_down: u64, target_rps: u32, mode: String) -> Self {
        Self {
            stats: StatsSnapshot::default(),
            start_time: Instant::now(),
            last_sample_time: Instant::now(),
            duration: Duration::from_secs(ramp_up + steady + ramp_down),
            ramp_up,
            steady,
            ramp_down,
            target_value: target_rps as f64,
            mode,
            status: "RUNNING".to_string(),
            latency_history: LatencyHistory::new(60),
            width: 80,
            height: 24,
        }
    }

    pub fn update_stats(&mut self, snap: StatsSnapshot) {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let last_sample = self.last_sample_time.elapsed().as_secs_f64();
        if last_sample >= 1.0 {
            self.latency_history.push(&snap, elapsed);
            self.last_sample_time = Instant::now();
        }
        self.stats = snap;
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        if self.width < 60 || self.height < 16 {
            let block = Block::default()
                .title(" Dashboard ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.surface));
            frame.render_widget(
                Paragraph::new("Terminal too small — resize to at least 60×16")
                    .style(Style::default().fg(theme.text))
                    .block(block),
                area,
            );
            return;
        }

        let elapsed = self.start_time.elapsed();
        let pct = progress_pct(elapsed, self.duration);
        let phase = phase_label(elapsed, self.ramp_up, self.steady, self.ramp_down);
        let s = &self.stats;
        let avg_rps = if elapsed.as_secs_f64() > 0.0 {
            s.requests as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        // Layout: top status bar, metric cards, charts, bottom
        let chunks = vertical_chunks(
            area,
            vec![
                Constraint::Length(1),  // Status bar
                Constraint::Length(3),  // Metric cards row 1
                Constraint::Length(3),  // Metric cards row 2
                Constraint::Min(6),     // Sparkline charts
                Constraint::Length(3),  // Progress gauge + status codes
            ],
        );

        // ── Status bar ──
        let (badge_text, badge_style) = status_badge_text(&self.status);
        let status_line = Line::from(vec![
            Span::styled(badge_text, badge_style),
            Span::raw("  "),
            Span::styled(phase, Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)),
            Span::raw("  │  "),
            Span::raw(format!("Elapsed: {}", format_duration(elapsed))),
            Span::raw("  │  "),
            Span::raw(format!("Mode: {}", self.mode.to_uppercase())),
            Span::raw("  │  "),
            Span::raw(format!("Target: {:.0} {}", self.target_value, if self.mode == "rps" { "RPS" } else { "Users" })),
        ]);
        frame.render_widget(
            Paragraph::new(status_line).style(Style::default().bg(theme.surface)),
            chunks[0],
        );

        // ── Metric cards row 1 ──
        let row1 = horizontal_chunks(
            chunks[1],
            vec![
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ],
        );
        render_metric_card(frame, row1[0], "Requests", &s.requests.to_string(), theme);
        render_metric_card(frame, row1[1], "Avg RPS", &format!("{:.1}", avg_rps), theme);
        render_metric_card(frame, row1[2], "Inflight", &s.inflight.to_string(), theme);
        let success_rate = if s.requests > 0 {
            format!("{:.1}%", s.success as f64 / s.requests as f64 * 100.0)
        } else {
            "—".to_string()
        };
        let sr_style = if s.success as f64 / (s.requests.max(1)) as f64 >= 0.95 {
            Style::default().fg(theme.success).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.warning).add_modifier(Modifier::BOLD)
        };
        render_success_card(frame, row1[3], "Success", &success_rate, sr_style, theme);
        render_metric_card(frame, row1[4], "Errors", &s.fail.to_string(), theme);

        // ── Metric cards row 2 (latencies) ──
        let row2 = horizontal_chunks(
            chunks[2],
            vec![
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ],
        );
        render_latency_card(frame, row2[0], "P50", s.p50_service_ms, theme);
        render_latency_card(frame, row2[1], "P90", s.p90_service_ms, theme);
        render_latency_card(frame, row2[2], "P95", s.p95_service_ms, theme);
        render_latency_card(frame, row2[3], "P99", s.p99_service_ms, theme);
        render_latency_card(frame, row2[4], "Max", s.max_service_ms, theme);

        // ── Sparkline charts ──
        let chart_chunks = horizontal_chunks(
            chunks[3],
            vec![Constraint::Percentage(50), Constraint::Percentage(50)],
        );
        render_latency_sparkline(frame, chart_chunks[0], &self.latency_history, theme);
        render_rps_sparkline(frame, chart_chunks[1], &self.latency_history, self.target_value as u64, theme);

        // ── Bottom row: progress gauge + status codes ──
        let bottom_chunks = horizontal_chunks(
            chunks[4],
            vec![Constraint::Percentage(40), Constraint::Percentage(60)],
        );
        render_progress_gauge(frame, bottom_chunks[0], pct, elapsed, &self.duration, phase, theme);
        render_status_codes(frame, bottom_chunks[1], s, theme);
    }
}

fn render_success_card(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str, style: Style, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));
    frame.render_widget(Paragraph::new(value).style(style).block(block), area);
}

fn render_latency_card(frame: &mut Frame<'_>, area: Rect, title: &str, ms: f64, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(theme.warning).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));
    let val = format!("{:.0}ms", ms.round());
    let style = if ms < 100.0 {
        Style::default().fg(theme.success).add_modifier(Modifier::BOLD)
    } else if ms < 500.0 {
        Style::default().fg(theme.warning).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.error).add_modifier(Modifier::BOLD)
    };
    frame.render_widget(Paragraph::new(val).style(style).block(block), area);
}

fn render_latency_sparkline(frame: &mut Frame<'_>, area: Rect, history: &LatencyHistory, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(" Latency (P50/P90/P99 ms) ", theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));

    let max_latency = history.p99.iter().copied().max().unwrap_or(100);
    let spark = Sparkline::default()
        .data(&history.p99)
        .max(max_latency)
        .style(Style::default().fg(theme.primary))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);

    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    if inner.height > 0 {
        frame.render_widget(spark, inner);
    }
}

fn render_rps_sparkline(frame: &mut Frame<'_>, area: Rect, history: &LatencyHistory, target: u64, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(" Requests/sec ", theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));

    let max_rps = history.rps.iter().copied().max().unwrap_or(target.max(1));
    let spark = Sparkline::default()
        .data(&history.rps)
        .max(max_rps)
        .style(Style::default().fg(theme.secondary))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);

    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);
    if inner.height > 0 {
        frame.render_widget(spark, inner);
    }
}

fn render_progress_gauge(frame: &mut Frame<'_>, area: Rect, pct: f64, elapsed: Duration, total: &Duration, phase: &str, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(" Progress ", theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));

    let gauge = Gauge::default()
        .block(block)
        .gauge_style(Style::default().fg(theme.progress_bar))
        .percent(pct as u16)
        .label(format!("{:.0}%  {} / {}  ({})", pct, format_duration(elapsed), format_duration(*total), phase));

    frame.render_widget(gauge, area);
}

fn render_status_codes(frame: &mut Frame<'_>, area: Rect, s: &StatsSnapshot, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(" Response Codes ", theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));

    let total = s.requests.max(1);
    let mut lines = vec![];
    let mut codes: Vec<_> = s.status_codes.iter().collect();
    codes.sort_by_key(|(code, _)| **code);

    for (code, count) in &codes {
        let pct = (**count as f64) / (total as f64) * 100.0;
        let bar_len = (pct / 2.0).round() as usize;
        let bar = "█".repeat(bar_len.min(40));
        let code_style = if **code >= 200 && **code < 300 {
            Style::default().fg(theme.success)
        } else if **code >= 400 {
            Style::default().fg(theme.error)
        } else {
            Style::default().fg(theme.warning)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:>3} ", code), code_style),
            Span::styled(format!("{:>5} ({:>5.1}%) ", count, pct), Style::default().fg(theme.text)),
            Span::styled(bar, Style::default().fg(theme.secondary)),
        ]));
    }

    if codes.is_empty() {
        lines.push(Line::from(Span::styled("  (waiting for responses...)", theme.subtle_style())));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
