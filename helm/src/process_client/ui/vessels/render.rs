use super::*;
use ratatui::{
    layout::Margin,
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthChar;

struct Row {
    text: String,
    button: Option<Button>,
    selected: bool,
}
impl Row {
    fn plain(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            button: None,
            selected: false,
        }
    }
    fn button(s: impl Into<String>, button: Button) -> Self {
        Self {
            text: s.into(),
            button: Some(button),
            selected: false,
        }
    }
}
impl Manager {
    pub(super) fn draw(&mut self, frame: &mut Frame, area: Rect) {
        if !self.panel.open {
            return;
        }
        self.sync_focus();
        self.panel.hits.clear();
        // Keep the current view visible around the modal, even in narrow layouts.
        let width = area.width.saturating_sub(4).min(96);
        let height = area.height.saturating_sub(2).min(30);
        let modal = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(crate::theme::Role::Focus.style())
            .title(" Vessels ")
            .title_bottom(" Esc Back / Close ");
        let area = block.inner(modal).inner(Margin::new(1, 0));
        frame.render_widget(Clear, modal);
        frame.render_widget(block, modal);
        // Wrapping, scrolling and mouse targets use the modal's content bounds.
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mut rows = vec![
            Row::plain("VESSELS — private connection panel"),
            Row::button("[Esc: Back / Close]", Button::Back),
        ];
        match &self.panel.page {
            Page::List => {
                rows.push(Row::plain(
                    "Unavailable Vessels are not polled. Select one and press c to retry.",
                ));
                rows.push(Row::plain(
                    "Drafts and pending commands are retained; remote runtime state is unknown.",
                ));
                rows.push(Row::button(
                    "[u Retry unavailable Vessels (including launch-only routes)]",
                    Button::RetryUnavailable,
                ));
                rows.push(Row::button("[a Add HTTPS + pairing]", Button::Add));
                rows.push(Row::button("[w New on selected Vessel]", Button::New));
                rows.push(Row::button(
                    "[v Filter selected / l All Vessels]",
                    Button::Filter,
                ));
                rows.push(Row::button("[l Show all Vessels]", Button::All));
                rows.push(Row::button(
                    "[i Import existing access file]",
                    Button::Import,
                ));
                rows.push(Row {
                    text: format!(
                        "This computer — {}",
                        self.local_id
                            .and_then(|id| self.states.get(&id))
                            .map(|(_, state)| state.label())
                            .unwrap_or("not connected")
                    ),
                    button: Some(Button::Select(0)),
                    selected: self.panel.selected == 0,
                });
                for (i, c) in self.records.iter().enumerate() {
                    let state = self
                        .states
                        .get(&c.id)
                        .map(|(_, s)| *s)
                        .unwrap_or(ConnectionState::Offline);
                    rows.push(Row {
                        text: format!(
                            "{} — {}",
                            text(&c.alias),
                            if c.forgotten {
                                "Forgotten · recovery retained"
                            } else {
                                state.label()
                            }
                        ),
                        button: Some(Button::Select(i + 1)),
                        selected: self.panel.selected == i + 1,
                    });
                    rows.push(Row::button(
                        format!(
                            "  {} · {} · connect at startup {}",
                            text(&c.endpoint),
                            scope(&c.scope),
                            if c.autoconnect { "on" } else { "off" }
                        ),
                        Button::Select(i + 1),
                    ));
                }
                for (i, id) in self.pending.iter().enumerate() {
                    let index = self.records.len() + i + 1;
                    rows.push(Row {
                        text: format!("Pending pairing {id} — select, then p/Enter to recover"),
                        button: Some(Button::Select(index)),
                        selected: self.panel.selected == index,
                    });
                }
                if let Some(c) = self.selected() {
                    rows.push(Row::plain(format!(
                        "Selected authenticated Vessel: {}",
                        c.vessel_id
                    )));
                    rows.push(Row::plain(format!(
                        "Principal: {} · connection: {}",
                        c.principal_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "session credential".into()),
                        c.id
                    )));
                    metadata(&mut rows, &c.metadata);
                    if c.forgotten {
                        rows.push(Row::button(
                            "[o Restore original access for recovery]",
                            Button::Restore,
                        ));
                    }
                    for (label, b) in [
                        ("[c / Enter Connect / Retry]", Button::Connect),
                        ("[d Disconnect observation only]", Button::Disconnect),
                        ("[r Rename friendly alias]", Button::Rename),
                        ("[t Toggle connect at startup]", Button::Auto),
                        ("[n Renew / Replace access]", Button::Replace),
                        ("[f Forget locally]", Button::Forget),
                    ] {
                        rows.push(Row::button(label, b));
                    }
                } else if self.panel.selected == 0 {
                    rows.push(Row::button(
                        "[c / Enter Connect / Retry local]",
                        Button::Connect,
                    ));
                    rows.push(Row::button(
                        "[d Disconnect local observation]",
                        Button::Disconnect,
                    ));
                } else if self.panel.selected > self.records.len() {
                    rows.push(Row::button(
                        "[p / Enter Recover pending pairing]",
                        Button::Resume,
                    ));
                }
                rows.push(Row::plain("Disconnect does not cancel remote work. No connection action sends your composer or creates a voyage."));
            }
            Page::Form {
                import,
                replacement,
            } => {
                if !import && let Some(principal) = self.principal {
                    rows.push(Row::plain(format!("Helm principal: {principal}")));
                    rows.push(Row::plain("Ask the executing account owner for an invitation bound to this principal."));
                }
                rows.push(Row::plain(if replacement.is_some() {
                    "Renew / Replace: old connection remains pinned and separate."
                } else {
                    "Add Vessel"
                }));
                rows.push(Row::button(
                    "[Switch pairing / access-file import]",
                    Button::Import,
                ));
                rows.push(self.field(
                    0,
                    if *import {
                        "Access-file path (private)"
                    } else {
                        "HTTPS endpoint"
                    },
                    false,
                ));
                if !import {
                    rows.push(self.field(1, "Owner pairing invitation (masked)", true));
                }
                rows.push(self.field(2, "Friendly alias (optional)", false));
                rows.push(Row::button(
                    format!("[Automatic reconnect: {}]", self.panel.autoconnect),
                    Button::Auto,
                ));
                rows.push(Row::button(
                    "[Enter: Validate and preview — does not save]",
                    Button::Submit,
                ));
                rows.push(Row::plain("Tab / Shift-Tab selects fields and actions. Enter submits a field or activates an action; Space activates an action. Ctrl-U clears a field. Paste stays private."));
            }
            Page::Preview {
                preview,
                replacement,
            } => {
                let c = &preview.connection;
                rows.push(Row::plain("AUTHENTICATED ACCESS — review before Save"));
                rows.push(Row::plain(format!("Endpoint: {}", text(&c.endpoint))));
                rows.push(Row::plain(format!("Vessel identity: {}", c.vessel_id)));
                rows.push(Row::plain(format!(
                    "Principal identity: {}",
                    c.principal_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "session credential".into())
                )));
                rows.push(Row::plain(format!("New connection identity: {}", c.id)));
                rows.push(Row::plain(format!("Scope: {}", scope(&c.scope))));
                if let Scope::Workspaces { workspace_ids } = &c.scope {
                    for id in workspace_ids {
                        rows.push(Row::plain(format!("Authorized workspace ID: {id}")));
                    }
                }
                if let Some(old) =
                    replacement.and_then(|id| self.records.iter().find(|c| c.id == id))
                {
                    rows.push(Row::plain(format!(
                        "OLD Vessel: {} · principal: {}",
                        old.vessel_id,
                        old.principal_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "session credential".into())
                    )));
                    rows.push(Row::plain("Replacement is NEW access. Identity/scope may differ. Saving does not migrate or replay old commands, remove old access or repin its identity."));
                }
                metadata(&mut rows, &c.metadata);
                rows.push(Row::plain(format!(
                    "Friendly alias: {}",
                    text(&self.panel.fields[2])
                )));
                rows.push(Row::button(
                    format!("[t Automatic reconnect: {}]", self.panel.autoconnect),
                    Button::Auto,
                ));
                rows.push(Row::button(
                    "[s / select + Enter: Save reviewed access and connect]",
                    Button::Save,
                ));
                rows.push(Row::plain("Provider credentials stay on the executing host. Human access is not model coordination authority."));
            }
            Page::Rename(_) => {
                rows.push(Row::plain(
                    "Rename local friendly alias — authenticated identity is unchanged.",
                ));
                rows.push(self.field(0, "Alias", false));
                rows.push(Row::button("[Enter Save alias]", Button::Submit));
            }
            Page::Forget(_) => {
                rows.push(Row::plain("Forget this saved connection locally?"));
                rows.push(Row::plain("This disconnects Helm observations, not remote execution. It does NOT revoke remote access, cancel, archive or delete voyages. Drafts, uncertain commands and delivery receipts remain recoverable under their original identity. Replacement access will not replay them."));
                rows.push(Row::button(
                    "[s / select + Enter: Confirm Forget]",
                    Button::Save,
                ));
            }
        }
        if self.panel.busy.is_some() {
            rows.push(Row::plain("BUSY — setup is bounded to 45 seconds. Recovery records survive uncertain pairing."));
        }
        if !self.panel.notice.is_empty() {
            rows.push(Row::plain(text(&self.panel.notice)));
        }
        rows.push(Row::plain(
            "Tab/Shift-Tab: form action • Enter/Space: activate • PgUp/PgDn: scroll • Esc: back/close",
        ));
        let mut physical = Vec::new();
        for row in rows {
            let selected = if self.panel.focus.is_empty() {
                row.selected
            } else {
                row.button
                    .is_some_and(|button| self.panel.focus.is_focused(&button))
            };
            for line in wrap(&row.text, area.width as usize) {
                physical.push((line, row.button, selected));
            }
        }
        if self.panel.reveal_selection {
            if let Some(index) = physical.iter().position(|(_, _, selected)| *selected) {
                if index < self.panel.scroll {
                    self.panel.scroll = index;
                } else if index >= self.panel.scroll + area.height as usize {
                    self.panel.scroll = index.saturating_sub(area.height as usize - 1);
                }
            }
            self.panel.reveal_selection = false;
        }
        self.panel.scroll = self
            .panel
            .scroll
            .min(physical.len().saturating_sub(area.height as usize));
        for (y, (line, button, selected)) in physical
            .into_iter()
            .skip(self.panel.scroll)
            .take(area.height as usize)
            .enumerate()
        {
            let rect = Rect::new(area.x, area.y + y as u16, area.width, 1);
            let style = if selected {
                crate::theme::Role::Selection.style()
            } else if button.is_some() {
                crate::theme::Role::Focus.style()
            } else {
                Style::default()
            };
            frame.render_widget(Paragraph::new(line).style(style), rect);
            if let Some(button) = button {
                self.panel.hits.push((rect, button));
            }
        }
    }
    fn field(&self, i: usize, label: &str, masked: bool) -> Row {
        Row {
            text: format!(
                "{} {}: {}",
                if self.panel.focus.is_focused(&Button::Field(i)) {
                    ">"
                } else {
                    " "
                },
                label,
                if masked {
                    "•".repeat(self.panel.fields[i].chars().count().min(32))
                } else {
                    text(&self.panel.fields[i])
                }
            ),
            button: Some(Button::Field(i)),
            selected: self.panel.focus.is_focused(&Button::Field(i)),
        }
    }
}
fn scope(scope: &Scope) -> String {
    match scope {
        Scope::Session { session_id } => {
            format!("Shared conversation {session_id} (no new conversations)")
        }
        Scope::Workspaces { workspace_ids } => {
            format!("Workspace access ({} authorized)", workspace_ids.len())
        }
    }
}
fn metadata(rows: &mut Vec<Row>, metadata: &crate::process_client::connections::Metadata) {
    rows.push(Row::plain(format!(
        "Vessel version: {}",
        metadata.version.as_deref().unwrap_or("not advertised")
    )));
    rows.push(Row::plain(format!(
        "Rights: {}",
        text(&format!("{:?}", metadata.rights))
    )));
    let expiry = metadata
        .expires_at_ms
        .and_then(|ms| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64));
    rows.push(Row::plain(format!(
        "Access expiry: {}",
        expiry
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "not advertised".into())
    )));
    if let Some(expiry) = expiry {
        let left = expiry.signed_duration_since(chrono::Utc::now());
        if left.num_seconds() <= 0 {
            rows.push(Row::plain(
                "ACCESS EXPIRED · obtain owner-authorized replacement access",
            ));
        } else if left.num_hours() < 24 {
            rows.push(Row::plain(
                "Access expires within 24 hours · arrange owner-authorized renewal",
            ));
        }
    }
    rows.push(Row::plain(format!(
        "Grant revision: {:?}",
        metadata.grant_revision
    )));
    rows.push(Row::plain(format!(
        "Server features: {}",
        text(&format!("{:?}", metadata.features))
    )));
    for workspace in &metadata.workspaces {
        rows.push(Row::plain(format!(
            "Workspace {}: {} · {} · provider ready: {}",
            workspace.id,
            text(&workspace.name),
            text(&workspace.path.to_string_lossy()),
            match workspace.provider_ready {
                Some(true) => "ready",
                Some(false) => "authorize on execution host",
                None => "not yet checked",
            }
        )));
    }
}
fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for c in value.chars() {
        let n = c.width().unwrap_or(0);
        if used + n > width && !line.is_empty() {
            result.push(std::mem::take(&mut line));
            used = 0;
        }
        if n <= width {
            line.push(c);
            used += n;
        }
    }
    result.push(line);
    result
}
