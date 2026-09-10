//! Decorative running-state names, never execution milestones or transcript text.
mod labels;
use labels::Labels;
use ratatui::{Frame, layout::Rect, style::Color};
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};
use tachyonfx::{Effect, Interpolation, fx, pattern::SweepPattern};

pub(crate) struct Working {
    enabled: bool,
    labels: Labels,
    epoch: Instant,
    animated: Cell<bool>,
    area: Cell<Option<Rect>>,
    shimmer: RefCell<Effect>,
}

impl Working {
    pub(crate) fn new(enabled: bool) -> anyhow::Result<Self> {
        Ok(Self {
            labels: Labels::from_env()?,
            enabled,
            epoch: Instant::now(),
            animated: Cell::new(false),
            area: Cell::new(None),
            shimmer: RefCell::new(
                fx::fade_to_fg(Color::Cyan, (1200, Interpolation::Linear))
                    .with_pattern(SweepPattern::left_to_right(4)),
            ),
        })
    }

    pub(crate) fn width(&self) -> u16 {
        self.labels.width
    }

    pub(crate) fn begin_frame(&self) {
        self.animated.set(false);
        self.area.set(None);
    }

    pub(crate) fn label(&self, status: &'static str, live: bool) -> &str {
        self.animated
            .set(self.enabled && live && status == "Working");
        self.at(status, live, self.epoch.elapsed())
    }

    fn at(&self, status: &'static str, live: bool, elapsed: Duration) -> &str {
        if status != "Working" {
            return status;
        }
        if !live {
            return "Status unavailable";
        }
        if !self.enabled {
            return status;
        }
        let ms = elapsed.as_millis();
        &self.labels.frames[((ms / 4000) % self.labels.frames.len() as u128) as usize]
            [((ms / 100) % 10) as usize]
    }

    // The renderer supplies only the word's visible cells, excluding the spinner,
    // host prefix, terminal summary and narrow-layout action button.
    pub(crate) fn record(&self, area: Rect) {
        if self.animated.get() && !area.is_empty() {
            self.area.set(Some(area));
        }
    }

    pub(crate) fn draw(&self, frame: &mut Frame<'_>, visible: bool) {
        if visible && let Some(area) = self.area.get() {
            self.paint(frame.buffer_mut(), area, self.epoch.elapsed());
        }
    }

    fn paint(&self, buffer: &mut ratatui::buffer::Buffer, area: Rect, elapsed: Duration) {
        if !self.enabled {
            return;
        }
        let area = area.intersection(buffer.area);
        if area.is_empty() {
            return;
        }
        let mut shimmer = self.shimmer.borrow_mut();
        // Seek within one bounded cycle, instead of replaying missed cycles after
        // a stall or private-terminal attachment. No timer or effect queue.
        shimmer.reset();
        let phase = (elapsed.as_millis() % 2400) as u64;
        let progress = if phase <= 1200 { phase } else { 2400 - phase };
        shimmer.process(Duration::from_millis(progress), buffer, area);
    }
}
