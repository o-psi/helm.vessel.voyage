//! Keyboard-first voyage scope editor. Discovery and execution belong to the caller.
use super::{composer::Composer, text::display_safe};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(super) struct HelmChoice {
    pub(super) id: Uuid,
    pub(super) label: String,
    pub(super) availability: String,
    pub(super) can_execute: bool,
}

pub(super) use crate::voyage::Draft;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Name,
    Purpose,
    Helms,
    Coordinator,
    Review,
}

#[derive(Default)]
pub(super) struct Panel {
    open: bool,
    step: Step,
    name: Composer,
    purpose: Composer,
    search: Composer,
    helms: Vec<HelmChoice>,
    selected: Vec<Uuid>,
    coordinator: Option<Uuid>,
    focused: Option<Uuid>,
    discovery_error: Option<String>,
    error: Option<String>,
    detail: bool,
    scroll: u16,
    ready: Option<Draft>,
}

impl Panel {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn open(&mut self) {
        self.open = true;
        self.ready = None;
    }
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
    /// Keep selected identities, including temporarily missing Helms. Never silently alter scope.
    pub(super) fn set_helms(&mut self, mut helms: Vec<HelmChoice>, error: Option<String>) {
        let mut seen = std::collections::HashSet::new();
        helms.retain(|h| seen.insert(h.id));
        self.helms = helms;
        self.discovery_error = error;
        self.reconcile_focus();
    }
    /// Restore a saved draft without depending on live discovery order.
    pub(super) fn set_draft(&mut self, draft: Draft) {
        self.name = Composer {
            cursor: draft.name.len(),
            text: draft.name,
        };
        self.purpose = Composer {
            cursor: draft.purpose.len(),
            text: draft.purpose,
        };
        self.selected = draft.participants;
        self.coordinator = draft.coordinator;
        self.step = Step::Name;
        self.detail = false;
        self.error = None;
        self.search = Composer::default();
        self.scroll = 0;
        self.reconcile_focus();
    }
    pub(super) fn take_ready(&mut self) -> Option<Draft> {
        self.ready.take()
    }
    fn visible(&self) -> Vec<Uuid> {
        let query = self.search.text.to_lowercase();
        self.helms
            .iter()
            .filter(|h| {
                if self.step == Step::Coordinator {
                    self.selected.contains(&h.id)
                } else {
                    display_safe(&h.label).to_lowercase().contains(&query)
                        || h.id.to_string().contains(&query)
                }
            })
            .map(|h| h.id)
            .collect()
    }
    fn reconcile_focus(&mut self) {
        let visible = self.visible();
        if !self.focused.is_some_and(|id| visible.contains(&id)) {
            self.focused = visible.first().copied();
        }
    }
    fn editor(&mut self) -> Option<(&mut Composer, usize)> {
        match self.step {
            Step::Name => Some((&mut self.name, 256)),
            Step::Purpose => Some((&mut self.purpose, 8192)),
            Step::Helms => Some((&mut self.search, 256)),
            _ => None,
        }
    }
    pub(super) fn paste(&mut self, text: &str) {
        if !self.open || self.detail {
            return;
        }
        if let Some((editor, limit)) = self.editor() {
            for ch in text.chars().filter(|c| !c.is_control()) {
                if editor.text.len() + ch.len_utf8() > limit {
                    break;
                }
                editor.insert(ch);
            }
        }
        self.error = None;
        self.reconcile_focus();
    }
    pub(super) fn draft(&self) -> Draft {
        Draft {
            name: self.name.text.trim().to_owned(),
            purpose: self.purpose.text.trim().to_owned(),
            participants: self.selected.clone(),
            coordinator: self.coordinator,
        }
    }
    fn validate(&self) -> Result<(), &'static str> {
        if self.name.text.len() > 256 || self.purpose.text.len() > 8192 || self.selected.len() > 64
        {
            return Err("Draft exceeds the name, purpose, or Helm selection limit.");
        }
        if self.name.text.trim().is_empty() {
            return Err("Give this voyage a name.");
        }
        if self.selected.is_empty() {
            return Err("Select at least one Helm.");
        }
        if !self
            .coordinator
            .is_some_and(|id| self.selected.contains(&id))
        {
            return Err("Choose a coordinator from the selected Helms.");
        }
        Ok(())
    }
    fn advance(&mut self) {
        self.error = None;
        self.step = match self.step {
            Step::Name if self.name.text.trim().is_empty() => {
                self.error = Some("Give this voyage a name.".into());
                return;
            }
            Step::Name => Step::Purpose,
            Step::Purpose => Step::Helms,
            Step::Helms if self.selected.is_empty() => {
                self.error = Some("Select at least one Helm using Space.".into());
                return;
            }
            Step::Helms => Step::Coordinator,
            Step::Coordinator | Step::Review => {
                if let Err(error) = self.validate() {
                    self.error = Some(error.into());
                    return;
                }
                if self.step == Step::Review {
                    self.ready = Some(self.draft());
                    self.open = false;
                }
                Step::Review
            }
        };
        self.scroll = 0;
        self.reconcile_focus();
    }
    pub(super) fn key(&mut self, key: KeyEvent) {
        if !self.open
            || key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat
                && matches!(key.code, KeyCode::Enter | KeyCode::Tab | KeyCode::Char(' ')))
        {
            return;
        }
        if self.detail {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::F(2) => {
                    self.detail = false;
                    self.scroll = 0;
                }
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.detail = false;
                    self.open = false;
                }
                _ => {}
            }
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.open = false;
            }
            KeyCode::Char('c') if control => {
                self.open = false;
            }
            KeyCode::BackTab => {
                self.step = match self.step {
                    Step::Name => Step::Name,
                    Step::Purpose => Step::Name,
                    Step::Helms => Step::Purpose,
                    Step::Coordinator => Step::Helms,
                    Step::Review => Step::Coordinator,
                };
                self.error = None;
                self.scroll = 0;
                self.reconcile_focus();
            }
            KeyCode::Enter => self.advance(),
            KeyCode::Tab if self.step != Step::Review => self.advance(),
            KeyCode::F(2) if matches!(self.step, Step::Helms | Step::Coordinator) => {
                self.detail = self.focused.is_some();
            }
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::Up | KeyCode::Down if matches!(self.step, Step::Helms | Step::Coordinator) => {
                let visible = self.visible();
                if !visible.is_empty() {
                    let position = self
                        .focused
                        .and_then(|id| visible.iter().position(|v| *v == id))
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Up {
                        position.saturating_sub(1)
                    } else {
                        (position + 1).min(visible.len() - 1)
                    };
                    self.focused = Some(visible[next]);
                    self.scroll = 0;
                }
            }
            KeyCode::Char(' ') if matches!(self.step, Step::Helms | Step::Coordinator) => {
                if let Some(id) = self.focused {
                    if self.step == Step::Coordinator {
                        self.coordinator = Some(id);
                    } else if self.selected.contains(&id) {
                        self.selected.retain(|v| *v != id);
                        if self.coordinator == Some(id) {
                            self.coordinator = None;
                        }
                    } else {
                        if self.selected.len() >= 64 {
                            self.error = Some("Select at most 64 Helms.".into());
                            return;
                        }
                        self.selected.push(id);
                    }
                    self.error = None;
                }
            }
            KeyCode::Char('u') if control && self.step == Step::Helms => {
                // Explicitly remove unavailable discovery entries from the draft.
                self.selected
                    .retain(|id| self.helms.iter().any(|h| h.id == *id));
                if !self
                    .coordinator
                    .is_some_and(|id| self.selected.contains(&id))
                {
                    self.coordinator = None;
                }
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.paste(&ch.to_string())
            }
            code => {
                if let Some((editor, _)) = self.editor() {
                    match code {
                        KeyCode::Backspace => editor.backspace(),
                        KeyCode::Delete => editor.delete(),
                        KeyCode::Home => editor.line_start(),
                        KeyCode::End => editor.line_end(),
                        KeyCode::Left => {
                            editor.cursor = editor.text[..editor.cursor]
                                .char_indices()
                                .next_back()
                                .map_or(0, |(i, _)| i)
                        }
                        KeyCode::Right => {
                            if let Some(ch) = editor.text[editor.cursor..].chars().next() {
                                editor.cursor += ch.len_utf8();
                            }
                        }
                        _ => {}
                    }
                }
                self.reconcile_focus();
            }
        }
    }
    fn helm_label(&self, id: Uuid) -> String {
        self.helms.iter().find(|h| h.id == id).map_or_else(
            || format!("{id} (missing from discovery)"),
            |h| {
                format!(
                    "{} ({}) · {}{}",
                    display_safe(&h.label),
                    &id.to_string()[..8],
                    display_safe(&h.availability),
                    if h.can_execute {
                        ""
                    } else {
                        " · cannot execute"
                    }
                )
            },
        )
    }
    pub(super) fn draw(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.open {
            return;
        }
        frame.render_widget(Clear, area);
        let title = match self.step {
            Step::Name => "1/5 · Name",
            Step::Purpose => "2/5 · Purpose",
            Step::Helms => "3/5 · Select Helms",
            Step::Coordinator => "4/5 · Coordinator",
            Step::Review => "5/5 · Review draft",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" New voyage · {title} "));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let footer_height = 3.min(inner.height);
        let body = Rect {
            height: inner.height.saturating_sub(footer_height),
            ..inner
        };
        let footer = Rect {
            y: inner.y + body.height,
            height: footer_height,
            ..inner
        };
        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.detail {
            if let Some(id) = self.focused {
                lines.push(self.helm_label(id).into());
                lines.push(format!("Helm ID: {id}").into());
            }
            lines.push(
                "Each executing Helm retains its local permissions and provider credentials."
                    .into(),
            );
            lines.push(
                "Availability describes discovery status; it does not grant execution authority."
                    .into(),
            );
        } else {
            match self.step {
                Step::Name | Step::Purpose => {
                    let (label, editor) = if self.step == Step::Name {
                        ("Name (required, up to 256 bytes)", &self.name)
                    } else {
                        ("Purpose (optional, up to 8192 bytes)", &self.purpose)
                    };
                    lines.push(label.into());
                    lines.push(
                        format!(
                            "{}▏{}",
                            display_safe(&editor.text[..editor.cursor]),
                            display_safe(&editor.text[editor.cursor..])
                        )
                        .into(),
                    );
                    lines.push("".into());
                    lines.push(
                        "A voyage scopes an open-ended conversation to Helms you choose.".into(),
                    );
                }
                Step::Helms | Step::Coordinator => {
                    if self.step == Step::Helms {
                        lines.push(
                            format!(
                                "Search: {}▏ · {} selected",
                                display_safe(&self.search.text),
                                self.selected.len()
                            )
                            .into(),
                        );
                    } else {
                        lines.push("Space chooses one selected Helm as coordinator.".into());
                    }
                    if let Some(error) = &self.discovery_error {
                        lines.push(Line::styled(
                            format!("Discovery: {}", display_safe(error)),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                    let visible = self.visible();
                    let start = self
                        .focused
                        .and_then(|id| visible.iter().position(|v| *v == id))
                        .unwrap_or(0)
                        .saturating_sub(body.height.saturating_sub(5) as usize);
                    if visible.is_empty() {
                        lines.push("No matching Helms. Clear search or refresh discovery.".into());
                    }
                    for id in visible.into_iter().skip(start) {
                        let selected = if self.step == Step::Coordinator {
                            self.coordinator == Some(id)
                        } else {
                            self.selected.contains(&id)
                        };
                        let line = format!(
                            "{} [{}] {}",
                            if self.focused == Some(id) { ">" } else { " " },
                            if selected { "x" } else { " " },
                            self.helm_label(id)
                        );
                        lines.push(Line::styled(
                            truncate(&line, body.width as usize),
                            if self.focused == Some(id) {
                                Style::default().fg(Color::Cyan)
                            } else {
                                Style::default()
                            },
                        ));
                    }
                    let missing = self
                        .selected
                        .iter()
                        .filter(|id| !self.helms.iter().any(|h| h.id == **id))
                        .count();
                    if missing > 0 {
                        lines.insert(1, format!("{missing} selected Helm(s) missing; Ctrl+U removes them explicitly.").into());
                    }
                }
                Step::Review => {
                    lines.push(format!("Name: {}", display_safe(self.name.text.trim())).into());
                    lines.push(
                        format!("Purpose: {}", display_safe(self.purpose.text.trim())).into(),
                    );
                    lines.push("Selected scope:".into());
                    for id in &self.selected {
                        lines.push(
                            format!(
                                "  {}{}",
                                self.helm_label(*id),
                                if self.coordinator == Some(*id) {
                                    " · coordinator"
                                } else {
                                    ""
                                }
                            )
                            .into(),
                        );
                    }
                    lines.push("".into());
                    lines.push("The interface Helm is included only if selected. Coordinator and participant roles may overlap.".into());
                    lines.push("Save a voyage draft. This does not start tasks, dispatch remote work, or transfer credentials. Multi-Helm orchestration remains planned.".into());
                }
            }
        }
        let wrapped: Vec<Line<'static>> = lines
            .into_iter()
            .flat_map(|line| {
                let style = line.style;
                super::questions::question_lines(&line.to_string(), body.width as usize)
                    .into_iter()
                    .map(move |line| line.style(style))
            })
            .collect();
        let maximum = wrapped
            .len()
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16;
        let mut scroll = self.scroll.min(maximum);
        if matches!(self.step, Step::Name | Step::Purpose) && !self.detail {
            let editor = if self.step == Step::Name {
                &self.name
            } else {
                &self.purpose
            };
            let label = if self.step == Step::Name {
                "Name (required, up to 256 bytes)"
            } else {
                "Purpose (optional, up to 8192 bytes)"
            };
            let cursor_row = super::questions::question_lines(
                &format!("{}▏", display_safe(&editor.text[..editor.cursor])),
                body.width as usize,
            )
            .len()
                + super::questions::question_lines(label, body.width as usize).len();
            scroll = cursor_row
                .saturating_sub(body.height as usize)
                .min(maximum as usize) as u16;
        }
        frame.render_widget(Paragraph::new(wrapped).scroll((scroll, 0)), body);
        let help = if self.detail {
            "Esc/Enter close details"
        } else if self.step == Step::Review {
            "Enter save draft · Shift+Tab back · Esc cancel"
        } else {
            "Enter next · Shift+Tab back · Esc cancel"
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(self.error.clone().unwrap_or_default()),
                Line::from(help),
                Line::from("↑↓ move · Space select · F2 details · PgUp/PgDn scroll"),
            ])
            .wrap(Wrap { trim: false }),
            footer,
        );
    }
}

/// Keep picker rows to one screen row; F2 exposes their complete text.
fn truncate(text: &str, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        if used + grapheme.width() > width.saturating_sub(1) {
            break;
        }
        used += grapheme.width();
        out.push_str(grapheme);
    }
    if width > 0 {
        out.push('…');
    }
    out
}
