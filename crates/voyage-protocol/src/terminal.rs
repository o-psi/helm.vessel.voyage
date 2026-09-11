//! Bounded, terminal-emulated human-only screens. Never journal or replay these frames.
use serde::{Deserialize, Serialize};

pub const MAX_COLUMNS: u16 = 240;
pub const MAX_ROWS: u16 = 120;
pub const MAX_SCREEN_BYTES: usize = 1024 * 1024;
pub const MAX_CELL_BYTES: usize = 64;
pub const SCREEN_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}
fn default_color(value: &TerminalColor) -> bool {
    *value == TerminalColor::Default
}
fn is_false(value: &bool) -> bool {
    !value
}
fn one() -> u8 {
    1
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalCell {
    pub text: String,
    /// 0 is the empty continuation of the preceding width-2 cell.
    #[serde(default = "one")]
    pub width: u8,
    #[serde(default, skip_serializing_if = "default_color")]
    pub foreground: TerminalColor,
    #[serde(default, skip_serializing_if = "default_color")]
    pub background: TerminalColor,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underlined: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reversed: bool,
}
impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            width: 1,
            foreground: TerminalColor::Default,
            background: TerminalColor::Default,
            bold: false,
            dim: false,
            italic: false,
            underlined: false,
            reversed: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalModes {
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalScreen {
    pub version: u8,
    pub columns: u16,
    pub height: u16,
    pub cells: Vec<Vec<TerminalCell>>,
    pub modes: TerminalModes,
}
/// Controls and directionality overrides must never reach terminal presentation.
pub fn safe_cell_char(ch: char) -> bool {
    !ch.is_control()
        && !matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' |
        '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}')
}
impl TerminalScreen {
    /// Validate after decoding an already size-limited transport frame, before display.
    /// Oversized screens are refused, not silently cropped or stripped of styles.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != SCREEN_VERSION {
            return Err("unsupported terminal screen version");
        }
        if !(1..=MAX_COLUMNS).contains(&self.columns) || !(1..=MAX_ROWS).contains(&self.height) {
            return Err("terminal screen dimensions exceed bounds");
        }
        if self.cells.len() != usize::from(self.height) {
            return Err("terminal screen row mismatch");
        }
        for row in &self.cells {
            if row.len() != usize::from(self.columns) {
                return Err("terminal screen column mismatch");
            }
            for (column, cell) in row.iter().enumerate() {
                if cell.text.len() > MAX_CELL_BYTES || !cell.text.chars().all(safe_cell_char) {
                    return Err("unsafe terminal cell text");
                }
                match cell.width {
                    0 if cell.text.is_empty() && column > 0 && row[column - 1].width == 2 => (),
                    1 if !cell.text.is_empty() => (),
                    2 if !cell.text.is_empty()
                        && row.get(column + 1).is_some_and(|next| next.width == 0) =>
                    {
                        ()
                    }
                    _ => return Err("invalid terminal cell width or continuation"),
                }
            }
        }
        // Count without allocating another potentially oversized serialized screen.
        struct Limit(usize);
        impl std::io::Write for Limit {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                if self.0 > MAX_SCREEN_BYTES {
                    return Err(std::io::Error::other("screen limit"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Limit(0), self)
            .map_err(|_| "serialized terminal screen exceeds 1 MiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn screen() -> TerminalScreen {
        TerminalScreen {
            version: 1,
            columns: 2,
            height: 1,
            cells: vec![vec![TerminalCell::default(); 2]],
            modes: TerminalModes::default(),
        }
    }
    #[test]
    fn roundtrip_wide_colors_and_modes() {
        let mut value = screen();
        value.cells[0][0] = TerminalCell {
            text: "界".into(),
            width: 2,
            foreground: TerminalColor::Indexed(123),
            background: TerminalColor::Rgb(1, 2, 3),
            bold: true,
            dim: true,
            italic: true,
            underlined: true,
            reversed: true,
        };
        value.cells[0][1].text.clear();
        value.cells[0][1].width = 0;
        value.modes = TerminalModes {
            app_cursor: true,
            app_keypad: true,
            bracketed_paste: true,
        };
        value.validate().unwrap();
        assert_eq!(
            value,
            serde_json::from_slice::<TerminalScreen>(&serde_json::to_vec(&value).unwrap()).unwrap()
        );
    }
    #[test]
    fn rejects_unsafe_malformed_and_oversized_screens() {
        for text in ["\x1b", "\n", "\u{009b}", "\u{202e}", "\u{2066}", "\u{061c}"] {
            let mut value = screen();
            value.cells[0][0].text = text.into();
            assert!(value.validate().is_err());
        }
        let mut value = screen();
        value.cells[0][0].text = "a".repeat(65);
        assert!(value.validate().is_err());
        value = screen();
        value.cells[0][0].width = 0;
        assert!(value.validate().is_err());
        value = screen();
        value.cells[0][1].width = 2;
        assert!(value.validate().is_err());
        value = screen();
        value.version = 2;
        assert!(value.validate().is_err());
        value = screen();
        value.columns = 241;
        assert!(value.validate().is_err());
        value = screen();
        value.height = 121;
        assert!(value.validate().is_err());
        value = screen();
        value.cells.pop();
        assert!(value.validate().is_err());
        value = TerminalScreen {
            version: 1,
            columns: MAX_COLUMNS,
            height: MAX_ROWS,
            cells: vec![
                vec![TerminalCell::default(); usize::from(MAX_COLUMNS)];
                usize::from(MAX_ROWS)
            ],
            modes: TerminalModes::default(),
        };
        value.validate().unwrap();
        for row in &mut value.cells {
            for cell in row {
                cell.text = "a".repeat(64);
            }
        }
        assert!(value.validate().is_err());
    }
}
