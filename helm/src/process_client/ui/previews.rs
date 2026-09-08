//! Owned composer previews. Presentation never acquires files, URLs or clipboard data.
mod backend;
mod decode;
pub(super) mod render;

use super::{App, attachments, paste::Destination};
use backend::{Batch, Source, Worker};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{Frame, layout::Rect, widgets::Paragraph};
use std::{io, sync::Arc};
use uuid::Uuid;

type Identity = (Destination, Vec<(Uuid, String)>);
type Entry = (Uuid, Result<Arc<render::PreparedPreview>, String>);

pub(super) struct State {
    pub enabled: bool,
    config: render::Config,
    worker: Option<Worker>,
    identity: Option<Identity>,
    generation: Option<u64>,
    entries: Vec<Entry>,
    error: Option<String>,
    cleanup_pending: bool,
    finished: bool,
}

impl State {
    fn request(&mut self, sources: Vec<Source>) -> Result<u64, String> {
        if self.worker.is_none() {
            self.worker = Some(
                Worker::new(self.config)
                    .map_err(|_| "Preview worker unavailable; attachment retained")?,
            );
        }
        self.worker
            .as_ref()
            .expect("created worker")
            .request(sources)
    }

    pub fn new(color: bool, native_color: bool) -> anyhow::Result<Self> {
        let config = render::Config::from_env(color, native_color)?;
        Ok(Self {
            enabled: false,
            config,
            worker: None,
            identity: None,
            generation: None,
            entries: Vec::new(),
            error: None,
            cleanup_pending: false,
            finished: false,
        })
    }

    pub(super) fn clear(&mut self) -> anyhow::Result<()> {
        if let Some(worker) = &self.worker {
            worker.clear();
        }
        self.entries.clear();
        self.generation = None;
        self.identity = None;
        self.error = None;
        if self.cleanup_pending {
            self.config.cleanup(&mut io::stdout())?;
            self.cleanup_pending = false;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        let cleanup = self.clear();
        let joined = self
            .worker
            .as_mut()
            .map_or(Ok(()), Worker::shutdown)
            .map_err(anyhow::Error::msg);
        self.finished = true;
        cleanup.and(joined)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish();
        }
    }
}

impl App {
    pub(super) fn resize_previews(&mut self) -> anyhow::Result<()> {
        let mut config = self.previews.config;
        if config.refresh_geometry() {
            self.previews.clear()?;
            self.previews.config = config;
            if let Some(worker) = &mut self.previews.worker {
                worker.reconfigure(config);
            }
        }
        Ok(())
    }

    fn preview_context(&self) -> Option<(Destination, &[attachments::Image])> {
        if self.vessels_open()
            || self.workspace_picker.is_some()
            || self.help
            || self.explore.is_some()
            || self.sidebar.menu.is_some()
            || self.interactions.borrow().focused
            || self.inference_picker_open()
        {
            return None;
        }
        if let Some(id) = self.active_draft {
            return self
                .new_draft_images(id)
                .ok()
                .map(|images| (Destination::Draft(id), images));
        }
        let target = self.selected?;
        let view = self.views.get(&target)?;
        if view.panel.is_some() || view.terminals.open {
            return None;
        }
        Some((Destination::Live(target), &view.images))
    }

    pub(super) fn preview_input(&mut self, event: &Event) -> bool {
        if !matches!(event, Event::Key(key) if key.kind == KeyEventKind::Press && key.modifiers == KeyModifiers::ALT && matches!(key.code, KeyCode::Char('p' | 'P')))
        {
            return false;
        }
        if self.preview_context().is_none() {
            return false;
        }
        self.previews.enabled = !self.previews.enabled;
        self.status = if self.previews.enabled {
            format!(
                "Attachment previews on · {} · Alt+P hides",
                self.previews.config.describe()
            )
        } else {
            "Attachment previews off · metadata retained · Alt+P shows".into()
        };
        true
    }

    pub(super) fn poll_previews(&mut self) -> anyhow::Result<()> {
        // Font/pixel metrics can change without a new row/column count, which
        // some event backends coalesce. Use the existing repaint loop's passive
        // OS geometry read; never query the terminal or add another timer.
        if self.previews.enabled {
            self.resize_previews()?;
        }
        let context = self.preview_context();
        let identity = context.and_then(|(destination, images)| {
            (self.previews.enabled && !images.is_empty() && !self.previews.config.is_text()).then(
                || {
                    (
                        destination,
                        images
                            .iter()
                            .map(|image| {
                                let metadata = image.metadata();
                                (metadata.id, metadata.sha256)
                            })
                            .collect::<Vec<_>>(),
                    )
                },
            )
        });
        if self.previews.identity != identity {
            // Invalidate before new effects. Kitty protocol state cannot be reused
            // after its explicitly owned image IDs have been deleted.
            self.previews.clear()?;
            if let Some(identity) = identity {
                let sources: Result<Vec<_>, _> = self
                    .preview_context()
                    .into_iter()
                    .flat_map(|(_, images)| images)
                    .map(|image| {
                        let metadata = image.metadata();
                        Ok(Source {
                            id: metadata.id,
                            sha256: metadata.sha256,
                            bytes: image.preview_bytes()?,
                        })
                    })
                    .collect::<anyhow::Result<_>>();
                self.previews.identity = Some(identity);
                match sources {
                    Ok(sources) => match self.previews.request(sources) {
                        Ok(generation) => {
                            self.previews.generation = Some(generation);
                            self.previews.cleanup_pending = true;
                        }
                        Err(error) => self.previews.error = Some(error),
                    },
                    Err(_) => {
                        self.previews.error =
                            Some("Preview unavailable: attachment validation failed".into())
                    }
                }
            }
        }
        if let Some(Batch { generation, images }) =
            self.previews.worker.as_ref().and_then(Worker::poll)
            && self.previews.generation == Some(generation)
        {
            self.previews.entries = images;
        }
        Ok(())
    }

    pub(super) fn preview_rows(&self, width: u16, height: u16) -> u16 {
        let Some((_, images)) = self.preview_context() else {
            return 0;
        };
        if !self.previews.enabled
            || images.is_empty()
            || self.previews.config.is_text()
            || height < 28
            || width / (images.len() as u16) < 16
        {
            return 0;
        }
        if height >= 40 { 9 } else { 5 }
    }

    pub(super) fn draw_previews(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some((destination, images)) = self.preview_context() else {
            return;
        };
        if area.height == 0 || images.is_empty() {
            return;
        }
        if self.previews.identity.as_ref().is_none_or(|(owner, keys)| {
            *owner != destination
                || keys.len() != images.len()
                || keys.iter().zip(images).any(|((id, hash), image)| {
                    let metadata = image.metadata();
                    *id != metadata.id || *hash != metadata.sha256
                })
        }) {
            return;
        }
        let width = area.width / images.len() as u16;
        for (index, image) in images.iter().enumerate() {
            let id = image.metadata().id;
            let tile = Rect::new(
                area.x + index as u16 * width,
                area.y,
                width,
                area.height.saturating_sub(1),
            );
            let result = self
                .previews
                .entries
                .iter()
                .find(|entry| entry.0 == id)
                .map(|entry| &entry.1);
            let label = match result {
                Some(Ok(preview))
                    if preview.height(tile.width, tile.height) > 0 && preview.draw(frame, tile) =>
                {
                    format!("Image {}", index + 1)
                }
                Some(Ok(_)) => format!("Image {} · narrow", index + 1),
                Some(Err(error)) => format!("Image {} · {}", index + 1, super::safe(error)),
                None if self.previews.error.is_some() => {
                    format!("Image {} · unavailable", index + 1)
                }
                None => format!("Image {} · loading", index + 1),
            };
            frame.render_widget(
                Paragraph::new(label).style(crate::theme::Role::Muted.style()),
                Rect::new(tile.x, area.bottom().saturating_sub(1), width, 1),
            );
        }
    }
}
