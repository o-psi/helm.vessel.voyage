//! UTF-8 composer editing and prompt history, independent of the application.

use unicode_width::UnicodeWidthChar;

#[derive(Clone, Default)]
pub(super) struct Composer {
    pub(super) text: String,
    pub(super) cursor: usize,
}

impl Composer {
    pub(super) fn insert(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    pub(super) fn insert_str(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
    }

    pub(super) fn delete(&mut self) {
        if let Some(character) = self.text[self.cursor..].chars().next() {
            self.text
                .drain(self.cursor..self.cursor + character.len_utf8());
        }
    }

    pub(super) fn line_start(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
    }

    pub(super) fn line_end(&mut self) {
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
    }

    pub(super) fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

/// Independent of model context so compaction does not erase recalled prompts.
#[derive(Default)]
pub(super) struct PromptHistory {
    pub(super) entries: Vec<String>,
    pub(super) position: Option<usize>,
    pub(super) draft: Composer,
}

impl PromptHistory {
    pub(super) fn reset_navigation(&mut self) {
        self.position = None;
        self.draft = Composer::default();
    }

    pub(super) fn record(&mut self, prompt: &str) {
        self.reset_navigation();
        if !prompt.trim().is_empty() && prompt.len() <= 65536 {
            if self
                .entries
                .last()
                .is_none_or(|previous| previous != prompt)
            {
                self.entries.push(prompt.to_owned());
            }
            while self.entries.len() > 256
                || self.entries.iter().map(String::len).sum::<usize>() > 1024 * 1024
            {
                self.entries.remove(0);
            }
        }
    }

    pub(super) fn navigate(&mut self, composer: &mut Composer, older: bool) {
        if self.entries.is_empty() {
            return;
        }
        let position = if older {
            match self.position {
                Some(0) => return,
                Some(position) => position - 1,
                None => {
                    self.draft = composer.clone();
                    self.entries.len() - 1
                }
            }
        } else {
            match self.position {
                None => return,
                Some(position) if position + 1 == self.entries.len() => {
                    *composer = std::mem::take(&mut self.draft);
                    self.position = None;
                    return;
                }
                Some(position) => position + 1,
            }
        };
        self.position = Some(position);
        composer.text = self.entries[position].clone();
        composer.cursor = composer.text.len();
    }
}

pub(super) fn cursor_position(text: &str, width: u16) -> (u16, u16) {
    let width = width.max(1);
    let mut row = 0_u16;
    let mut column = 0_u16;
    for character in text.chars() {
        if character == '\n' {
            row = row.saturating_add(1);
            column = 0;
            continue;
        }
        let character_width = character.width().unwrap_or(0) as u16;
        if column.saturating_add(character_width) > width {
            row = row.saturating_add(1);
            column = 0;
        }
        column = column.saturating_add(character_width);
        if column == width {
            row = row.saturating_add(1);
            column = 0;
        }
    }
    (row, column)
}
