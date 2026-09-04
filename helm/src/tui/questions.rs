//! Question dialog state, answer editing and terminal-safe rendering.

use super::{bridge::QuestionRequest, composer::Composer, text::display_safe};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

pub(super) struct QuestionDialog {
    pub(super) request: QuestionRequest,
    pub(super) selected: usize,
    pub(super) custom: Composer,
    pub(super) scroll: Option<u16>,
}

impl QuestionDialog {
    pub(super) fn insert(&mut self, text: &str) {
        if self.selected != self.request.question.options.len() {
            return;
        }
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            if self.custom.text.len() + ch.len_utf8() > crate::tools::MAX_ANSWER_BYTES {
                break;
            }
            self.custom.insert(ch);
        }
        self.scroll = None;
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Option<crate::tools::QuestionAnswer> {
        use crate::tools::QuestionAnswer;
        let count = self.request.question.options.len();
        match key.code {
            KeyCode::Esc => return Some(QuestionAnswer::Cancelled),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(QuestionAnswer::Cancelled);
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = None;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1) % (count + 1);
                self.scroll = None;
            }
            KeyCode::PageUp => self.scroll = Some(self.scroll.unwrap_or(0).saturating_sub(5)),
            KeyCode::PageDown => self.scroll = Some(self.scroll.unwrap_or(0).saturating_add(5)),
            KeyCode::Enter if self.selected < count => {
                return Some(QuestionAnswer::Selected {
                    index: self.selected,
                    answer: self.request.question.options[self.selected].clone(),
                });
            }
            KeyCode::Enter if !self.custom.text.trim().is_empty() => {
                return Some(QuestionAnswer::Custom {
                    answer: self.custom.text.clone(),
                });
            }
            KeyCode::Backspace if self.selected == count => self.custom.backspace(),
            KeyCode::Delete if self.selected == count => self.custom.delete(),
            KeyCode::Home if self.selected == count => self.custom.line_start(),
            KeyCode::End if self.selected == count => self.custom.line_end(),
            KeyCode::Left if self.selected == count => {
                self.custom.cursor = self.custom.text[..self.custom.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(i, _)| i);
            }
            KeyCode::Right if self.selected == count => {
                if let Some(ch) = self.custom.text[self.custom.cursor..].chars().next() {
                    self.custom.cursor += ch.len_utf8();
                }
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&ch.to_string())
            }
            _ => {}
        }
        None
    }
}

// Wrap as plain terminal-safe text, never interpret model options as Markdown.
pub(super) fn question_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let size = grapheme.width();
        if used + size > width.max(1) && !current.is_empty() {
            lines.push(Line::from(std::mem::take(&mut current)));
            used = 0;
        }
        current.push_str(grapheme);
        used += size;
    }
    lines.push(Line::from(current));
    lines
}

const QUESTION_HELP: &str = "↑/↓/Tab choose · Enter submit · Esc cancel · PgUp/PgDn scroll. Answers are saved; do not enter secrets.";

// Measurement and painting share the same wrapping, including the custom cursor.
fn question_content(dialog: &QuestionDialog, width: usize) -> (Vec<Line<'static>>, usize) {
    let mut lines = question_lines(&display_safe(&dialog.request.question.question), width);
    lines.push(Line::from(""));
    let mut selected_line = 0;
    for (index, option) in dialog
        .request
        .question
        .options
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("Other / custom answer"))
        .enumerate()
    {
        let selected = index == dialog.selected;
        if selected {
            selected_line = lines.len();
        }
        let prefix = if selected { "▶" } else { " " };
        let mut option_lines = question_lines(
            &format!("{prefix} {}. {}", index + 1, display_safe(option)),
            width,
        );
        if selected {
            for line in &mut option_lines {
                *line = line.clone().style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
        lines.extend(option_lines);
    }
    if dialog.selected == dialog.request.question.options.len() {
        lines.push(Line::from(""));
        let (before, after) = dialog.custom.text.split_at(dialog.custom.cursor);
        let input = format!("Answer: {}▏{}", display_safe(before), display_safe(after));
        let cursor_line = question_lines(&format!("Answer: {}▏", display_safe(before)), width)
            .len()
            .saturating_sub(1);
        selected_line = lines.len() + cursor_line;
        lines.extend(question_lines(&input, width));
    }
    (lines, selected_line)
}

pub(super) fn question_height(dialog: &QuestionDialog, width: u16) -> u16 {
    let (lines, _) = question_content(dialog, width.saturating_sub(2).max(1) as usize);
    let help = question_lines(QUESTION_HELP, width.max(1) as usize);
    (lines.len() + 2 + help.len()).min(u16::MAX as usize) as u16
}

pub(super) fn draw_question(frame: &mut ratatui::Frame<'_>, area: Rect, dialog: &QuestionDialog) {
    let help = question_lines(QUESTION_HELP, area.width.max(1) as usize);
    // On short terminals keep at least one body row inside the border. The
    // title still identifies sharing when the full help cannot fit.
    let help_height = (help.len().min(u16::MAX as usize) as u16).min(area.height.saturating_sub(3));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(help_height)])
        .split(area);
    let width = chunks[0].width.saturating_sub(2).max(1) as usize;
    let height = chunks[0].height.saturating_sub(2).max(1) as usize;
    let (lines, selected_line) = question_content(dialog, width);
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = dialog
        .scroll
        .map(usize::from)
        .unwrap_or_else(|| selected_line.saturating_sub(height.saturating_sub(2)))
        .min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll.min(u16::MAX as usize) as u16, 0))
            .block(
                Block::default()
                    .title(" Question · shared with model ")
                    .borders(Borders::ALL),
            ),
        chunks[0],
    );
    frame.render_widget(Paragraph::new(help), chunks[1]);
}
