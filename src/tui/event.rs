use crate::core::snapshot::StatsSnapshot;
use std::time::Instant;
use tokio::sync::mpsc;

/// Event loop for the TUI.
///
/// Receives stats from the runner and sends periodic tick messages.
pub struct EventLoop {
    pub rx: mpsc::UnboundedReceiver<StatsSnapshot>,
    pub last_stats: StatsSnapshot,
    pub last_update: Instant,
}

impl EventLoop {
    pub fn new(rx: mpsc::UnboundedReceiver<StatsSnapshot>) -> Self {
        Self {
            rx,
            last_stats: StatsSnapshot::default(),
            last_update: Instant::now(),
        }
    }

    /// Try to receive the next message with a timeout.
    pub async fn poll(&mut self, timeout: std::time::Duration) -> Option<StatsSnapshot> {
        tokio::select! {
            snap = self.rx.recv() => {
                if let Some(ref snap) = snap {
                    self.last_stats = snap.clone();
                    self.last_update = Instant::now();
                }
                snap
            }
            _ = tokio::time::sleep(timeout) => None,
        }
    }
}
