use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use super::helpers::vertical_chunks;
use crate::core::config::{Config, Mode};
use crate::tui::theme::Theme;

/// Configuration form field indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormField {
    Url,
    Method,
    Headers,
    Body,
    LoadMode,
    Target,
    Duration,
    RampUp,
    RampDown,
    ThinkTime,
}

impl FormField {
    fn label(&self) -> &'static str {
        match self {
            FormField::Url => "URL",
            FormField::Method => "Method",
            FormField::Headers => "Headers",
            FormField::Body => "Body",
            FormField::LoadMode => "Load Mode",
            FormField::Target => "Target",
            FormField::Duration => "Duration",
            FormField::RampUp => "Ramp Up",
            FormField::RampDown => "Ramp Down",
            FormField::ThinkTime => "Think Time",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            FormField::Url => "The base URL for the target server",
            FormField::Method => "Select the HTTP request method",
            FormField::Headers => "Define custom headers (format: Key: Value)",
            FormField::Body => "Payload contents for POST/PUT requests",
            FormField::LoadMode => "The model used to generate pressure",
            FormField::Target => "The intensity of the load generation",
            FormField::Duration => "How long to maintain the steady pressure",
            FormField::RampUp => "Transition time to reach the full load",
            FormField::RampDown => "Gradual cool-down before finishing the test",
            FormField::ThinkTime => "Wait time between virtual user requests",
        }
    }

    fn detail(&self) -> &'static str {
        match self {
            FormField::Url => {
                "Specify the endpoint you want to pressure test. Ensure the protocol (http/https) is included for the engine to connect properly."
            }
            FormField::Method => {
                "Standard HTTP methods used for your API. Most load tests use GET for reading or POST for submitting data."
            }
            FormField::Headers => {
                "Add headers such as Authorization, API-Keys, or User-Agent. Each header must be on a new line and follow the standard 'Key: Value' format."
            }
            FormField::Body => {
                "For POST/PUT methods, enter raw data here. You can also specify a relative file path using @ (e.g., @data.json) to load large payloads from disk."
            }
            FormField::LoadMode => {
                "RPS (Open-Loop): Generates fixed requests per second regardless of server speed. USERS (Closed-Loop): Fixed number of concurrent clients looping requests."
            }
            FormField::Target => {
                "In RPS mode, this defines goal requests/sec. In USERS mode, it defines total concurrent virtual users simulated by the runner."
            }
            FormField::Duration => {
                "This is the duration for the 'top' of the test profile. Total test time equals Ramp-Up + Steady Duration + Ramp-Down."
            }
            FormField::RampUp => {
                "Essential for avoiding immediate connection spikes that might trigger firewall blocks or instant system failure. Smoothly builds traffic over time."
            }
            FormField::RampDown => {
                "Allows the server to clear its request queues and finish processing pending tasks gracefully before the engine stops reporting."
            }
            FormField::ThinkTime => {
                "Simulates real user behavior by adding a pause (in ms) between each iteration. Helps model realistic traffic for stateful applications."
            }
        }
    }

    fn example(&self) -> &'static str {
        match self {
            FormField::Url => "http://api.staging.local/v1/search",
            FormField::Method => "POST",
            FormField::Headers => "Authorization: Bearer <token>\nAccept: application/json",
            FormField::Body => "{\"query\": \"performance\", \"limit\": 10}",
            FormField::LoadMode => "Press SPACE to toggle between RPS and USERS",
            FormField::Target => "RPS: 500  |  USERS: 50",
            FormField::Duration => "60s (Recommended for initial baseline)",
            FormField::RampUp => "10s (Linear increase for better stability)",
            FormField::RampDown => "5s (Clear queues for accurate reporting)",
            FormField::ThinkTime => "150ms (Simulates typical browser delay)",
        }
    }
}

/// Think time only applies in closed-loop mode; every other field is always
/// visible.
fn is_field_visible(field: FormField, load_mode: &str) -> bool {
    field != FormField::ThinkTime || load_mode == "users"
}

fn visible_fields(load_mode: &str) -> Vec<FormField> {
    [
        FormField::Url,
        FormField::Method,
        FormField::Headers,
        FormField::Body,
        FormField::LoadMode,
        FormField::Target,
        FormField::Duration,
        FormField::RampUp,
        FormField::RampDown,
        FormField::ThinkTime,
    ]
    .into_iter()
    .filter(|f| is_field_visible(*f, load_mode))
    .collect()
}

/// Runner configuration form view with editable inputs.
pub struct RunnerView {
    pub inputs: Vec<(FormField, Input)>,
    pub load_mode: String,
    pub focused_field: usize,
    pub width: u16,
    pub height: u16,
    pub errors: std::collections::HashMap<FormField, String>,
}

impl RunnerView {
    pub fn new(initial: Config) -> Self {
        let load_mode = match initial.mode {
            Mode::Rps => "rps",
            Mode::Users => "users",
        };

        let inputs = vec![
            (FormField::Url, Input::new(initial.url)),
            (FormField::Method, Input::new(initial.method)),
            (FormField::Headers, Input::default()),
            (
                FormField::Body,
                Input::new(initial.body.unwrap_or_default()),
            ),
            (FormField::LoadMode, Input::new(load_mode.to_string())),
            (
                FormField::Target,
                Input::new(if load_mode == "rps" {
                    initial.target_rps.to_string()
                } else {
                    initial.num_users.to_string()
                }),
            ),
            (
                FormField::Duration,
                Input::new(initial.steady_dur_secs.to_string()),
            ),
            (
                FormField::RampUp,
                Input::new(initial.ramp_up_secs.to_string()),
            ),
            (
                FormField::RampDown,
                Input::new(initial.ramp_down_secs.to_string()),
            ),
            (
                FormField::ThinkTime,
                Input::new(initial.think_time_ms.to_string()),
            ),
        ];

        Self {
            inputs,
            load_mode: load_mode.to_string(),
            focused_field: 0,
            width: 80,
            height: 24,
            errors: std::collections::HashMap::new(),
        }
    }

    pub fn validate(&mut self) {
        self.errors.clear();
        let url = self.inputs[0].1.value();
        if url.is_empty() {
            self.errors
                .insert(FormField::Url, "URL cannot be empty".to_string());
        } else if !url.starts_with("http") {
            self.errors.insert(
                FormField::Url,
                "Must start with http:// or https://".to_string(),
            );
        }

        let target_str = self.inputs[5].1.value();
        if target_str.parse::<u32>().is_err() {
            self.errors
                .insert(FormField::Target, "Must be a positive integer".to_string());
        }

        let dur_str = self.inputs[6].1.value();
        if dur_str.parse::<u64>().is_err() {
            self.errors.insert(
                FormField::Duration,
                "Must be a positive integer".to_string(),
            );
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub fn focus_next(&mut self) {
        let visible = visible_fields(&self.load_mode);
        if visible.is_empty() {
            return;
        }
        let current_idx = self.focused_field.min(visible.len() - 1);
        self.focused_field = (current_idx + 1) % visible.len();
    }

    pub fn focus_prev(&mut self) {
        let visible = visible_fields(&self.load_mode);
        if visible.is_empty() {
            return;
        }
        let current_idx = self.focused_field.min(visible.len() - 1);
        self.focused_field = if current_idx == 0 {
            visible.len() - 1
        } else {
            current_idx - 1
        };
    }

    pub fn toggle_load_mode(&mut self) {
        if self.load_mode == "rps" {
            self.load_mode = "users".to_string();
            self.inputs[4].1 = Input::new("users".to_string());
            self.inputs[5].0 = FormField::Target;
        } else {
            self.load_mode = "rps".to_string();
            self.inputs[4].1 = Input::new("rps".to_string());
            self.inputs[5].0 = FormField::Target;
        }
        self.focused_field = 0;
    }

    /// Handle a key event for the currently focused input field.
    /// Returns true if should quit.
    pub fn handle_input(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;

        let visible = visible_fields(&self.load_mode);
        if visible.is_empty() {
            return false;
        }
        if self.focused_field >= visible.len() {
            self.focused_field = 0;
        }

        // Navigation keys that don't type into input
        match (key.modifiers, key.code) {
            (_, KeyCode::Tab) | (_, KeyCode::Down) => {
                self.focus_next();
                return false;
            }
            (_, KeyCode::BackTab) | (_, KeyCode::Up) => {
                self.focus_prev();
                return false;
            }
            (_, KeyCode::Right) => {
                let current = self.focused_field;
                if current < 5 && current + 5 < visible.len() {
                    self.focused_field += 5;
                }
                return false;
            }
            (_, KeyCode::Left) => {
                let current = self.focused_field;
                if current >= 5 {
                    self.focused_field -= 5;
                }
                return false;
            }
            (_, KeyCode::Enter) => {
                if self.focused_field < visible.len() - 1 {
                    self.focus_next();
                }
                return false;
            }
            (_, KeyCode::Esc) => return true,
            // Space toggles load mode (Smart Global Toggle)
            (_, KeyCode::Char(' ')) => {
                let field = visible[self.focused_field];
                // Only block toggle if we are in a text-heavy field that actually needs spaces
                if field != FormField::Headers && field != FormField::Body {
                    self.toggle_load_mode();
                    return false;
                }
            }
            _ => {}
        }

        let global_idx = visible[self.focused_field] as usize;
        // Pass key to tui-input
        let event = crossterm::event::Event::Key(key);
        let input = &mut self.inputs[global_idx].1;
        input.handle_event(&event);
        self.validate();
        false
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::MouseEventKind;
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            let visible = visible_fields(&self.load_mode);
            // A frame narrower than two columns yields col_width 0, and the
            // integer division below would panic.
            let col_width = (self.width / 2).max(1);
            let row = (mouse.row as i32 - 1) / 2;
            let col = mouse.column / col_width;

            if (0..5).contains(&row) {
                let idx = if col == 0 {
                    row as usize
                } else {
                    row as usize + 5
                };
                if idx < visible.len() {
                    self.focused_field = idx;
                }
            }
        }
    }

    pub fn get_config(&self) -> Config {
        let url = self.inputs[0].1.value().to_string();
        let method = self.inputs[1].1.value().to_string();
        let body_raw = self.inputs[3].1.value().to_string();
        let body = if body_raw.is_empty() {
            None
        } else {
            Some(body_raw)
        };
        let load_mode = &self.load_mode;
        let target_str = self.inputs[5].1.value();
        let duration_str = self.inputs[6].1.value();
        let ramp_up_str = self.inputs[7].1.value();
        let ramp_down_str = self.inputs[8].1.value();
        let think_time_str = self.inputs[9].1.value();

        let target = target_str
            .parse()
            .unwrap_or(if load_mode == "rps" { 100 } else { 10 });
        let duration = duration_str.parse().unwrap_or(30);
        let ramp_up = ramp_up_str.parse().unwrap_or(0);
        let ramp_down = ramp_down_str.parse().unwrap_or(0);
        let think_time = think_time_str.parse().unwrap_or(0);

        let mode = if load_mode == "users" {
            Mode::Users
        } else {
            Mode::Rps
        };
        let num_users = if matches!(mode, Mode::Users) {
            target
        } else {
            10
        };

        Config {
            url,
            method: if method.is_empty() {
                "GET".to_string()
            } else {
                method
            },
            body,
            headers: Default::default(),
            target_rps: if matches!(mode, Mode::Rps) {
                target
            } else {
                100
            },
            steady_dur_secs: duration,
            ramp_up_secs: ramp_up,
            ramp_down_secs: ramp_down,
            timeout_secs: 30,
            mode,
            num_users,
            think_time_ms: think_time,
            command: None,
            out_prefix: None,
            max_concurrency: 1000,
            pool_max_idle_per_host: 64,
            pool_idle_timeout_secs: 15,
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        use super::helpers::horizontal_chunks;

        if self.width < 70 || self.height < 16 {
            let block = Block::default()
                .title(" Configuration ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.bg));
            frame.render_widget(
                Paragraph::new("Terminal too small — resize to at least 70×16")
                    .style(Style::default().fg(theme.text))
                    .block(block),
                area,
            );
            return;
        }

        let visible = visible_fields(&self.load_mode);
        let main_chunks = vertical_chunks(
            area,
            vec![
                Constraint::Length(1),
                Constraint::Min(10),
                Constraint::Length(3),
            ],
        );

        // Header Mode Indicator
        let mode_label = if self.load_mode == "rps" {
            " ⚡ RPS MODE (Open Loop) "
        } else {
            " 👥 USERS MODE (Closed Loop) "
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    mode_label,
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Press [Space] to toggle mode", theme.subtle_style()),
            ])),
            main_chunks[0],
        );

        let cols = horizontal_chunks(
            main_chunks[1],
            vec![Constraint::Percentage(50), Constraint::Percentage(50)],
        );

        let mut left_lines = vec![];
        let mut right_lines = vec![];

        for (i, field) in visible.iter().enumerate() {
            let global_idx = *field as usize;
            let is_focused = i == self.focused_field;
            let label = field.label();
            let input = &self.inputs[global_idx].1;
            let value = input.value();

            let indicator = if is_focused { "▶ " } else { "  " };
            let is_error = self.errors.contains_key(field);

            let label_style = if is_error {
                Style::default()
                    .fg(ratatui::style::Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else if is_focused {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD)
            };

            let val_style = if is_focused {
                Style::default().fg(theme.bg).bg(if is_error {
                    ratatui::style::Color::Red
                } else {
                    theme.primary
                })
            } else {
                Style::default()
                    .fg(if is_error {
                        ratatui::style::Color::Red
                    } else {
                        theme.text
                    })
                    .bg(theme.bg)
            };

            let line = Line::from(vec![
                Span::styled(indicator, Style::default().fg(theme.primary)),
                Span::styled(format!("{:>10}: ", label), label_style),
                Span::styled(
                    format!(" {} ", if value.is_empty() { "..." } else { value }),
                    val_style,
                ),
            ]);

            if i < 5 {
                left_lines.push(line);
                left_lines.push(Line::from("")); // Spacer
            } else {
                right_lines.push(line);
                right_lines.push(Line::from("")); // Spacer
            }
        }

        frame.render_widget(
            Paragraph::new(left_lines).block(
                Block::default()
                    .title(" BASIC ")
                    .borders(Borders::NONE)
                    .padding(ratatui::widgets::Padding::uniform(1)),
            ),
            cols[0],
        );
        frame.render_widget(
            Paragraph::new(right_lines).block(
                Block::default()
                    .title(" ADVANCED ")
                    .borders(Borders::NONE)
                    .padding(ratatui::widgets::Padding::uniform(1)),
            ),
            cols[1],
        );

        // ── Info Area (Expanding on your feedback) ──
        if self.focused_field < visible.len() {
            let field = visible[self.focused_field];
            // We use the last chunk of main_chunks from the parent layout
            let area = main_chunks[2];

            let footer_split =
                vertical_chunks(area, vec![Constraint::Length(1), Constraint::Min(5)]);

            // ── Info Header ──
            let (label, badge_style) = if self.errors.contains_key(&field) {
                (
                    " ERROR ",
                    theme
                        .error_style()
                        .bg(ratatui::style::Color::Red)
                        .fg(theme.bg),
                )
            } else {
                (
                    " INFO ",
                    theme.panel_style().bg(theme.secondary).fg(theme.bg),
                )
            };

            let info_header = Line::from(vec![
                Span::styled(label, badge_style.add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(field.hint(), theme.title_style()),
            ]);
            frame.render_widget(Paragraph::new(info_header), footer_split[0]);

            // ── Descriptive Help Box ──
            let mut help_text = vec![
                Line::from(vec![
                    Span::styled("  Description: ", theme.subtle_style()),
                    Span::raw(field.detail()),
                ]),
                Line::from(vec![
                    Span::styled("  Example:     ", theme.subtle_style()),
                    Span::styled(field.example(), theme.value_style()),
                ]),
            ];

            // If there's an error, show it prominently
            if let Some(err) = self.errors.get(&field) {
                help_text.insert(
                    0,
                    Line::from(vec![
                        Span::styled("  CAUTION:     ", theme.error_style()),
                        Span::styled(err, theme.error_style()),
                    ]),
                );
                help_text.insert(1, Line::from(""));
            }

            let help_panel = Paragraph::new(help_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(theme.border_style()),
                )
                .style(theme.normal_field_style());

            frame.render_widget(help_panel, footer_split[1]);
        }
    }
}
