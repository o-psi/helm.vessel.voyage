//! Typed terminal cells, never an escape stream. One backend owns the cursor.
use anyhow::{Result, ensure};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::terminal::{TerminalColor, TerminalModes, TerminalScreen};

pub(super) fn viewport(columns: u16, rows: u16) -> Rect {
    Rect::new(0, 0, columns.clamp(1, 240), rows.clamp(1, 124))
}

pub(super) struct Observation {
    run: Uuid,
    terminal: Uuid,
    size: (u16, u16),
    revision: Option<u64>,
    screen: Option<TerminalScreen>,
}
impl Observation {
    pub fn new(run: Uuid, terminal: Uuid, size: (u16, u16)) -> Self {
        Self {
            run,
            terminal,
            size,
            revision: None,
            screen: None,
        }
    }
    pub fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
    }
    pub fn modes(&self) -> TerminalModes {
        self.screen.as_ref().map(|s| s.modes).unwrap_or_default()
    }
    pub fn screen(&self) -> Option<&TerminalScreen> {
        self.screen.as_ref()
    }
    pub fn accept(&mut self, value: &Value) -> Result<bool> {
        ensure!(
            value["terminal_id"] == self.terminal.to_string()
                && value["run_id"] == self.run.to_string(),
            "terminal observation identity mismatch"
        );
        let revision = value["revision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("terminal revision missing"))?;
        if self.revision.is_some_and(|previous| revision < previous) {
            return Ok(false);
        }
        let screen: Option<TerminalScreen> = value
            .get("screen")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid private terminal screen"))?;
        if let Some(screen) = &screen {
            screen.validate().map_err(anyhow::Error::msg)?;
            if !value["cursor"].is_null() {
                let cursor = value["cursor"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid terminal cursor"))?;
                ensure!(
                    cursor.len() == 2
                        && cursor[0]
                            .as_u64()
                            .is_some_and(|x| x < u64::from(screen.columns))
                        && cursor[1]
                            .as_u64()
                            .is_some_and(|y| y < u64::from(screen.height)),
                    "terminal cursor outside screen"
                );
            }
            if (screen.columns, screen.height) != self.size {
                return Ok(false);
            }
        }
        self.revision = Some(revision);
        self.screen = screen;
        Ok(true)
    }
}
fn color(color: TerminalColor) -> Color {
    match color {
        TerminalColor::Default => Color::Reset,
        TerminalColor::Indexed(n) => Color::Indexed(n),
        TerminalColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
pub(super) fn render(
    buffer: &mut Buffer,
    area: Rect,
    screen: Option<&TerminalScreen>,
    fallback: &Value,
) {
    if let Some(screen) = screen {
        for (y, row) in screen.cells.iter().take(area.height as usize).enumerate() {
            for (x, cell) in row.iter().take(area.width as usize).enumerate() {
                if cell.width == 0 || x + usize::from(cell.width) > area.width as usize {
                    continue;
                }
                let mut style = Style::default()
                    .fg(color(cell.foreground))
                    .bg(color(cell.background));
                for (enabled, modifier) in [
                    (cell.bold, Modifier::BOLD),
                    (cell.dim, Modifier::DIM),
                    (cell.italic, Modifier::ITALIC),
                    (cell.underlined, Modifier::UNDERLINED),
                    (cell.reversed, Modifier::REVERSED),
                ] {
                    if enabled {
                        style = style.add_modifier(modifier);
                    }
                }
                let target = &mut buffer[(area.x + x as u16, area.y + y as u16)];
                // vt100 and the display can use different Unicode width tables. Keep
                // explicit coordinates, never permit a peer to write across cells.
                use unicode_segmentation::UnicodeSegmentation;
                use unicode_width::UnicodeWidthStr;
                let valid = cell.text.graphemes(true).count() == 1
                    && cell.text.width() == usize::from(cell.width);
                target
                    .set_symbol(if valid { &cell.text } else { "�" })
                    .set_style(style);
                if !valid && cell.width == 2 {
                    buffer[(area.x + x as u16 + 1, area.y + y as u16)]
                        .set_symbol(" ")
                        .set_style(style);
                }
            }
        }
    } else {
        use ratatui::widgets::{Paragraph, Widget};
        for (y, row) in fallback
            .as_array()
            .into_iter()
            .flatten()
            .take(area.height as usize)
            .enumerate()
        {
            Paragraph::new(super::clipped(row.as_str().unwrap_or_default(), area.width))
                .render(Rect::new(area.x, area.y + y as u16, area.width, 1), buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::terminal::TerminalCell;
    fn screen(columns: u16, height: u16) -> TerminalScreen {
        TerminalScreen {
            version: 1,
            columns,
            height,
            cells: vec![vec![TerminalCell::default(); columns as usize]; height as usize],
            modes: TerminalModes::default(),
        }
    }
    #[test]
    fn viewport_allocation_is_bounded_even_for_extreme_resize() {
        assert_eq!(viewport(u16::MAX, u16::MAX), Rect::new(0, 0, 240, 124));
        assert_eq!(viewport(0, 0), Rect::new(0, 0, 1, 1));
    }
    #[test]
    fn identity_revision_and_resize_fences() {
        let run = Uuid::new_v4();
        let terminal = Uuid::new_v4();
        let mut observation = Observation::new(run, terminal, (2, 1));
        let mut value = serde_json::json!({"run_id":run,"terminal_id":terminal,"revision":2,"screen":screen(2,1)});
        assert!(observation.accept(&value).unwrap());
        value["revision"] = 1.into();
        assert!(!observation.accept(&value).unwrap());
        value["revision"] = 3.into();
        observation.resize((3, 1));
        assert!(!observation.accept(&value).unwrap());
        value["screen"] = serde_json::to_value(screen(3, 1)).unwrap();
        assert!(observation.accept(&value).unwrap());
        value["cursor"] = serde_json::json!([3, 0]);
        assert!(observation.accept(&value).is_err());
        value["cursor"] = Value::Null;
        value["terminal_id"] = Uuid::new_v4().to_string().into();
        assert!(observation.accept(&value).is_err());
    }
    #[test]
    fn styled_wide_combining_and_monochrome_fallback() {
        let mut value = screen(4, 1);
        value.cells[0][0].text = "界".into();
        value.cells[0][0].width = 2;
        value.cells[0][0].foreground = TerminalColor::Rgb(1, 2, 3);
        value.cells[0][0].bold = true;
        value.cells[0][1].text.clear();
        value.cells[0][1].width = 0;
        value.cells[0][2].text = "e\u{301}".into();
        let area = Rect::new(0, 0, 4, 1);
        let mut buffer = Buffer::empty(area);
        render(&mut buffer, area, Some(&value), &Value::Null);
        assert_eq!(buffer[(0, 0)].symbol(), "界");
        assert_eq!(buffer[(1, 0)].symbol(), " ");
        assert_eq!(buffer[(2, 0)].symbol(), "e\u{301}");
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(1, 2, 3));
        assert!(buffer[(0, 0)].modifier.contains(Modifier::BOLD));
        let area = Rect::new(0, 0, 1, 1);
        let mut buffer = Buffer::empty(area);
        render(&mut buffer, area, Some(&value), &Value::Null);
        assert_eq!(buffer[(0, 0)].symbol(), " ");
        render(&mut buffer, area, None, &serde_json::json!(["a\x1b[31m"]));
        assert_eq!(buffer[(0, 0)].symbol(), "a");
    }
    #[test]
    fn measured_bounded_screen_cost() {
        use std::time::Instant;
        for (columns, height) in [(80, 24), (240, 120)] {
            let value = screen(columns, height);
            value.validate().unwrap();
            let start = Instant::now();
            let bytes = serde_json::to_vec(&value).unwrap();
            let serialize = start.elapsed();
            let start = Instant::now();
            let decoded: TerminalScreen = serde_json::from_slice(&bytes).unwrap();
            let decode = start.elapsed();
            let area = Rect::new(0, 0, columns, height);
            let start = Instant::now();
            let mut buffer = Buffer::empty(area);
            let allocate = start.elapsed();
            let start = Instant::now();
            render(&mut buffer, area, Some(&decoded), &Value::Null);
            let render = start.elapsed();
            println!(
                "terminal {columns}x{height}: wire={} bytes, owned_cell_headers={} bytes + text, serialize={serialize:?}, decode={decode:?}, buffer_allocate={allocate:?}, render={render:?}",
                bytes.len(),
                usize::from(columns) * usize::from(height) * std::mem::size_of::<TerminalCell>()
            );
            assert!(bytes.len() <= voyage_protocol::terminal::MAX_SCREEN_BYTES);
        }
    }
}
