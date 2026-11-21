use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::core::constants::STATS_CHANNEL_CAPACITY;

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

    /// Move blocking terminal reads off the async runtime worker.
    ///
    /// `crossterm::poll` and `read` are synchronous. Called in an async loop
    /// with no await point, they can starve the load task on a single-thread
    /// runtime and reproduce the "RUNNING with zero requests" failure. A
    /// dedicated blocking thread forwards events over a bounded channel; the
    /// returned token stops it on every exit path.
    pub fn spawn(
        self,
        cancel: CancellationToken,
    ) -> (mpsc::Receiver<crossterm::event::Event>, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(STATS_CHANNEL_CAPACITY);
        let poll_timeout = self.poll_timeout;
        let handle = tokio::task::spawn_blocking(move || {
            loop {
                if cancel.is_cancelled() || tx.is_closed() {
                    break;
                }
                if crossterm::event::poll(poll_timeout).unwrap_or(false) {
                    match crossterm::event::read() {
                        Ok(event) => {
                            // A full channel drops an input event rather than
                            // blocking teardown; 64 buffered keystrokes is ample.
                            if tx.try_send(event).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        (rx, handle)
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}
