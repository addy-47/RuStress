use ratatui::style::{Color, Modifier, Style};

/// Stress-themed color palette. Dark terminals get fiery reds/oranges/cyans.
pub struct Theme {
    pub primary: Color,   // Main accent (coral red)
    pub secondary: Color, // Secondary accent (cyan)
    pub success: Color,   // Green for success
    pub warning: Color,   // Orange/amber for warnings
    pub error: Color,     // Red for errors
    pub text: Color,
    pub subtle: Color,
    pub border: Color,
    pub bg: Color,
    pub surface: Color,   // Panel background
    pub highlight: Color, // Focused field background
    pub focused_border: Color,
    pub progress_bar: Color, // Progress bar fill
    pub sparkline: Color,    // Sparkline bar color
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            primary: Color::Rgb(196, 248, 245), // Aqua Foreground (#C4F8F5)
            secondary: Color::Rgb(44, 82, 77),  // Deep Aqua Accent (#2C524D)
            success: Color::Rgb(181, 182, 184), // Muted Green (#B5B6B8)
            warning: Color::Rgb(150, 186, 220), // Muted Yellow (#96BADC)
            error: Color::Rgb(163, 173, 171),   // Muted Red (#A3ADAB)
            text: Color::Rgb(196, 248, 245),    // Aqua Foreground
            subtle: Color::Rgb(142, 173, 202),  // Bright Black/Blue (#8EADCA is approx 142,173,202)
            border: Color::Rgb(79, 137, 139),   // Bright Blue/Cyan (#4F898B)
            bg: Color::Reset,                   // Transparent background
            surface: Color::Reset,              // Transparent surface
            highlight: Color::Rgb(28, 27, 27),  // Black Background (#1C1B1B)
            focused_border: Color::Rgb(196, 248, 245),
            progress_bar: Color::Rgb(44, 82, 77),
            sparkline: Color::Rgb(196, 248, 245),
        }
    }

    pub fn light() -> Self {
        Self {
            primary: Color::Rgb(214, 48, 49),
            secondary: Color::Rgb(0, 149, 200),
            success: Color::Rgb(0, 184, 148),
            warning: Color::Rgb(225, 112, 0),
            error: Color::Rgb(214, 48, 49),
            text: Color::Rgb(20, 20, 30),
            subtle: Color::Rgb(120, 120, 140),
            border: Color::Rgb(200, 200, 220),
            bg: Color::Rgb(250, 250, 255),
            surface: Color::Rgb(240, 240, 250),
            highlight: Color::Rgb(230, 220, 245),
            focused_border: Color::Rgb(214, 48, 49),
            progress_bar: Color::Rgb(0, 149, 200),
            sparkline: Color::Rgb(214, 48, 49),
        }
    }

    pub fn detect_dark_mode() -> bool {
        if let Ok(val) = std::env::var("COLORFGBG") {
            if let Some(parts) = val.rsplit_once(';') {
                if let Ok(bg) = parts.1.parse::<i32>() {
                    return !(7..234).contains(&bg);
                }
            }
        }
        if let Ok(val) = std::env::var("COLORTERM") {
            if val.contains("dark") {
                return true;
            }
        }
        true
    }

    // --- Style factories ---

    pub fn panel_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.surface)
    }

    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn value_style(&self) -> Style {
        Style::default()
            .fg(self.secondary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }

    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning)
    }

    pub fn subtle_style(&self) -> Style {
        Style::default().fg(self.subtle)
    }

    pub fn success_style(&self) -> Style {
        Style::default()
            .fg(self.success)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_style(&self) -> Style {
        Style::default()
            .fg(self.bg)
            .bg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_inactive_style(&self) -> Style {
        Style::default().fg(self.subtle).bg(self.surface)
    }

    pub fn focused_field_style(&self) -> Style {
        Style::default()
            .fg(self.bg)
            .bg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn normal_field_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.surface)
    }

    pub fn focused_border_style(&self) -> Style {
        Style::default().fg(self.focused_border)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn key_hint(&self, key: &str, desc: &str) -> String {
        format!("[{}] {}", key, desc)
    }
}

impl Default for Theme {
    fn default() -> Self {
        if Self::detect_dark_mode() {
            Self::dark()
        } else {
            Self::light()
        }
    }
}
