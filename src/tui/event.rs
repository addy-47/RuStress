use std::time::Duration;

/// Terminal event loop for the TUI.
///
/// Terminal input only. It previously also carried the engine's stats channel,
/// which made it the single point through which every measurement passed — a
/// responsibility that belongs to `runner::RunController`, and one that could
/// not be tested without a TTY. Keeping the loop to terminal concerns means the
/// run lifecycle is testable on its own and the input path has one job.
pub struct EventLoop {
    /// Longest the loop will block waiting for input before returning.
    ///
    /// This also bounds how long a snapshot can wait to be drawn, so it is the
    /// dashboard's refresh floor.
    pub poll_timeout: Duration,
}

impl EventLoop {
    pub fn new() -> Self {
        Self {
            poll_timeout: Duration::from_millis(10),
        }
    }

    /// Whether input is waiting, without blocking the async runtime.
    pub fn has_input(&self) -> bool {
        crossterm::event::poll(self.poll_timeout).unwrap_or(false)
    }

    /// The next terminal event, if one is pending.
    pub fn next_event(&self) -> Option<crossterm::event::Event> {
        crossterm::event::read().ok()
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}
