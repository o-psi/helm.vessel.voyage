//! A display clock, not an execution-progress estimate. No transcript mutations.
use std::time::{Duration, Instant};

pub(crate) struct Working {
    enabled: bool,
    epoch: Instant,
}

impl Working {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            epoch: Instant::now(),
        }
    }

    pub(crate) fn label(&self, status: &'static str, live: bool) -> &'static str {
        self.at(status, live, self.epoch.elapsed())
    }

    fn at(&self, status: &'static str, live: bool, elapsed: Duration) -> &'static str {
        if status != "Working" {
            return status;
        }
        if !live {
            return "Status unavailable";
        }
        if !self.enabled {
            return status;
        }
        // Each frame occupies the same columns. The ordinary 100 ms repaint is
        // sufficient; missed frames are skipped rather than queued after a stall.
        const FRAMES: [&str; 10] = [
            "⠋ Working",
            "⠙ Working",
            "⠹ Working",
            "⠸ Working",
            "⠼ Working",
            "⠴ Working",
            "⠦ Working",
            "⠧ Working",
            "⠇ Working",
            "⠏ Working",
        ];
        FRAMES[((elapsed.as_millis() / 100) % FRAMES.len() as u128) as usize]
    }
}
