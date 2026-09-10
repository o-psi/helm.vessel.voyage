//! Presentation-only motion. Never touches conversation content.
mod working;
use super::{App, state::Target};
use ratatui::{Frame, layout::Rect, style::Color};
use std::time::{Duration, Instant};
use tachyonfx::{Effect, Interpolation, fx};
pub(super) use working::Working;

pub(super) struct Navigation {
    enabled: bool,
    selected: Option<Target>,
    geometry: Option<Rect>,
    effect: Option<Effect>,
    last_frame: Instant,
}

impl Navigation {
    pub(super) fn from_env(color: bool) -> anyhow::Result<Self> {
        let enabled = match std::env::var("HELM_MOTION") {
            Err(std::env::VarError::NotPresent) => color,
            Ok(value) if value == "auto" => color,
            Ok(value) if value == "never" => false,
            _ => anyhow::bail!("HELM_MOTION must be auto or never"),
        };
        Ok(Self {
            enabled,
            selected: None,
            geometry: None,
            effect: None,
            last_frame: Instant::now(),
        })
    }

    pub(super) fn working_indicator(&self) -> Working {
        Working::new(self.enabled)
    }

    pub(super) fn repaint_after(&self) -> Duration {
        Duration::from_millis(if self.effect.is_some() { 33 } else { 100 })
    }

    pub(super) fn draw(&mut self, frame: &mut Frame<'_>, app: &App) {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;
        // Use current hit geometry, not a duplicate layout or stale coordinates.
        // Modal forms must never receive effects intended for underlying widgets.
        let visible = !app.help
            && app.explore.is_none()
            && app.active_draft.is_none()
            && app.sidebar.menu.is_none()
            && !app.interactions.borrow().focused
            && !app.inference_picker_open()
            && app.workspace_picker.is_none()
            && app.vessels.as_ref().is_none_or(|v| !v.borrow().is_open());
        app.working.draw(frame, visible);
        let area = if visible {
            app.sidebar
                .hits
                .borrow()
                .iter()
                .find(|hit| Some(hit.target) == app.selected)
                .map(|hit| hit.button.intersection(frame.area()))
                .filter(|area| !area.is_empty())
        } else {
            None
        };
        let changed = self.selected != app.selected;
        self.selected = app.selected;
        if !self.enabled || area.is_none() || self.geometry != Some(frame.area()) {
            self.effect = None;
        } else if changed {
            // Accent the selected voyage's action affordance, not its status colors.
            // Replacement rather than a queue bounds rapid navigation to one effect.
            self.effect = Some(fx::fade_from_fg(
                Color::White,
                (240, Interpolation::QuadOut),
            ));
        }
        self.geometry = Some(frame.area());
        if let (Some(effect), Some(area)) = (&mut self.effect, area) {
            effect.process(
                if changed { Duration::ZERO } else { elapsed },
                frame.buffer_mut(),
                area,
            );
            if effect.done() {
                self.effect = None;
            }
        }
    }
}
