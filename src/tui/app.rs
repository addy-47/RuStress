use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tokio_util::sync::CancellationToken;

use super::EventLoop;
use super::TerminalGuard;
use super::Theme;
use crate::core::config::Config;
use crate::core::snapshot::StatsSnapshot;
use crate::runner::RunController;
use crate::tui::views::helpers::vertical_chunks;
use crate::tui::views::{DashboardView, RunnerView};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppView {
    Runner,
    Dashboard,
}

pub struct App {
    pub view: AppView,
    pub runner_view: RunnerView,
    pub dash_view: DashboardView,
    pub theme: Theme,
    pub status_msg: Option<String>,
    pub run_active: bool,
    pub draining: bool,
    pub cancel_token: Option<CancellationToken>,
    /// Owns the actual load run. The dashboard used to render "RUNNING" with
    /// no traffic behind it because the engine was constructed and dropped.
    pub controller: RunController,
    pub width: u16,
    pub height: u16,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        let theme = Theme::default();
        let runner_view = RunnerView::new(cfg.clone());
        let dash_view = DashboardView::new(
            cfg.ramp_up_secs,
            cfg.steady_dur_secs,
            cfg.ramp_down_secs,
            cfg.target_rps,
            format!("{:?}", cfg.mode).to_lowercase(),
        );
        Self {
            view: AppView::Runner,
            runner_view,
            dash_view,
            theme,
            status_msg: None,
            run_active: false,
            draining: false,
            cancel_token: None,
            controller: RunController::new(),
            width: 80,
            height: 24,
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Global keys (work on any view)
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('c')) | (_, KeyCode::Char('q'))
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                return true;
            }
            (_, KeyCode::Char('s'))
                if key.modifiers.contains(KeyModifiers::CONTROL) && self.run_active =>
            {
                self.stop_run();
                return false;
            }
            // Tab switching
            (_, KeyCode::Char('1')) => {
                self.view = AppView::Runner;
                return false;
            }
            (_, KeyCode::Char('2')) => {
                self.view = AppView::Dashboard;
                return false;
            }
            (_, KeyCode::Tab) if self.view == AppView::Dashboard => {
                self.view = AppView::Runner;
                return false;
            }
            (_, KeyCode::BackTab) if self.view == AppView::Dashboard => {
                self.view = AppView::Runner;
                return false;
            }
            (KeyModifiers::SHIFT, KeyCode::Left) | (KeyModifiers::SHIFT, KeyCode::Right) => {
                self.view = if self.view == AppView::Runner {
                    AppView::Dashboard
                } else {
                    AppView::Runner
                };
                return false;
            }
            _ => {}
        }

        match self.view {
            AppView::Runner => {
                // Ctrl+R starts a real run. The config comes from the runner
                // form, so a bad value there is reported in the status line
                // rather than taking the process down mid-render.
                if let (KeyModifiers::CONTROL, KeyCode::Char('r')) = (key.modifiers, key.code) {
                    match self.controller.start(self.runner_view.get_config()) {
                        Ok(()) => {
                            self.run_active = true;
                            self.draining = false;
                            self.cancel_token = Some(CancellationToken::new());
                            self.dash_view.status = "RUNNING".to_string();
                            self.dash_view.start_time = std::time::Instant::now();
                            self.status_msg = None;
                            self.view = AppView::Dashboard;
                        }
                        Err(errors) => {
                            self.status_msg = Some(format!("cannot start: {}", errors.join("; ")));
                        }
                    }
                    return false;
                }
                // All other keys go to input handler
                self.runner_view.handle_input(key)
            }
            AppView::Dashboard => {
                // +/- Real-time adjustment (Draft logic, will integrate with engine next)
                match key.code {
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        self.dash_view.target_value += 10.0;
                        false
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        self.dash_view.target_value = (self.dash_view.target_value - 10.0).max(0.0);
                        false
                    }
                    _ => false,
                }
            }
        }
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        if self.view == AppView::Runner {
            // Offset for header
            let mut m = mouse;
            if m.row > 0 {
                m.row -= 1;
                self.runner_view.handle_mouse(m);
            }
        }
    }

    pub fn handle_stats(&mut self, snap: StatsSnapshot) {
        self.dash_view.update_stats(snap);

        if self.run_active && !self.draining {
            let elapsed = self.dash_view.start_time.elapsed();
            if elapsed >= self.dash_view.duration {
                self.draining = true;
                if let Some(cancel) = self.cancel_token.take() {
                    cancel.cancel();
                }
                self.dash_view.status = String::from("DRAINING");
                self.status_msg = Some("Draining: waiting for in-flight requests...".to_string());
            }
        }

        // FINISHED is decided by the run task completing, not by an in-flight
        // count reaching zero. `inflight` is admitted at dispatch, so it can
        // read 0 during a ramp before any request is admitted, which would
        // report a finished run that has not started.
        if self.draining && !self.controller.is_running() {
            self.run_active = false;
            self.draining = false;
            self.dash_view.status = String::from("FINISHED");
            self.status_msg = Some("Test completed.".to_string());
        }
    }

    pub fn stop_run(&mut self) {
        if let Some(cancel) = self.cancel_token.take() {
            cancel.cancel();
        }
        self.controller.stop();
        self.draining = true;
        self.dash_view.status = String::from("DRAINING");
        self.status_msg = Some("Stopping...".to_string());
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        self.width = area.width;
        self.height = area.height;
        self.runner_view.resize(area.width, area.height);
        self.dash_view.resize(area.width, area.height);

        let active_tab = match self.view {
            AppView::Runner => "Config",
            AppView::Dashboard => "Dashboard",
        };

        let chunks = vertical_chunks(
            area,
            vec![
                Constraint::Length(3), // Header
                Constraint::Min(10),   // Content
                Constraint::Length(3), // Help bar
            ],
        );

        render_header(frame, chunks[0], &self.theme, active_tab);

        match self.view {
            AppView::Runner => {
                self.runner_view.render(frame, chunks[1], &self.theme);
            }
            AppView::Dashboard => {
                self.dash_view.render(frame, chunks[1], &self.theme);
            }
        }

        render_help_bar(frame, chunks[2], &self.theme, active_tab);

        if let Some(ref msg) = self.status_msg {
            render_status_overlay(frame, area, msg, &self.theme);
        }
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, theme: &Theme, active_tab: &str) {
    let tabs = ["Config", "Dashboard"];
    let tab_spans: Vec<Span> = tabs
        .iter()
        .enumerate()
        .flat_map(|(i, tab)| {
            let (style, prefix) = if *tab == active_tab {
                (theme.tab_active_style(), " ● ")
            } else {
                (theme.tab_inactive_style(), " ○ ")
            };
            if i > 0 {
                vec![
                    Span::raw("  "),
                    Span::styled(prefix, style),
                    Span::styled(format!("{} ", tab), style),
                ]
            } else {
                vec![
                    Span::styled(prefix, style),
                    Span::styled(format!("{} ", tab), style),
                ]
            }
        })
        .collect();

    let left = Span::styled(
        " STEADYQ ",
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    );
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    let mut spans = vec![left, Span::raw("  ")];
    spans.extend(tab_spans);
    let para = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(para, area);
}

fn render_help_bar(frame: &mut Frame<'_>, area: Rect, theme: &Theme, active_tab: &str) {
    let bindings: Vec<(&str, &str)> = if active_tab == "Config" {
        vec![
            ("Tab", "Next"),
            ("Space", "Mode"),
            ("Shift+Arrow", "Tab"),
            ("Ctrl+R", "Run"),
            ("Esc", "Exit"),
        ]
    } else {
        vec![
            ("Shift+Arrow", "Tab"),
            ("Ctrl+S", "Stop"),
            ("+/-", "Adj."),
            ("Esc", "Exit"),
        ]
    };

    let spans: Vec<Span> = bindings
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(
                    format!(" {} ", key),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {} ", desc), Style::default().fg(theme.text)),
                Span::raw("   "),
            ]
        })
        .collect();

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    let para = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(para, area);
}

fn render_status_overlay(frame: &mut Frame<'_>, area: Rect, msg: &str, theme: &Theme) {
    let block_width = msg.len() as u16 + 6;
    let block_height = 3;
    if area.width < block_width || area.height < block_height {
        return;
    }
    let popup_width = block_width.min(60);
    let popup_area = Rect {
        x: area.x + (area.width - popup_width) / 2,
        y: area.y + (area.height - block_height) / 2,
        width: popup_width,
        height: block_height,
    };
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(theme.highlight).fg(theme.warning));
    let para = Paragraph::new(Line::from(msg))
        .style(
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )
        .block(block);
    frame.render_widget(para, popup_area);
}

pub async fn run_tui(
    cfg: Config,
) -> anyhow::Result<Option<std::sync::Arc<crate::runner::RunStats>>> {
    // The guard owns raw mode, the alternate screen, and mouse capture. It is
    // dropped on every exit path, including `?` returns and panics, so the host
    // terminal is never left unusable.
    let guard = TerminalGuard::enter()?;
    let mut terminal = guard.terminal()?;

    let mut app = App::new(cfg);
    // The controller owns the run; the event loop only carries the frames it
    // produces, so the two are polled independently and neither can block the
    // other's progress.
    let events = EventLoop::new();

    // Redraw only when state actually changed. The poll timeout bounds this
    // loop at ~100 Hz, and an unconditional full redraw at that rate competes
    // with the load generator for the same cores. Snapshots arrive at 10 Hz, so
    // this settles at 10 redraws per second.
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|frame| {
                app.render(frame, frame.area());
            })?;
            dirty = false;
        }

        if events.has_input() {
            match events.next_event() {
                Some(crossterm::event::Event::Key(key)) => {
                    if app.handle_key(key) {
                        break;
                    }
                    dirty = true;
                }
                Some(crossterm::event::Event::Mouse(mouse)) => {
                    app.handle_mouse(mouse);
                    dirty = true;
                }
                _ => {}
            }
        }

        if let Some(snap) = app.controller.poll() {
            app.handle_stats(snap);
            dirty = true;
        }
    }

    if let Some(cancel) = app.cancel_token.take() {
        cancel.cancel();
    }

    // Cancel and wait for the drain barrier *before* the guard drops. A run
    // task still in flight while the terminal is torn down would race the
    // engine's own teardown, and the post-run summary needs the counters to
    // have stopped moving.
    app.controller.shutdown().await;

    let stats = app.controller.stats().cloned();

    // Teardown happens in TerminalGuard::drop.
    Ok(stats)
}
