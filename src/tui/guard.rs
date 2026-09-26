use std::io::{self, Stdout, Write};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// RAII guard that restores the host terminal on every exit path.
///
/// A terminal UI that enables raw mode and the alternate screen must guarantee
/// they are undone. Manual cleanup at the end of the run is not enough: an `?`
/// on any intervening call, a panic, or an abort all skip it and leave the user
/// with a shell that echoes nothing and ignores Ctrl-C. Dropping this guard
/// runs the same teardown on the error and unwind paths.
///
/// # Unwind requirement
///
/// Teardown runs in `Drop`, so the release profile must **not** set
/// `panic = "abort"` — an abort skips destructors entirely and reintroduces the
/// broken-terminal failure this guard exists to prevent.
pub struct TerminalGuard {
    stdout: Stdout,
    active: bool,
}

impl TerminalGuard {
    /// Take over the terminal: raw mode, alternate screen, mouse capture.
    pub fn enter() -> io::Result<Self> {
        let stdout = io::stdout();
        enable_raw_mode()?;

        // Construct the guard before entering the alternate screen. If the
        // `execute!` below fails, the `?` returns while `guard` is still in
        // scope, so its Drop disables raw mode. Returning before the guard
        // exists would leave the user with a shell that ignores Ctrl-C.
        let mut guard = Self {
            stdout,
            active: true,
        };

        execute!(
            guard.stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture
        )?;

        Ok(guard)
    }

    /// Build a ratatui terminal drawing into the guarded screen.
    ///
    /// `io::stdout()` is a cheap handle to the same underlying descriptor the
    /// guard holds, so the renderer and the teardown path write to the same
    /// terminal without the guard giving up ownership.
    pub fn terminal(&self) -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
        Terminal::new(CrosstermBackend::new(io::stdout()))
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;

        // Best-effort teardown. A failure here must not mask the original error
        // and must not panic inside a destructor, so results are discarded.
        let _ = disable_raw_mode();
        let _ = execute!(
            self.stdout,
            LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
            crossterm::cursor::Show
        );
        let _ = self.stdout.flush();
    }
}
