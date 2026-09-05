//! Voyage draft library and transient authenticated discovery. No execution path.
use super::{
    composer::Composer,
    text::display_safe,
    voyages::{HelmChoice, Panel},
};
use crate::{
    voyage::{Draft, Store, StoredDraft},
    voyage_client::{Client, HelmPresence},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};
type DiscoveryResult = (Uuid, String, Result<Vec<HelmPresence>, String>);

#[derive(Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Closed,
    Library,
    Connect,
    Form,
}

pub(super) struct Hub {
    mode: Mode,
    pub(super) form: Panel,
    path: PathBuf,
    drafts: Vec<StoredDraft>,
    selected: usize,
    editing: Option<StoredDraft>,
    choices: Vec<HelmChoice>,
    form_choices: Vec<HelmChoice>,
    form_origin: Option<String>,
    form_started: bool,
    replacement: Option<Option<StoredDraft>>,
    save_failed: bool,
    origin: Composer,
    token: Zeroizing<String>,
    token_focus: bool,
    client: Option<Arc<Client>>,
    connected_origin: Option<String>,
    discovery: Option<tokio::task::JoinHandle<()>>,
    request: Option<Uuid>,
    rx: mpsc::UnboundedReceiver<DiscoveryResult>,
    tx: mpsc::UnboundedSender<DiscoveryResult>,
    status: String,
}
impl Default for Hub {
    fn default() -> Self {
        Self::new(crate::voyage::default_path())
    }
}
impl Drop for Hub {
    fn drop(&mut self) {
        self.stop_discovery();
    }
}
impl Hub {
    pub(super) fn new(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            mode: Mode::Closed,
            form: Panel::new(),
            path,
            drafts: vec![],
            selected: 0,
            editing: None,
            choices: vec![],
            form_choices: vec![],
            form_origin: None,
            form_started: false,
            replacement: None,
            save_failed: false,
            origin: Composer::default(),
            token: Zeroizing::new(String::new()),
            token_focus: false,
            client: None,
            connected_origin: None,
            discovery: None,
            request: None,
            rx,
            tx,
            status: String::new(),
        }
    }
    pub(super) fn is_open(&self) -> bool {
        self.mode != Mode::Closed
    }
    pub(super) fn close(&mut self) {
        self.mode = Mode::Closed;
        self.replacement = None;
        self.token.zeroize();
        self.stop_discovery();
    }
    fn stop_discovery(&mut self) {
        self.request = None;
        if let Some(task) = self.discovery.take() {
            task.abort();
        }
        while self.rx.try_recv().is_ok() {}
    }
    pub(super) fn open(&mut self) {
        self.mode = Mode::Library;
        match Store::new(self.path.clone()).and_then(|store| store.list()) {
            Ok(drafts) => { self.drafts = drafts; self.selected = self.selected.min(self.drafts.len().saturating_sub(1)); self.status = "Drafts stay on this computer. The voyage runtime is not available yet.".into(); },
            Err(_) => self.status = "Cannot read saved voyage drafts. Check private storage permissions; existing drafts are preserved.".into(),
        }
        if self.choices.is_empty() {
            match crate::attachment::local_actor::LocalActorStore::open(
                &self.path.join("interface"),
            )
            .and_then(|local| local.identity())
            {
                Ok(local) => self.choices.push(HelmChoice {
                    id: local.installation_id,
                    label: "This Helm · local interface".into(),
                    availability: "Local setup identity · not an enrolled remote Helm".into(),
                    can_execute: false,
                }),
                Err(_) => self.status =
                    "Local setup identity unavailable. Connect to Vessel to select remote Helms."
                        .into(),
            }
        }
    }
    fn request_form(&mut self, saved: Option<StoredDraft>) {
        let original = self
            .editing
            .as_ref()
            .map(|s| s.draft.clone())
            .unwrap_or_default();
        if self.form_started && (self.save_failed || self.form.draft() != original) {
            self.replacement = Some(saved);
            self.status = "Discard unfinished setup? Enter discards and opens the selected draft; Esc keeps your edits.".into();
        } else {
            self.start_form(saved);
        }
    }
    fn start_form(&mut self, saved: Option<StoredDraft>) {
        self.form_origin = saved
            .as_ref()
            .map(|saved| saved.origin.clone())
            .unwrap_or_else(|| self.connected_origin.clone());
        self.form_started = true;
        self.save_failed = false;
        self.editing = saved;
        let draft = self
            .editing
            .as_ref()
            .map(|saved| saved.draft.clone())
            .unwrap_or_default();
        let mut choices = self.choices.clone();
        if let Some(saved) = &self.editing {
            // A different Vessel must not silently reinterpret saved machine IDs.
            if saved.origin != self.connected_origin {
                choices.retain(|c| c.label == "This Helm · local interface");
            }
            for id in &saved.draft.participants {
                if !choices.iter().any(|choice| choice.id == *id) {
                    choices.push(HelmChoice {
                        id: *id,
                        label: saved
                            .helm_labels
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| id.to_string()),
                        availability:
                            "Not currently observed · reconnect to the saved Vessel to refresh"
                                .into(),
                        can_execute: false,
                    });
                }
            }
        }
        self.form.set_draft(draft);
        self.form_choices = choices.clone();
        self.form.set_helms(choices, None);
        self.form.open();
        self.mode = Mode::Form;
    }
    pub(super) fn poll(&mut self) {
        while let Ok((request, origin, result)) = self.rx.try_recv() {
            if self.request != Some(request) {
                continue;
            }
            self.request = None;
            self.discovery = None;
            match result {
                Ok(helms) => {
                    let remote_count = helms.len();
                    self.choices
                        .retain(|c| c.label == "This Helm · local interface");
                    for helm in helms {
                        self.choices.push(HelmChoice {
                            id: helm.machine_id,
                            label: format!("Helm {}", helm.machine_id),
                            availability: format!(
                                "Connected to Vessel · enrollment epoch {} · runtime not available",
                                helm.epoch
                            ),
                            can_execute: false,
                        });
                    }
                    self.status = format!(
                        "Connected · {} remote Helms observed. Presence does not authorize execution.",
                        remote_count
                    );
                    self.connected_origin = Some(origin);
                    if self.mode == Mode::Connect {
                        self.mode = Mode::Library;
                    }
                }
                Err(error) => {
                    self.client = None;
                    for choice in &mut self.choices {
                        if choice.label != "This Helm · local interface" {
                            choice.availability =
                                "Connection unavailable · previously observed identity".into();
                        }
                    }
                    self.status = error;
                }
            }
        }
    }
    fn discover(&mut self) {
        self.stop_discovery();
        let Some(client) = self.client.clone() else {
            return;
        };
        let tx = self.tx.clone();
        let request = Uuid::new_v4();
        self.request = Some(request);
        let origin = client.origin().to_owned();
        self.discovery = Some(tokio::spawn(async move {
            let result = client.discover().await.map_err(|error| error.to_string());
            let _ = tx.send((request, origin, result));
        }));
        self.status =
            "Connecting to Vessel… Esc cancels. Only machine presence is requested.".into();
    }
    fn save_ready(&mut self, draft: Draft) {
        let labels: BTreeMap<_, _> = self
            .editing
            .iter()
            .flat_map(|s| s.helm_labels.iter().map(|(id, label)| (*id, label.clone())))
            .chain(self.form_choices.iter().map(|c| (c.id, c.label.clone())))
            .filter(|(id, _)| draft.participants.contains(id))
            .collect();
        let record = if let Some(saved) = &self.editing {
            let mut record = saved.clone();
            record.draft = draft;
            record.helm_labels = labels;
            record
        } else {
            StoredDraft::new(draft, self.form_origin.clone(), labels)
        };
        // Preserve immutable identity even if publication succeeds but acknowledgement fails.
        self.editing = Some(record.clone());
        match Store::new(self.path.clone()).and_then(|store| store.save(&record)) {
            Ok(saved) => {
                self.save_failed = false;
                self.editing = Some(saved);
                self.open();
                self.status = "Voyage draft saved on this computer. No sessions were shared and no work was started.".into();
            }
            Err(_) => {
                self.save_failed = true;
                self.form.open();
                self.status = "Save failed or draft changed. Enter retries the same draft; Ctrl+S saves your edits as a separate copy. Esc keeps edits for later.".into();
            }
        }
    }
    pub(super) fn key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.close();
            return;
        }
        if self.mode == Mode::Library && self.replacement.is_some() {
            match key.code {
                KeyCode::Enter => {
                    let saved = self.replacement.take().expect("checked replacement");
                    self.start_form(saved);
                }
                KeyCode::Esc => {
                    self.replacement = None;
                    self.status = "Edits retained · C continues setup".into();
                }
                _ => {}
            }
            return;
        }
        match self.mode {
            Mode::Closed => {}
            Mode::Form => {
                if self.save_failed
                    && key.code == KeyCode::Char('s')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.editing = None;
                    self.save_ready(self.form.draft());
                    return;
                }
                self.form.key(key);
                if let Some(draft) = self.form.take_ready() {
                    self.save_ready(draft);
                } else if !self.form.is_open() {
                    self.mode = Mode::Library;
                    self.status =
                        "Unsaved setup retained in this window · C continues editing".into();
                }
            }
            Mode::Library => match key.code {
                KeyCode::Esc => self.close(),
                KeyCode::Char('n' | 'N') => self.request_form(None),
                KeyCode::Char('c' | 'C') => {
                    if !self.form_started {
                        self.start_form(None);
                    } else {
                        self.form.open();
                        self.mode = Mode::Form;
                    }
                }
                KeyCode::Enter => {
                    if let Some(saved) = self.drafts.get(self.selected).cloned() {
                        self.request_form(Some(saved));
                    }
                }
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.drafts.len().saturating_sub(1))
                }
                KeyCode::Char('v' | 'V') => {
                    self.mode = Mode::Connect;
                    self.token_focus = false;
                    self.status = "Use the Vessel HTTPS origin and operator token. Token stays in memory and is never saved with drafts.".into();
                }
                KeyCode::Char('d' | 'D') => {
                    self.stop_discovery();
                    self.client = None;
                    self.connected_origin = None;
                    self.token.zeroize();
                    self.choices
                        .retain(|choice| choice.label == "This Helm · local interface");
                    self.status = "Disconnected from Vessel. Saved drafts are unchanged.".into();
                }
                KeyCode::Char('r' | 'R') => {
                    if self.client.is_some() {
                        self.discover();
                    } else {
                        self.open();
                    }
                }
                _ => {}
            },
            Mode::Connect => match key.code {
                KeyCode::Esc => {
                    self.stop_discovery();
                    self.token.zeroize();
                    self.mode = Mode::Library;
                }
                _ if self.discovery.is_some() => {}
                KeyCode::Tab | KeyCode::BackTab => self.token_focus = !self.token_focus,
                KeyCode::Enter => {
                    if !self.token_focus {
                        self.token_focus = true;
                    } else {
                        let token = std::mem::take(&mut *self.token);
                        match Client::new(self.origin.text.trim(), token) {
                        Ok(client) => { self.client = Some(Arc::new(client)); self.discover(); },
                        Err(_) => self.status = "Enter an HTTPS origin (HTTP only on literal loopback) and a valid 32–1024 byte operator token. Credentials in URLs are not allowed.".into(),
                    }
                    }
                }
                KeyCode::Backspace if self.token_focus => {
                    self.token.pop();
                }
                KeyCode::Backspace => self.origin.backspace(),
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.paste(&c.to_string())
                }
                _ => {}
            },
        }
    }
    pub(super) fn paste(&mut self, text: &str) {
        if self.mode == Mode::Form {
            self.form.paste(text);
        } else if self.mode == Mode::Connect
            && self.discovery.is_none()
            && !text.chars().any(char::is_control)
        {
            if self.token_focus {
                if self.token.len().saturating_add(text.len()) <= 1024 {
                    self.token.push_str(text);
                }
            } else if self.origin.text.len().saturating_add(text.len()) <= 2048 {
                self.origin.insert_str(text);
            }
        }
    }
    pub(super) fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect) {
        frame.render_widget(ratatui::widgets::Clear, area);
        if self.mode == Mode::Form {
            let status_height = if area.height >= 12 {
                3
            } else {
                area.height.min(1)
            };
            self.form.draw(
                frame,
                Rect::new(
                    area.x,
                    area.y,
                    area.width,
                    area.height.saturating_sub(status_height),
                ),
            );
            let status_area = Rect::new(
                area.x,
                area.bottom().saturating_sub(status_height),
                area.width,
                status_height,
            );
            frame.render_widget(
                Paragraph::new(display_safe(&self.status))
                    .wrap(Wrap { trim: false })
                    .style(Style::default().fg(Color::Yellow)),
                status_area,
            );
            return;
        }
        let title = if self.mode == Mode::Connect {
            " Vessel connection "
        } else {
            " Voyages · drafts "
        };
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        // Keep the active field or selected draft visible, even when explanations
        // and status messages wrap. List rows never consume additional lines.
        let status_height = if inner.height >= 9 {
            3
        } else if inner.height >= 6 {
            2
        } else {
            1.min(inner.height)
        };
        let help_height = if inner.height >= 9 {
            2
        } else if inner.height >= 4 {
            1
        } else {
            0
        };
        let body_height = inner.height.saturating_sub(status_height + help_height);
        let body = Rect::new(inner.x, inner.y, inner.width, body_height);
        let help_area = Rect::new(inner.x, body.bottom(), inner.width, help_height);
        let status_area = Rect::new(inner.x, help_area.bottom(), inner.width, status_height);
        let mut lines: Vec<Line<'static>> = Vec::new();
        let help = if self.mode == Mode::Connect {
            let origin = display_safe(&self.origin.text);
            let origin_value = hub_tail(&origin, body.width.saturating_sub(1) as usize);
            let token = "•".repeat(
                self.token
                    .chars()
                    .count()
                    .min(body.width.saturating_sub(1) as usize),
            );
            let origin_lines = [
                Line::styled(
                    if self.token_focus {
                        "Vessel origin"
                    } else {
                        "> Vessel origin"
                    },
                    Style::default().fg(if self.token_focus {
                        Color::Gray
                    } else {
                        Color::Cyan
                    }),
                ),
                Line::from(format!(
                    "{origin_value}{}",
                    if self.token_focus { "" } else { "▏" }
                )),
            ];
            let token_lines = [
                Line::styled(
                    if self.token_focus {
                        "> Operator token"
                    } else {
                        "Operator token"
                    },
                    Style::default().fg(if self.token_focus {
                        Color::Cyan
                    } else {
                        Color::Gray
                    }),
                ),
                Line::from(format!(
                    "{token}{}",
                    if self.token_focus { "▏" } else { "" }
                )),
            ];
            if body.height >= 4 {
                lines.extend(origin_lines);
                lines.extend(token_lines);
                if body.height >= 6 {
                    lines.push(Line::from(""));
                    lines.push(Line::from(hub_clip(
                        "Token stays in memory. No remote work starts.",
                        body.width as usize,
                    )));
                }
            } else {
                lines.extend(if self.token_focus {
                    token_lines
                } else {
                    origin_lines
                });
            }
            if inner.width >= 45 {
                vec![
                    "Tab switch field · Enter connect · Esc back",
                    "HTTPS origin; HTTP only on literal loopback",
                ]
            } else {
                vec![
                    if self.token_focus {
                        "Tab field · Enter send"
                    } else {
                        "Tab field · Enter next"
                    },
                    "Esc back · token private",
                ]
            }
        } else {
            if self.drafts.is_empty() {
                lines.push(Line::from(hub_clip(
                    "No saved drafts. N creates one.",
                    body.width as usize,
                )));
            }
            let visible = body.height.max(1) as usize;
            let start = self.selected.saturating_sub(visible.saturating_sub(1));
            for (index, saved) in self.drafts.iter().enumerate().skip(start).take(visible) {
                let label = format!(
                    "{} {} · {} Helms",
                    if index == self.selected { ">" } else { " " },
                    display_safe(&saved.draft.name),
                    saved.draft.participants.len()
                );
                lines.push(Line::styled(
                    hub_clip(&label, body.width as usize),
                    Style::default().fg(if index == self.selected {
                        Color::Cyan
                    } else {
                        Color::Gray
                    }),
                ));
            }
            if inner.width >= 61 {
                vec![
                    "N new · Enter edit · C continue · Esc back",
                    "V connect · R refresh · D disconnect · Drafts do not execute",
                ]
            } else {
                vec!["N new · V connect · D end", "Enter edit · C resume · Esc"]
            }
        };
        frame.render_widget(Paragraph::new(lines), body);
        frame.render_widget(
            Paragraph::new(
                help.into_iter()
                    .map(|line| Line::from(hub_clip(line, help_area.width as usize)))
                    .collect::<Vec<_>>(),
            )
            .style(Style::default().fg(Color::Gray)),
            help_area,
        );
        frame.render_widget(
            Paragraph::new(display_safe(&self.status))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Yellow)),
            status_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn complete(id: Uuid) -> Draft {
        Draft {
            name: "Trip".into(),
            purpose: "Research".into(),
            participants: vec![id],
            coordinator: Some(id),
        }
    }
    fn remote(id: Uuid) -> HelmChoice {
        HelmChoice {
            id,
            label: "Remote".into(),
            availability: "Observed".into(),
            can_execute: false,
        }
    }

    #[test]
    fn saved_draft_roundtrip_is_separate_from_chat_and_reopens_offline() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("drafts");
        let mut hub = Hub::new(path.clone());
        hub.open();
        let local = hub.choices[0].id;
        hub.start_form(None);
        hub.save_ready(complete(local));
        assert_eq!(hub.drafts.len(), 1);
        let record = hub.drafts[0].clone();
        drop(hub);
        let mut reopened = Hub::new(path);
        reopened.open();
        reopened.start_form(Some(record));
        assert_eq!(reopened.form.draft(), complete(local));
        assert!(reopened.client.is_none());
        assert!(!temp.path().join("sessions").exists());
    }
    #[test]
    fn cancelled_form_keeps_original_vessel_after_connection_change() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        let id = Uuid::new_v4();
        hub.choices.push(remote(id));
        hub.connected_origin = Some("https://a.example".into());
        hub.start_form(None);
        hub.form.set_draft(complete(id));
        hub.key(key(KeyCode::Esc));
        hub.connected_origin = Some("https://b.example".into());
        hub.choices.retain(|c| c.id != id);
        hub.key(key(KeyCode::Char('c')));
        hub.save_ready(hub.form.draft());
        assert_eq!(hub.drafts[0].origin.as_deref(), Some("https://a.example"));
        assert_eq!(hub.drafts[0].helm_labels[&id], "Remote");
    }
    #[test]
    fn originless_saved_draft_cannot_acquire_another_vessels_choices() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        let local = hub.choices[0].id;
        let foreign = Uuid::new_v4();
        hub.choices.push(remote(foreign));
        hub.connected_origin = Some("https://b.example".into());
        hub.start_form(Some(StoredDraft::new(
            complete(local),
            None,
            BTreeMap::new(),
        )));
        assert!(!hub.form_choices.iter().any(|c| c.id == foreign));
    }
    #[test]
    fn stale_discovery_cannot_replace_destination_or_interrupt_form() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        hub.start_form(None);
        let current = Uuid::new_v4();
        hub.request = Some(current);
        hub.tx
            .send((Uuid::new_v4(), "https://stale.example".into(), Ok(vec![])))
            .unwrap();
        hub.poll();
        assert!(hub.connected_origin.is_none());
        assert!(hub.mode == Mode::Form);
        hub.tx
            .send((current, "https://current.example".into(), Ok(vec![])))
            .unwrap();
        hub.poll();
        assert_eq!(
            hub.connected_origin.as_deref(),
            Some("https://current.example")
        );
        assert!(hub.mode == Mode::Form);
    }
    #[test]
    fn cancellation_clears_secret_and_continuation_initializes_choices() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        hub.key(key(KeyCode::Char('c')));
        assert!(!hub.form_choices.is_empty());
        hub.key(key(KeyCode::Esc));
        hub.key(key(KeyCode::Char('v')));
        hub.token_focus = true;
        hub.paste("secret-canary");
        hub.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!hub.is_open());
        assert!(hub.token.is_empty());
    }
    #[test]
    fn conflict_copy_preserves_both_versions_and_new_requires_discard_decision() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        hub.start_form(None);
        let draft = complete(hub.choices[0].id);
        hub.form.set_draft(draft.clone());
        hub.save_ready(draft);
        let saved = hub.drafts[0].clone();
        hub.start_form(Some(saved.clone()));
        let mut other = saved.clone();
        other.draft.name = "Other writer".into();
        Store::new(hub.path.clone()).unwrap().save(&other).unwrap();
        let mut edited = hub.form.draft();
        edited.name = "My retained edits".into();
        hub.form.set_draft(edited.clone());
        hub.save_ready(edited);
        assert!(hub.save_failed);
        hub.key(key(KeyCode::Esc));
        hub.key(key(KeyCode::Char('n')));
        assert!(hub.replacement.is_some());
        hub.key(key(KeyCode::Esc));
        assert_eq!(hub.form.draft().name, "My retained edits");
        hub.key(key(KeyCode::Char('c')));
        hub.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(hub.drafts.len(), 2);
        assert!(hub.drafts.iter().any(|d| d.draft.name == "Other writer"));
        assert!(
            hub.drafts
                .iter()
                .any(|d| d.draft.name == "My retained edits")
        );
    }
    #[test]
    fn failed_save_retains_identity_and_form_for_retry() {
        let temp = tempfile::tempdir().unwrap();
        let mut hub = Hub::new(temp.path().join("drafts"));
        hub.open();
        hub.start_form(None);
        let draft = complete(hub.choices[0].id);
        hub.path = temp.path().join("missing-parent").join("drafts");
        hub.save_ready(draft.clone());
        let id = hub.editing.as_ref().unwrap().id;
        assert!(hub.mode == Mode::Form);
        std::fs::create_dir(temp.path().join("missing-parent")).unwrap();
        hub.save_ready(draft);
        assert_eq!(hub.drafts[0].id, id);
    }
}

fn hub_clip(text: &str, width: usize) -> String {
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
fn hub_tail(text: &str, width: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    if text.width() <= width {
        return text.to_owned();
    }
    let mut suffix = Vec::new();
    let mut used = 0;
    for grapheme in text.graphemes(true).rev() {
        if used + grapheme.width() > width.saturating_sub(1) {
            break;
        }
        used += grapheme.width();
        suffix.push(grapheme);
    }
    let mut out = if width > 0 {
        "…".to_owned()
    } else {
        String::new()
    };
    for grapheme in suffix.into_iter().rev() {
        out.push_str(grapheme);
    }
    out
}

#[cfg(test)]
mod draw_tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn render(hub: &Hub, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| hub.draw(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }
    #[test]
    fn narrow_connect_keeps_focused_token_and_errors_visible_without_credentials() {
        let mut hub = Hub::new(PathBuf::from("/unused"));
        hub.mode = Mode::Connect;
        hub.token_focus = true;
        hub.origin.text = "https://a-very-long-but-valid-vessel-origin.example.com".into();
        hub.token
            .push_str("private-operator-token-that-must-not-be-rendered");
        hub.status = "Token rejected. Reconnect with an authorized token.".into();
        for (width, height) in [(24, 12), (40, 12), (80, 24)] {
            let screen = render(&hub, width, height);
            assert!(screen.contains("Operator token"));
            assert!(screen.contains("Token rejected."));
            assert!(screen.contains('•'));
            assert!(!screen.contains("private-operator-token"));
        }
        for (width, height) in [(1, 1), (2, 2), (8, 4)] {
            let _ = render(&hub, width, height);
        }
    }
    #[test]
    fn long_library_names_do_not_hide_selection_or_storage_failure() {
        let mut hub = Hub::new(PathBuf::from("/unused"));
        hub.mode = Mode::Library;
        for index in 0..30 {
            let draft = Draft {
                name: format!("draft{index:02} {}", "界".repeat(70)),
                ..Draft::default()
            };
            hub.drafts
                .push(StoredDraft::new(draft, None, BTreeMap::new()));
        }
        hub.selected = 29;
        hub.status = "Save failed. Your edits are retained.".into();
        let screen = render(&hub, 24, 12);
        assert!(screen.contains("> draft29"));
        assert!(screen.contains("Save failed."));
    }
    #[test]
    fn clipping_respects_grapheme_width_and_keeps_editing_suffix() {
        use unicode_width::UnicodeWidthStr;
        for width in 0..12 {
            assert!(hub_clip("界😀abcdefgh", width).width() <= width);
            assert!(hub_tail("界😀abcdefgh", width).width() <= width);
        }
        assert_eq!(hub_tail("https://example.com", 5), "….com");
    }
}
