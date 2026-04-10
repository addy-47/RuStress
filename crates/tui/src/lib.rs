pub mod app;
pub mod banner;
pub mod event;
pub mod theme;
pub mod views;

pub use app::{run_tui, App};
pub use banner::{banner, short_banner};
pub use event::EventLoop;
pub use theme::Theme;

#[cfg(test)]
mod tests {
    use crate::banner;
    use crate::short_banner;
    use crate::Theme;

    #[test]
    fn test_banner_not_empty() {
        let b = banner();
        assert!(!b.is_empty());
        assert!(b.contains("████"));
        assert!(b.contains("v0.1.0"));
    }

    #[test]
    fn test_theme_dark_mode() {
        let theme = Theme::dark();
        assert_eq!(theme.bg, ratatui::style::Color::Rgb(18, 5, 9));
        assert_eq!(theme.text, ratatui::style::Color::Rgb(224, 208, 213));
    }

    #[test]
    fn test_theme_light_mode() {
        let theme = Theme::light();
        assert_eq!(theme.bg, ratatui::style::Color::Rgb(250, 250, 255));
        assert_eq!(theme.text, ratatui::style::Color::Rgb(20, 20, 30));
    }

    #[test]
    fn test_theme_styles_not_crash() {
        let theme = Theme::dark();
        let _ = theme.panel_style();
        let _ = theme.title_style();
        let _ = theme.value_style();
        let _ = theme.error_style();
        let _ = theme.tab_active_style();
        let _ = theme.tab_inactive_style();
        let _ = theme.focused_field_style();
    }

    #[test]
    fn test_short_banner() {
        let b = short_banner();
        assert!(b.contains("RUSTRESS"));
        assert!(b.contains("v0.1.0"));
    }
}
