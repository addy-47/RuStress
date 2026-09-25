//! Interactive terminal dashboard (ratatui + crossterm).

pub mod app;
pub mod banner;
pub mod event;
pub mod guard;
pub mod theme;
pub mod views;

pub use app::{run_tui, App};
pub use banner::{banner, short_banner};
pub use event::EventLoop;
pub use guard::TerminalGuard;
pub use theme::Theme;
