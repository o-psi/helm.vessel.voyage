//! UTF-8 composer editing and prompt history, independent of the application.

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ImageMarker {
    pub(super) id: Uuid,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ComposerPart {
    Text(String),
    Image(Uuid),
}

/// Recovery accepts only canonical, ordered, disjoint owned markers. Literal
/// lookalikes outside these ranges are deliberately not interpreted as images.
pub(super) fn validate_markers(text: &str, markers: &[ImageMarker]) -> Result<(), String> {
    let mut end = 0;
    let mut ids = std::collections::HashSet::new();
    for (index, marker) in markers.iter().enumerate() {
        if marker.start < end
            || marker.start >= marker.end
            || text.get(marker.start..marker.end) != Some(format!("[Image {}]", index + 1).as_str())
            || marker.id.is_nil()
            || !ids.insert(marker.id)
        {
            return Err(format!("invalid image marker {}", index + 1));
        }
        end = marker.end;
    }
    Ok(())
}

#[derive(Clone, Default)]
pub(super) struct Composer {
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) markers: Vec<ImageMarker>,
    paste_anchor: Option<usize>,
    paste_end: Option<usize>,
    selection_anchor: Option<usize>,
    undo: Vec<EditState>,
    redo: Vec<EditState>,
    yank: String,
    pub(super) viewport_width: std::cell::Cell<u16>,
}

#[derive(Clone)]
struct EditState {
    text: String,
    cursor: usize,
    markers: Vec<ImageMarker>,
}

impl Composer {
    pub(super) fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.safe_offset(self.selection_anchor?);
        let cursor = self.safe_offset(self.cursor);
        (anchor != cursor).then_some((anchor.min(cursor), anchor.max(cursor)))
    }

    pub(super) fn insertion_len(&self, added: usize) -> usize {
        self.text.len() - self.selection().map_or(0, |(a, b)| b - a) + added
    }

    fn snapshot(&self) -> EditState {
        EditState {
            text: self.text.clone(),
            cursor: self.cursor,
            markers: self.markers.clone(),
        }
    }

    fn history_barrier(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        self.redo.clear();
        self.trim_history();
    }

    fn trim_history(&mut self) {
        // Bound both stacks together; no attachment bytes are duplicated.
        while self.undo.len() + self.redo.len() > 64
            || self
                .undo
                .iter()
                .chain(&self.redo)
                .map(|s| s.text.len())
                .sum::<usize>()
                > 1024 * 1024
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else {
                self.redo.remove(0);
            }
        }
    }

    fn restore_edit(&mut self, redo: bool) {
        let state = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        let Some(state) = state else {
            return;
        };
        let current = self.snapshot();
        if redo {
            self.undo.push(current);
        } else {
            self.redo.push(current);
        }
        self.trim_history();
        // An asynchronous clipboard result cannot safely target a replaced history state.
        self.clear_paste_anchor();
        self.selection_anchor = None;
        self.text = state.text;
        self.cursor = state.cursor;
        self.markers = state.markers;
    }

    fn word_move(&mut self, right: bool) {
        self.cursor = self.safe_offset(self.cursor);
        let mut offset = self.cursor;
        let mut found_word = false;
        if right {
            for (i, ch) in self.text[self.cursor..].char_indices() {
                if ch.is_whitespace() && found_word {
                    break;
                }
                found_word |= !ch.is_whitespace();
                offset = self.cursor + i + ch.len_utf8();
            }
        } else {
            for (i, ch) in self.text[..self.cursor].char_indices().rev() {
                if ch.is_whitespace() && found_word {
                    break;
                }
                found_word |= !ch.is_whitespace();
                offset = i;
            }
        }
        self.cursor = self.safe_offset(offset);
        if right
            && let Some(marker) = self
                .markers
                .iter()
                .find(|m| m.start < offset && offset < m.end)
        {
            self.cursor = marker.end;
        }
    }

    fn vertical_move(&mut self, down: bool) {
        let width = self.viewport_width.get().max(1);
        let (row, column) = cursor_position_at(&self.text, self.safe_offset(self.cursor), width);
        let target = if down {
            row.saturating_add(1)
        } else {
            row.saturating_sub(1)
        };
        if target == row {
            return;
        }
        let mut position = (0u16, 0u16);
        let mut best = None;
        for (offset, grapheme) in self
            .text
            .grapheme_indices(true)
            .chain(std::iter::once((self.text.len(), "")))
        {
            let size = grapheme.width() as u16;
            if grapheme != "\n" && position.1.saturating_add(size) > width {
                position = (position.0.saturating_add(1), 0);
            }
            let visible = if position.1 == width {
                (position.0.saturating_add(1), 0)
            } else {
                position
            };
            if visible.0 == target
                && !self
                    .markers
                    .iter()
                    .any(|m| m.start < offset && offset < m.end)
            {
                let distance = visible.1.abs_diff(column);
                if best.is_none_or(|(_, previous)| distance < previous) {
                    best = Some((offset, distance));
                }
            }
            if grapheme == "\n" {
                position = (position.0.saturating_add(1), 0);
            } else {
                position.1 = position.1.saturating_add(size);
            }
        }
        if let Some((offset, _)) = best {
            self.cursor = offset;
        }
    }

    /// Editing only: callers retain ownership of send, history, completion and private input.
    pub(super) fn edit_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode as K, KeyModifiers as M};
        let control = key.modifiers.contains(M::CONTROL);
        let shift = key.modifiers.contains(M::SHIFT);
        let alt = key.modifiers.contains(M::ALT);
        if control && matches!(key.code, K::Char('z' | 'Z' | 'y' | 'Y')) {
            self.restore_edit(matches!(key.code, K::Char('y' | 'Y' | 'Z')) || shift);
            return true;
        }
        if control && key.code == K::Char('a') {
            self.selection_anchor = Some(0);
            self.cursor = self.text.len();
            return true;
        }
        if control && key.code == K::Char('x') {
            if let Some((start, end)) = self.selection() {
                // Internal text yank deliberately excludes owned image markers.
                let mut at = start;
                self.yank.clear();
                for marker in self
                    .markers
                    .iter()
                    .filter(|m| start <= m.start && m.end <= end)
                {
                    self.yank.push_str(&self.text[at..marker.start]);
                    at = marker.end;
                }
                self.yank.push_str(&self.text[at..end]);
                self.insert_str("");
            }
            return true;
        }
        if alt && key.code == K::Char('y') {
            let text = self.yank.clone();
            if self.insertion_len(text.len()) <= 65536 {
                self.insert_str(&text);
            }
            return true;
        }
        if matches!(
            key.code,
            K::Left | K::Right | K::Up | K::Down | K::Home | K::End
        ) {
            if shift {
                let cursor = self.safe_offset(self.cursor);
                self.selection_anchor.get_or_insert(cursor);
            } else {
                self.selection_anchor = None;
            }
            match key.code {
                K::Left if control || alt => self.word_move(false),
                K::Right if control || alt => self.word_move(true),
                K::Left => self.move_left(),
                K::Right => self.move_right(),
                K::Up => self.vertical_move(false),
                K::Down => self.vertical_move(true),
                K::Home if control => self.cursor = 0,
                K::End if control => self.cursor = self.text.len(),
                K::Home => self.line_start(),
                K::End => self.line_end(),
                _ => {}
            }
            return true;
        }
        match key.code {
            K::Backspace | K::Delete if control || alt => {
                if self.selection().is_none() {
                    let cursor = self.safe_offset(self.cursor);
                    self.word_move(key.code == K::Delete);
                    self.selection_anchor = Some(self.cursor);
                    self.cursor = cursor;
                }
                self.insert_str("");
            }
            K::Backspace => self.backspace(),
            K::Delete => self.delete(),
            _ => return false,
        }
        true
    }

    // External cursor assignments are tolerated: clamp to UTF-8 and an atomic
    // boundary. Text/marker mutation must use these methods, not direct writes.
    fn safe_offset(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        for marker in &self.markers {
            if marker.start <= offset && offset <= marker.end {
                return if offset == marker.end {
                    marker.end
                } else {
                    marker.start
                };
            }
        }
        let start = self
            .markers
            .iter()
            .rev()
            .find(|m| m.end < offset)
            .map_or(0, |m| m.end);
        let end = self
            .markers
            .iter()
            .find(|m| m.start > offset)
            .map_or(self.text.len(), |m| m.start);
        let text = &self.text[start..end];
        let mut cursor =
            unicode_segmentation::GraphemeCursor::new(offset - start, text.len(), true);
        if !cursor.is_boundary(text, 0).unwrap_or(false) {
            offset = start + cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0);
        }
        offset
    }

    fn rebase(position: usize, start: usize, end: usize, len: usize, right: bool) -> usize {
        if position < start || (position == start && !right) {
            position
        } else if position >= end {
            start + len + (position - end)
        } else {
            start
        }
    }

    /// Replace a range whose boundaries do not split an owned marker.
    fn replace(&mut self, start: usize, end: usize, text: &str) {
        self.cursor = Self::rebase(self.cursor, start, end, text.len(), true);
        self.paste_end = self.paste_end.map(|p| {
            if start < end && start < p && p <= end {
                start
            } else {
                Self::rebase(p, start, end, text.len(), false)
            }
        });
        self.paste_anchor = self
            .paste_anchor
            .map(|p| Self::rebase(p, start, end, text.len(), false));
        self.markers.retain(|m| !(m.start < end && start < m.end));
        for marker in &mut self.markers {
            if marker.start >= end {
                marker.start = start + text.len() + (marker.start - end);
                marker.end = start + text.len() + (marker.end - end);
            }
        }
        self.text.replace_range(start..end, text);
    }

    fn renumber(&mut self) {
        for index in 0..self.markers.len() {
            let marker = self.markers[index].clone();
            let label = format!("[Image {}]", index + 1);
            if self.text[marker.start..marker.end] != label {
                // Renaming a surviving image is not deletion of the selected paste range.
                let paste_end = self.paste_end;
                self.replace(marker.start, marker.end, &label);
                self.paste_end = paste_end
                    .map(|p| Self::rebase(p, marker.start, marker.end, label.len(), false));
                self.markers.insert(
                    index,
                    ImageMarker {
                        end: marker.start + label.len(),
                        ..marker
                    },
                );
            }
        }
    }

    pub(super) fn insert(&mut self, character: char) {
        self.insert_str(character.encode_utf8(&mut [0; 4]));
    }

    pub(super) fn insert_str(&mut self, text: &str) {
        self.cursor = self.safe_offset(self.cursor);
        if text.is_empty() && self.selection().is_none() {
            self.selection_anchor = None;
            return;
        }
        self.checkpoint();
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        let removes_image = self.markers.iter().any(|m| m.start < end && start < m.end);
        self.selection_anchor = None;
        self.replace(start, end, text);
        self.cursor = start + text.len();
        self.renumber();
        if removes_image {
            self.history_barrier();
        }
    }

    /// Invalid/duplicate identities are ignored, preserving the existing image.
    pub(super) fn insert_image(&mut self, id: Uuid) {
        self.insert_image_at(self.cursor, id);
    }

    /// Offsets are byte offsets, clamped left to UTF-8/marker boundaries.
    /// The current cursor is rebased, not teleported to the inserted image.
    pub(super) fn insert_image_at(&mut self, offset: usize, id: Uuid) {
        if id.is_nil() || self.markers.iter().any(|m| m.id == id) {
            return;
        }
        self.history_barrier();
        self.selection_anchor = None;
        self.cursor = self.safe_offset(self.cursor);
        let start = self.safe_offset(offset);
        let index = self.markers.partition_point(|m| m.start < start);
        let label = format!("[Image {}]", index + 1);
        self.replace(start, start, &label);
        self.markers.insert(
            index,
            ImageMarker {
                id,
                start,
                end: start + label.len(),
            },
        );
        self.renumber();
    }

    pub(super) fn move_left(&mut self) {
        self.cursor = self.safe_offset(self.cursor);
        self.cursor = self
            .markers
            .iter()
            .find(|m| m.end == self.cursor)
            .map(|m| m.start)
            .unwrap_or_else(|| {
                let start = self
                    .markers
                    .iter()
                    .rev()
                    .find(|m| m.end < self.cursor)
                    .map_or(0, |m| m.end);
                self.text[start..self.cursor]
                    .grapheme_indices(true)
                    .next_back()
                    .map_or(start, |(i, _)| start + i)
            });
    }

    pub(super) fn move_right(&mut self) {
        self.cursor = self.safe_offset(self.cursor);
        self.cursor = self
            .markers
            .iter()
            .find(|m| m.start == self.cursor)
            .map(|m| m.end)
            .unwrap_or_else(|| {
                let end = self
                    .markers
                    .iter()
                    .find(|m| m.start > self.cursor)
                    .map_or(self.text.len(), |m| m.start);
                self.cursor
                    + self.text[self.cursor..end]
                        .graphemes(true)
                        .next()
                        .map_or(0, str::len)
            });
    }

    pub(super) fn backspace(&mut self) {
        if self.selection().is_some() {
            self.insert_str("");
            return;
        }
        let end = self.safe_offset(self.cursor);
        self.move_left();
        self.selection_anchor = Some(self.cursor);
        self.cursor = end;
        self.insert_str("");
    }

    pub(super) fn delete(&mut self) {
        if self.selection().is_some() {
            self.insert_str("");
            return;
        }
        let start = self.safe_offset(self.cursor);
        self.move_right();
        self.selection_anchor = Some(self.cursor);
        self.cursor = start;
        self.insert_str("");
    }

    pub(super) fn line_start(&mut self) {
        self.cursor = self.safe_offset(self.cursor);
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
    }

    pub(super) fn line_end(&mut self) {
        self.cursor = self.safe_offset(self.cursor);
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
    }

    pub(super) fn set_paste_anchor(&mut self) {
        self.cursor = self.safe_offset(self.cursor);
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.paste_anchor = Some(start);
        self.paste_end = Some(end);
    }

    #[cfg(test)]
    pub(super) fn take_paste_anchor(&mut self) -> Option<usize> {
        self.paste_end = None;
        self.paste_anchor.take()
    }
    pub(super) fn clear_paste_anchor(&mut self) {
        self.paste_anchor = None;
        self.paste_end = None;
    }

    pub(super) fn take_paste_range(&mut self) -> Option<(usize, usize)> {
        let start = self.paste_anchor.take()?;
        Some((start, self.paste_end.take().unwrap_or(start).max(start)))
    }

    pub(super) fn apply_paste_range(&mut self, start: usize, end: usize, text: &str) {
        self.checkpoint();
        self.selection_anchor = None;
        let removes_image = self.markers.iter().any(|m| m.start < end && start < m.end);
        self.replace(start, end, text);
        self.renumber();
        if removes_image {
            self.history_barrier();
        }
    }

    pub(super) fn ordered_parts(&self) -> Vec<ComposerPart> {
        let mut parts = Vec::new();
        let mut end = 0;
        for marker in &self.markers {
            if end < marker.start {
                parts.push(ComposerPart::Text(self.text[end..marker.start].to_owned()));
            }
            parts.push(ComposerPart::Image(marker.id));
            end = marker.end;
        }
        if end < self.text.len() {
            parts.push(ComposerPart::Text(self.text[end..].to_owned()));
        }
        parts
    }

    pub(super) fn authored_text(&self) -> String {
        self.ordered_parts()
            .into_iter()
            .filter_map(|p| match p {
                ComposerPart::Text(text) => Some(text),
                ComposerPart::Image(_) => None,
            })
            .collect()
    }

    /// On failure nothing changes. Call set_text first when restoring a draft.
    pub(super) fn restore_markers(&mut self, markers: Vec<ImageMarker>) -> Result<(), String> {
        validate_markers(&self.text, &markers)?;
        self.history_barrier();
        self.selection_anchor = None;
        self.markers = markers;
        self.cursor = self.safe_offset(self.cursor);
        self.clear_paste_anchor();
        Ok(())
    }

    pub(super) fn set_text(&mut self, text: String) {
        self.history_barrier();
        self.selection_anchor = None;
        self.text = text;
        self.cursor = self.text.len();
        self.markers.clear();
        self.clear_paste_anchor();
    }

    pub(super) fn take(&mut self) -> String {
        self.history_barrier();
        self.selection_anchor = None;
        self.yank.clear();
        self.cursor = 0;
        self.markers.clear();
        self.clear_paste_anchor();
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
        if self.entries.is_empty() || !composer.markers.is_empty() {
            return;
        }
        let position = if older {
            match self.position {
                Some(0) => return,
                Some(position) => position - 1,
                None => {
                    self.draft = composer.clone();
                    self.draft.markers.clear();
                    self.draft.clear_paste_anchor();
                    self.entries.len() - 1
                }
            }
        } else {
            match self.position {
                None => return,
                Some(position) if position + 1 == self.entries.len() => {
                    let draft = std::mem::take(&mut self.draft);
                    composer.set_text(draft.text);
                    composer.cursor = composer.safe_offset(draft.cursor);
                    self.position = None;
                    return;
                }
                Some(position) => position + 1,
            }
        };
        self.position = Some(position);
        composer.set_text(self.entries[position].clone());
    }
}

pub(super) fn cursor_position_at(text: &str, offset: usize, width: u16) -> (u16, u16) {
    let position = cursor_position(&text[..offset], width);
    let next = text[offset..].graphemes(true).next().unwrap_or("");
    if next != "\n" && position.1.saturating_add(next.width() as u16) > width.max(1) {
        (position.0.saturating_add(1), 0)
    } else {
        position
    }
}

pub(super) fn cursor_position(text: &str, width: u16) -> (u16, u16) {
    let width = width.max(1);
    let mut row = 0_u16;
    let mut column = 0_u16;
    for grapheme in text.graphemes(true) {
        if grapheme == "\n" {
            row = row.saturating_add(1);
            column = 0;
            continue;
        }
        let size = grapheme.width() as u16;
        if column.saturating_add(size) > width {
            row = row.saturating_add(1);
            column = 0;
        }
        column = column.saturating_add(size);
    }
    if column == width {
        (row.saturating_add(1), 0)
    } else {
        (row, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn valid(c: &Composer) {
        validate_markers(&c.text, &c.markers).unwrap();
        assert!(c.text.is_char_boundary(c.cursor));
        assert!(
            c.markers
                .iter()
                .all(|m| c.cursor <= m.start || c.cursor >= m.end)
        );
    }

    #[test]
    fn owned_markers_are_atomic_and_literals_remain_text() {
        let mut c = Composer::default();
        c.insert_str("[Image 1]é");
        c.insert_image(id(1));
        c.insert_str("尾");
        assert_eq!(c.authored_text(), "[Image 1]é尾");
        assert_eq!(
            c.ordered_parts(),
            vec![
                ComposerPart::Text("[Image 1]é".into()),
                ComposerPart::Image(id(1)),
                ComposerPart::Text("尾".into())
            ]
        );
        c.move_left();
        assert_eq!(c.cursor, c.markers[0].end);
        c.move_left();
        assert_eq!(c.cursor, c.markers[0].start);
        c.move_right();
        assert_eq!(c.cursor, c.markers[0].end);
        c.backspace();
        assert_eq!(c.text, "[Image 1]é尾");
        assert!(c.markers.is_empty());
        valid(&c);
    }

    #[test]
    fn insert_delete_and_renumber_across_digit_width() {
        let mut c = Composer::default();
        for n in 1..=12 {
            c.insert_image(id(n));
        }
        c.cursor = 0;
        c.insert_image(id(13));
        assert_eq!(c.markers[0].id, id(13));
        assert!(c.text.ends_with("[Image 13]"));
        for _ in 0..13 {
            c.cursor = 0;
            c.delete();
            valid(&c);
        }
        assert_eq!(c.text, "");
        c.backspace();
        c.delete();
        valid(&c);
    }

    #[test]
    fn async_anchor_is_left_biased_and_cursor_tracks_typing() {
        let mut c = Composer::default();
        c.insert_str("αω");
        c.move_left();
        c.set_paste_anchor();
        c.insert_str("typed");
        let anchor = c.take_paste_anchor().unwrap();
        assert_eq!(anchor, "α".len());
        c.insert_image_at(anchor, id(1));
        assert_eq!(c.text, "α[Image 1]typedω");
        assert_eq!(&c.text[..c.cursor], "α[Image 1]typed");
        assert_eq!(c.take_paste_anchor(), None);
        c.cursor = 0;
        c.insert_image_at(c.text.len(), id(2));
        assert_eq!(c.cursor, 0);
        valid(&c);
    }

    #[test]
    fn anchors_rebase_on_insertion_deletion_and_renumbering() {
        let mut c = Composer::default();
        c.insert_str("abc");
        c.cursor = 2;
        c.set_paste_anchor();
        c.replace(1, 3, "");
        assert_eq!(c.take_paste_anchor(), Some(1));
        c.set_text(String::new());
        for n in 1..=10 {
            c.insert_image(id(n));
        }
        c.set_paste_anchor();
        c.cursor = 0;
        c.delete();
        assert_eq!(c.take_paste_anchor(), Some(c.text.len()));
        c.cursor = 0;
        c.set_paste_anchor();
        c.insert_image(id(11));
        assert_eq!(c.take_paste_anchor(), Some(0));
        valid(&c);
    }

    #[test]
    fn utf8_and_graphemes_are_atomic_around_owned_markers() {
        let mut c = Composer::default();
        c.insert_str("é👩\u{200d}💻e\u{301}");
        c.backspace(); // The combining sequence is one editing unit.
        assert!(c.text.ends_with("👩\u{200d}💻"));
        c.move_left();
        c.insert_image(id(1));
        valid(&c);
        c.move_left();
        c.delete();
        assert_eq!(c.text, "é👩\u{200d}💻");
        c.cursor = 1; // Even a stale offset within é is normalized safely.
        c.insert_image(id(2));
        assert!(c.text.starts_with("[Image 1]é"));
        c.cursor = 3; // A stale cursor inside a marker cannot split it.
        c.insert('x');
        assert!(c.text.starts_with("x[Image 1]"));
        valid(&c);
        assert_eq!(cursor_position("e\u{301}", 10), (0, 1));
    }

    #[test]
    fn line_navigation_and_clear_operations() {
        let mut c = Composer::default();
        c.insert_str("first\n");
        c.insert_image(id(1));
        c.insert_str("last\nend");
        c.cursor = c.markers[0].start + 2;
        c.line_end();
        assert_eq!(&c.text[..c.cursor], "first\n[Image 1]last");
        c.line_start();
        assert_eq!(c.cursor, 6);
        c.set_paste_anchor();
        assert_eq!(c.take(), "first\n[Image 1]last\nend");
        assert!(c.markers.is_empty());
        assert_eq!(c.take_paste_anchor(), None);
        c.insert_image(id(1));
        c.set_paste_anchor();
        c.set_text("[Image 1]".into());
        assert!(c.markers.is_empty());
        assert_eq!(c.authored_text(), "[Image 1]");
        assert_eq!(c.take_paste_anchor(), None);
    }

    #[test]
    fn recovery_rejects_corruption_without_mutation() {
        let mut c = Composer::default();
        c.insert_str("é");
        c.insert_image(id(1));
        c.insert_image(id(2));
        let good = c.markers.clone();
        let mut cases = Vec::new();
        for (start, end) in [(1, 10), (2, usize::MAX), (10, 2), (2, 2), (0, 10)] {
            let mut bad = good.clone();
            bad[0].start = start;
            bad[0].end = end;
            cases.push(bad);
        }
        let mut bad = good.clone();
        bad[1].id = bad[0].id;
        cases.push(bad);
        let mut bad = good.clone();
        bad[0].id = Uuid::nil();
        cases.push(bad);
        let mut bad = good.clone();
        bad.reverse();
        cases.push(bad);
        let mut bad = good.clone();
        bad[1].start = good[0].start;
        cases.push(bad);
        for bad in cases {
            assert!(c.restore_markers(bad).is_err());
            assert_eq!(c.markers, good);
        }
        assert!(validate_markers("[Image 9]", &good[..1]).is_err());
        c.set_paste_anchor();
        c.restore_markers(good).unwrap();
        assert_eq!(c.take_paste_anchor(), None);
        let before = c.text.clone();
        c.insert_image(id(1));
        c.insert_image(Uuid::nil());
        assert_eq!(c.text, before);
    }

    #[test]
    fn history_does_not_drop_or_resurrect_image_ownership_or_anchors() {
        let mut h = PromptHistory::default();
        h.record("old [Image 1]");
        let mut c = Composer::default();
        c.insert_image(id(1));
        c.set_paste_anchor();
        h.navigate(&mut c, true);
        assert_eq!(c.markers.len(), 1);
        assert_eq!(h.position, None);
        c.set_text("draft".into());
        c.cursor = 2;
        c.set_paste_anchor();
        h.navigate(&mut c, true);
        assert_eq!(c.authored_text(), "old [Image 1]");
        assert_eq!(c.take_paste_anchor(), None);
        h.navigate(&mut c, false);
        assert_eq!(c.text, "draft");
        assert_eq!(c.cursor, 2);
        assert!(c.markers.is_empty());
        assert_eq!(c.take_paste_anchor(), None);
        h.navigate(&mut c, true);
        c.insert_image(id(2));
        h.navigate(&mut c, false);
        assert_eq!(c.markers.len(), 1);
        assert_eq!(h.position, Some(0));
    }
}
