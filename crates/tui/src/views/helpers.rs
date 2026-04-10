use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::theme::Theme;

/// Render a metric card (bordered box with title, value, and optional sparkline).
pub fn render_metric_card(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(title, Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let paragraph = Paragraph::new(value)
        .style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD))
        .block(block);
    frame.render_widget(paragraph, area);
}

/// Render a panel with title and content.
#[allow(dead_code)]
pub fn render_panel(frame: &mut Frame<'_>, area: Rect, title: &str, content: &str, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(title, theme.title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border));

    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, area);
}

/// Render a status badge (RUNNING/DRAINING/FINISHED).
pub fn status_badge_text(status: &str) -> (String, Style) {
    match status {
        "RUNNING" => (
            " ● RUNNING ".to_string(),
            Style::default().fg(ratatui::style::Color::Rgb(85, 239, 196)).add_modifier(Modifier::BOLD),
        ),
        "DRAINING" => (
            " ◐ DRAINING ".to_string(),
            Style::default().fg(ratatui::style::Color::Rgb(253, 203, 110)).add_modifier(Modifier::BOLD),
        ),
        "FINISHED" => (
            " ■ FINISHED ".to_string(),
            Style::default().fg(ratatui::style::Color::Rgb(150, 150, 170)).add_modifier(Modifier::BOLD),
        ),
        _ => (format!(" {status} "), Style::default().fg(ratatui::style::Color::Rgb(150, 150, 170))),
    }
}

/// Split a rectangle into rows with given constraints.
pub fn vertical_chunks(area: Rect, constraints: Vec<Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

/// Split a rectangle into columns.
pub fn horizontal_chunks(area: Rect, constraints: Vec<Constraint>) -> Vec<Rect> {
    Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}
