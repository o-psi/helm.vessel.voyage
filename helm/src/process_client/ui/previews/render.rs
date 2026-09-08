//! Fixed, worker-prepared image cells. No decoder, resizing, terminal query, or
//! multiplexer subprocess runs during drawing.
use image::{DynamicImage, imageops::FilterType};
use ratatui::{
    Frame,
    layout::{Rect, Size},
};
use ratatui_image::{
    Image,
    protocol::{Protocol, halfblocks::Halfblocks, kitty::Kitty},
};
use std::io::{self, Write};

const VARIANTS: [(u16, u16); 3] = [(16, 4), (32, 8), (48, 12)];
const SLOTS: usize = 4;
const MAX_PIXELS: u32 = 512;

#[derive(Clone, Copy)]
enum Mode {
    Text,
    Halfblocks,
    Kitty { cell: Size, first_id: u32 },
}

#[derive(Clone, Copy)]
pub(super) struct Config {
    mode: Mode,
}

impl Config {
    /// Read only environment and OS window geometry, before the event reader.
    /// Do not use Picker: even Picker::halfblocks mutates tmux passthrough.
    pub(super) fn from_env(color: bool, native_color: bool) -> anyhow::Result<Self> {
        let requested = match std::env::var("HELM_IMAGE_PROTOCOL") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => "auto".to_owned(),
            Err(_) => anyhow::bail!("HELM_IMAGE_PROTOCOL must be auto, halfblocks, kitty, or text"),
        };
        anyhow::ensure!(
            matches!(requested.as_str(), "auto" | "halfblocks" | "kitty" | "text"),
            "HELM_IMAGE_PROTOCOL must be auto, halfblocks, kitty, or text"
        );
        let mut mode = if color && requested != "text" {
            Mode::Halfblocks
        } else {
            Mode::Text
        };
        let nonempty = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let term = std::env::var("TERM").unwrap_or_default();
        let multiplexed = nonempty("TMUX")
            || nonempty("STY")
            || term.starts_with("tmux")
            || term.starts_with("screen")
            || std::env::var("TERM_PROGRAM").as_deref() == Ok("tmux");
        let known_kitty = term == "xterm-kitty"
            && nonempty("KITTY_WINDOW_ID")
            && !nonempty("SSH_CONNECTION")
            && !nonempty("SSH_TTY");
        if color
            && native_color
            && !multiplexed
            && (requested == "kitty" || requested == "auto" && known_kitty)
            && let Some(cell) = Self::cell_geometry()
        {
            // Session-local random namespace, twelve IDs reserved and reused.
            let bytes = uuid::Uuid::new_v4().into_bytes();
            let first_id =
                (u32::from_le_bytes(bytes[..4].try_into().expect("four bytes")) & 0x7fff_fff0) + 1;
            mode = Mode::Kitty { cell, first_id };
        }
        Ok(Self { mode })
    }

    fn cell_geometry() -> Option<Size> {
        let window = crossterm::terminal::window_size().ok()?;
        if window.columns == 0 || window.rows == 0 {
            return None;
        }
        let cell = Size::new(window.width / window.columns, window.height / window.rows);
        (cell.width > 0
            && cell.height > 0
            && u32::from(cell.width) * 48 <= MAX_PIXELS
            && u32::from(cell.height) * 12 <= MAX_PIXELS)
            .then_some(cell)
    }

    /// Refresh only passive OS geometry after Resize. The caller must clean up
    /// the previous config's IDs, invalidate prepared images and reconfigure the
    /// worker before using this changed config. Losing geometry selects halfblocks.
    pub(super) fn refresh_geometry(&mut self) -> bool {
        let Mode::Kitty { cell, first_id } = self.mode else {
            return false;
        };
        match Self::cell_geometry() {
            Some(next) if next == cell => false,
            Some(next) => {
                self.mode = Mode::Kitty {
                    cell: next,
                    first_id,
                };
                true
            }
            None => {
                self.mode = Mode::Halfblocks;
                true
            }
        }
    }

    pub(super) fn is_text(&self) -> bool {
        matches!(self.mode, Mode::Text)
    }

    pub(super) fn describe(&self) -> &'static str {
        match self.mode {
            Mode::Text => "text metadata",
            Mode::Halfblocks => "halfblock previews",
            Mode::Kitty { .. } => "Kitty previews",
        }
    }

    /// Delete only this screen's reserved IDs; q=2 prevents unsolicited replies.
    /// The owner must discard all cached prepared images before reusing the IDs,
    /// because ratatui-image's transmitted flags cannot be reset after deletion.
    pub(super) fn cleanup(&self, writer: &mut impl Write) -> io::Result<()> {
        if let Mode::Kitty { first_id, .. } = self.mode {
            for offset in 0..SLOTS * VARIANTS.len() {
                write!(
                    writer,
                    "\x1b_Ga=d,d=I,i={},q=2\x1b\\",
                    first_id + offset as u32
                )?;
            }
            writer.flush()?;
        }
        Ok(())
    }
}

pub(super) struct PreparedPreview {
    variants: Vec<Protocol>,
}

impl PreparedPreview {
    /// Worker-only: decoded input is already bounded and alpha-flattened. Each
    /// variant encodes once; the UI only selects and renders the prepared cells.
    pub(super) fn new(image: DynamicImage, config: &Config, slot: usize) -> Result<Self, String> {
        if image.width() == 0
            || image.height() == 0
            || image.width() > MAX_PIXELS
            || image.height() > MAX_PIXELS
            || slot >= SLOTS
        {
            return Err("Preview dimensions are outside the display limit".into());
        }
        let mut variants = Vec::new();
        for (index, (width, height)) in VARIANTS.into_iter().enumerate() {
            let cell = match config.mode {
                Mode::Text => break,
                Mode::Halfblocks => Size::new(1, 2),
                Mode::Kitty { cell, .. } => cell,
            };
            let thumbnail = image.thumbnail(
                u32::from(width) * u32::from(cell.width),
                u32::from(height) * u32::from(cell.height),
            );
            let size = Size::new(
                thumbnail.width().div_ceil(u32::from(cell.width)) as u16,
                thumbnail.height().div_ceil(u32::from(cell.height)) as u16,
            );
            let protocol = match config.mode {
                Mode::Text => unreachable!(),
                Mode::Halfblocks => Halfblocks::new(thumbnail, size).map(Protocol::Halfblocks),
                Mode::Kitty { first_id, .. } => {
                    // Kitty's virtual placement uses intrinsic pixel dimensions.
                    // Align those dimensions to the known cell metrics so its
                    // full image fits our placeholder rectangle without cropping.
                    let pixels = thumbnail.resize_exact(
                        u32::from(size.width) * u32::from(cell.width),
                        u32::from(size.height) * u32::from(cell.height),
                        FilterType::Triangle,
                    );
                    Kitty::new(
                        pixels,
                        size,
                        first_id + (slot * VARIANTS.len() + index) as u32,
                        false,
                    )
                    .map(Protocol::Kitty)
                }
            }
            .map_err(|_| "Image preview could not be prepared".to_owned())?;
            variants.push(protocol);
        }
        Ok(Self { variants })
    }

    fn fitting(&self, width: u16, height: u16) -> Option<&Protocol> {
        self.variants.iter().rev().find(|protocol| {
            let size = protocol.size();
            size.width <= width && size.height <= height
        })
    }

    pub(super) fn height(&self, width: u16, max_height: u16) -> u16 {
        self.fitting(width, max_height)
            .map_or(0, |protocol| protocol.size().height)
    }

    pub(super) fn draw(&self, frame: &mut Frame<'_>, area: Rect) -> bool {
        let Some(protocol) = self.fitting(area.width, area.height) else {
            return false;
        };
        let size = protocol.size();
        frame.render_widget(
            Image::new(protocol),
            Rect::new(area.x, area.y, size.width, size.height),
        );
        true
    }
}
