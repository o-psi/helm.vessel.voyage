//! Saved workflows stay on the selected executing host. No private value is a
//! composer draft, history entry, serializable panel, or durable command field.
//!
//! Integration: add `workflows: workflows::State` to App (Default), call
//! `open_workflows()` from /workflows and F2, `workflow_input(&event)` BEFORE
//! paste/composer handling, `poll_workflows()` on repaint, and `draw` last.
//! Gate ordinary paste destinations on `!workflows_open()`. Requests use an
//! internal oneshot; no new Update variant is needed. Submission uses the
//! existing Update::Command / Pending::resolution / drafts recovery contract.
//! No workflow-specific history insertion or new-process launch hook is needed.
use super::{
    App, drafts,
    observe::Update,
    state::{Pending, Target},
};
use crate::workflow::{Document, Parameter, ParameterType, Scope};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct State {
    panel: Option<Panel>,
}
// Intentionally no Debug, Clone or Serialize on the private editor or panel.
struct Panel {
    target: Target,
    incarnation: Uuid,
    host: String,
    entries: Vec<Entry>,
    selected: usize,
    phase: Phase,
    fields: Vec<String>,
    field: usize,
    public: BTreeMap<String, String>,
    private: BTreeMap<String, Zeroizing<String>>,
    editor: Zeroizing<String>,
    preview: String,
    revision: u64,
    notice: String,
    scroll: u16,
    deadline: Instant,
    request: Option<oneshot::Receiver<Result<Value, &'static str>>>,
}
#[derive(Clone, Deserialize)]
struct Entry {
    scope: Scope,
    digest: String,
    document: Document,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Inventory,
    Trust,
    Inputs,
    Preview,
    Confirm,
}

impl Entry {
    fn scope(&self) -> &'static str {
        match self.scope {
            Scope::User => "user",
            Scope::Repository => "repository",
        }
    }
}
impl Panel {
    fn entry(&self) -> &Entry {
        &self.entries[self.selected]
    }
    fn parameter(&self) -> &Parameter {
        &self.entry().document.parameters[&self.fields[self.field]]
    }
    fn clear_inputs(&mut self) {
        self.private.clear();
        self.public.clear();
        self.editor = Zeroizing::new(String::new());
        self.preview.clear();
    }
    fn select(&mut self) {
        self.clear_inputs();
        self.fields = self.entry().document.parameters.keys().cloned().collect();
        self.field = 0;
        self.scroll = 0;
        self.phase = Phase::Trust;
    }
    fn load_field(&mut self) {
        self.editor = Zeroizing::new(String::new());
        if let Some(name) = self.fields.get(self.field) {
            let p = &self.entry().document.parameters[name];
            let text = if p.secret {
                None
            } else {
                self.public
                    .get(name)
                    .cloned()
                    .or_else(|| p.default.as_ref().map(input_text))
            };
            if let Some(text) = text {
                self.editor.push_str(&text);
            }
        }
    }
    fn commit_field(&mut self) -> Result<()> {
        let p = self.parameter();
        // The current host preview API models required secrets only. Never show
        // a null optional binding then silently execute with a private value.
        ensure!(
            !(p.secret && !p.required && !self.editor.is_empty()),
            "Optional private input is not supported by this host preview protocol; leave it omitted"
        );
        if self.editor.is_empty() && !p.required && p.default.is_none() {
            return Ok(());
        }
        validate_input(p, &self.editor)?;
        let secret = p.secret;
        let name = self.fields[self.field].clone();
        let value = std::mem::replace(&mut self.editor, Zeroizing::new(String::new()));
        if secret {
            self.private.insert(name, value);
        } else {
            self.public.insert(name, value.to_string());
        }
        Ok(())
    }
    fn public_inputs(&self) -> Vec<(String, String)> {
        self.public
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    fn preview_command(&self) -> VoyageCommand {
        VoyageCommand::WorkflowPreview {
            id: self.entry().document.id.clone(),
            scope: Some(self.entry().scope().into()),
            user_directory: None,
            inputs: self.public_inputs(),
            trust_digest: Some(self.entry().digest.clone()),
        }
    }
    fn pending(&self, expires_at_ms: u64, private_inputs_id: Option<Uuid>) -> Pending {
        let command_id = Uuid::new_v4();
        Pending {
            command_id,
            incarnation: self.incarnation,
            draft: String::new(),
            preserve_draft: true,
            receipt_only: false,
            original: Some(Box::new(VoyageCommand::WorkflowSubmit {
                command_id,
                expected_revision: self.revision,
                expires_at_ms,
                id: self.entry().document.id.clone(),
                scope: Some(self.entry().scope().into()),
                user_directory: None,
                inputs: self.public_inputs(),
                trust_digest: Some(self.entry().digest.clone()),
                private_inputs_id,
            })),
        }
    }
}
fn plain_activation(key: &crossterm::event::KeyEvent) -> bool {
    key.kind == KeyEventKind::Press && key.modifiers.is_empty()
}

fn input_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
fn validate_input(p: &Parameter, text: &str) -> Result<()> {
    ensure!(
        text.len() <= 8192 && !text.contains('\0'),
        "Input exceeds bounds or contains NUL"
    );
    let value = match p.kind {
        ParameterType::String => {
            ensure!(
                text.len() <= p.max_length.unwrap_or(8192),
                "String exceeds length limit"
            );
            if p.secret {
                ensure!(p.choices.is_empty(), "Unsupported private choices");
                return Ok(());
            }
            Value::String(text.into())
        }
        ParameterType::Integer => {
            let n = text
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("Enter an integer"))?;
            ensure!(
                p.minimum.is_none_or(|m| n >= m) && p.maximum.is_none_or(|m| n <= m),
                "Integer outside allowed bounds"
            );
            Value::from(n)
        }
        ParameterType::Boolean => Value::Bool(
            text.parse::<bool>()
                .map_err(|_| anyhow::anyhow!("Enter true or false"))?,
        ),
    };
    ensure!(
        p.choices.is_empty() || p.choices.contains(&value),
        "Input is not an allowed choice"
    );
    Ok(())
}

impl App {
    pub(super) fn workflows_open(&self) -> bool {
        self.workflows.panel.is_some()
    }

    pub(super) fn open_workflows(&mut self) -> Result<()> {
        ensure!(
            self.active_draft.is_none(),
            "Saved workflows require an existing selected voyage"
        );
        let target = self.selected.context("Select an existing voyage first")?;
        self.cancel_paste_for_private_panel();
        ensure!(
            self.clients.available(target.route),
            "Executing host is disconnected"
        );
        let view = self.views.get(&target).context("Waiting for voyage")?;
        ensure!(
            !view.deleted() && !view.archived(),
            "Restore an available voyage before opening workflows"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("Waiting for voyage snapshot")?;
        let panel = Panel {
            target,
            incarnation: view.process.incarnation,
            host: self.route_label(target.route),
            entries: Vec::new(),
            selected: 0,
            phase: Phase::Inventory,
            fields: Vec::new(),
            field: 0,
            public: BTreeMap::new(),
            private: BTreeMap::new(),
            editor: Zeroizing::new(String::new()),
            preview: String::new(),
            revision: snapshot.revision,
            notice: "Loading saved workflows from executing host…".into(),
            scroll: 0,
            deadline: Instant::now() + Duration::from_secs(300),
            request: None,
        };
        let run_id = snapshot.run.as_ref().map(|r| r.run_id);
        self.workflows.panel = Some(panel);
        self.workflow_request(VoyageCommand::Controls {
            run_id,
            section: "workflows".into(),
        });
        Ok(())
    }
    fn workflow_request(&mut self, command: VoyageCommand) {
        let panel = self.workflows.panel.as_mut().expect("workflow panel");
        let (sender, receiver) = oneshot::channel();
        panel.request = Some(receiver);
        let target = panel.target;
        let incarnation = panel.incarnation;
        let client = self.clients[target.route].clone();
        let job = tokio::spawn(async move {
            // Never expose server diagnostics: private transport and host content
            // must not acquire an error-string path to the composer or history.
            let result = client.voyage(target.session, incarnation, command).await
                .map_err(|_| "Host request failed or was refused; close and reload to inspect current state");
            let _ = sender.send(result);
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(job);
    }
    pub(super) fn poll_workflows(&mut self) {
        let Some(panel) = self.workflows.panel.as_mut() else {
            return;
        };
        if Instant::now() >= panel.deadline
            || !self.clients.available(panel.target.route)
            || self.selected != Some(panel.target)
            || self.active_draft.is_some()
            || self
                .views
                .get(&panel.target)
                .is_none_or(|v| v.process.incarnation != panel.incarnation)
        {
            self.workflows.panel = None;
            self.status = "Workflow editor closed; transient private inputs discarded. Pending submissions still reconcile.".into();
            return;
        }
        let Some(receiver) = panel.request.as_mut() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(value) => value,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(_) => Err("Workflow observation interrupted; close and reload"),
        };
        panel.request = None;
        if let Err(error) = apply_response(panel, result) {
            panel.notice = error.to_string();
        }
    }
    /// Must precede *every* clipboard, composer, shortcut and history input path.
    /// All events are consumed while open, including asynchronous paste events.
    pub(super) fn workflow_input(&mut self, event: &Event) -> Result<bool> {
        if self.workflows.panel.is_none() {
            return Ok(false);
        }
        if matches!(event, Event::FocusLost)
            || matches!(event, Event::Key(k) if k.code == KeyCode::Esc || (k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c' | 'q'))))
        {
            self.workflows.panel = None;
            self.status = "Workflow entry cancelled; private inputs discarded".into();
            return Ok(true);
        }
        self.poll_workflows();
        let Some(panel) = self.workflows.panel.as_mut() else {
            return Ok(true);
        };
        if panel.request.is_some() {
            return Ok(true);
        }
        if let Event::Paste(text) = event {
            if panel.phase == Phase::Inputs {
                if panel.editor.len() + text.len() <= 8192 && !text.contains('\0') {
                    panel.editor.push_str(text);
                } else {
                    panel.notice = "Paste exceeds input limit or contains NUL".into();
                }
            }
            return Ok(true);
        }
        let Event::Key(key) = event else {
            return Ok(true);
        };
        if key.kind == KeyEventKind::Release {
            return Ok(true);
        }
        match (panel.phase, key.code) {
            (Phase::Inventory, KeyCode::Up) => panel.selected = panel.selected.saturating_sub(1),
            (Phase::Inventory, KeyCode::Down) => {
                panel.selected = (panel.selected + 1).min(panel.entries.len().saturating_sub(1))
            }
            (Phase::Inventory, KeyCode::Enter) if !panel.entries.is_empty() => {
                panel.select();
                panel.notice.clear();
            }
            (Phase::Trust, KeyCode::Char('t')) if plain_activation(key) => {
                panel.phase = Phase::Inputs;
                panel.load_field();
                panel.notice.clear();
                if panel.fields.is_empty() {
                    self.start_workflow_preview()?;
                }
            }
            (Phase::Inputs, KeyCode::Backspace) => {
                panel.editor.pop();
            }
            (Phase::Inputs, KeyCode::Char(c))
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if panel.editor.len() + c.len_utf8() <= 8192 && !c.is_control() {
                    panel.editor.push(c);
                }
            }
            (Phase::Inputs, KeyCode::Enter) => match panel.commit_field() {
                Err(error) => panel.notice = error.to_string(),
                Ok(()) => {
                    panel.field += 1;
                    panel.notice.clear();
                    if panel.field == panel.fields.len() {
                        self.start_workflow_preview()?;
                    } else {
                        panel.load_field();
                    }
                }
            },
            (Phase::Confirm, KeyCode::Char('y')) if plain_activation(key) => {
                if let Err(error) = self.submit_workflow() {
                    if let Some(panel) = self.workflows.panel.as_mut() {
                        panel.notice = error.to_string();
                    } else {
                        self.status = error.to_string();
                    }
                }
            }
            (_, KeyCode::PageDown) => panel.scroll = panel.scroll.saturating_add(8),
            (_, KeyCode::PageUp) => panel.scroll = panel.scroll.saturating_sub(8),
            _ => {}
        }
        Ok(true)
    }
    fn start_workflow_preview(&mut self) -> Result<()> {
        let panel = self.workflows.panel.as_mut().expect("workflow panel");
        let command = panel.preview_command();
        panel.phase = Phase::Preview;
        panel.scroll = 0;
        panel.notice = "Validating public inputs and rendering on executing host…".into();
        self.workflow_request(command);
        Ok(())
    }
    fn submit_workflow(&mut self) -> Result<()> {
        let panel = self
            .workflows
            .panel
            .as_ref()
            .context("Workflow editor closed")?;
        ensure!(
            panel.phase == Phase::Confirm,
            "Executing-host preview required"
        );
        let target = panel.target;
        ensure!(
            self.clients.available(target.route),
            "Executing host disconnected"
        );
        let view = self.views.get(&target).context("Voyage unavailable")?;
        let snapshot = view.snapshot.as_ref().context("Snapshot unavailable")?;
        ensure!(
            view.process.incarnation == panel.incarnation && snapshot.revision == panel.revision,
            "Voyage changed during entry; close and preview again"
        );
        ensure!(
            view.pending.is_none(),
            "Another command is still unresolved"
        );
        ensure!(
            !view.deleted()
                && !view.archived()
                && !snapshot.recovery_pending
                && snapshot.pending_cleanup_run.is_none()
                && snapshot.run.as_ref().is_none_or(|r| !r.active()),
            "Workflow requires an idle voyage with no pending recovery or cleanup"
        );
        ensure!(
            panel
                .private
                .iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
                <= 65536,
            "Private input bundle exceeds executing-host handoff limit; close and retry with smaller values"
        );
        let input_id = (!panel.private.is_empty()).then(Uuid::new_v4);
        let expires_at_ms = u64::try_from(chrono::Utc::now().timestamp_millis())?
            .checked_add(60_000)
            .context("Clock overflow")?;
        let pending = panel.pending(expires_at_ms, input_id);
        let command_id = pending.command_id;
        let command = pending
            .original
            .as_deref()
            .expect("workflow command")
            .clone();
        let incarnation = panel.incarnation;
        let view = self.views.get_mut(&target).expect("checked view");
        view.pending = Some(pending);
        if drafts::save(&self.clients[target.route], view).is_err() {
            view.pending = None;
            anyhow::bail!("Cannot persist exact workflow command identity; nothing sent");
        }
        let panel = self.workflows.panel.take().expect("workflow panel");
        // Insert the in-flight guard before any effect or a reconciliation tick.
        self.command_checks.insert((target, command_id), None);
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let job = tokio::spawn(async move {
            let result = async {
                if let Some(input_id) = input_id {
                    let values = panel
                        .private
                        .into_iter()
                        .map(|(name, mut value)| (name, std::mem::take(&mut *value)))
                        .collect();
                    client
                        .voyage(
                            target.session,
                            incarnation,
                            VoyageCommand::WorkflowInputs { input_id, values },
                        )
                        .await?;
                }
                client.voyage(target.session, incarnation, command).await
            }
            .await;
            // A lost private handoff is never replayed. Resolve the durable public
            // envelope even if submit was not reached; the runtime seals absence.
            if let Ok(value) = &result {
                let _ = super::routes::retain_receipt(target, command_id, value);
            }
            let _ = sender.send(Update::Command {
                target, command_id, refused: false,
                result: result.map_err(|_: anyhow::Error| "Workflow result unavailable; exact command retained for reconciliation. Private input is never replayed.".into()),
            }).await;
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(job);
        self.status =
            format!("Workflow submitted for admission · command {command_id}; not yet confirmed");
        Ok(())
    }
}

fn apply_response(panel: &mut Panel, result: Result<Value, &'static str>) -> Result<()> {
    let value = result.map_err(anyhow::Error::msg)?;
    if panel.phase == Phase::Inventory {
        ensure!(
            value["section"] == "workflows",
            "Unexpected host inventory response"
        );
        let entries: Vec<Entry> = serde_json::from_value(value["value"].clone())
            .map_err(|_| anyhow::anyhow!("Unsupported workflow inventory schema"))?;
        ensure!(entries.len() <= 256, "Host inventory exceeds display limit");
        for entry in &entries {
            ensure!(
                entry.digest.len() == 64
                    && entry.digest.bytes().all(|b| b.is_ascii_hexdigit())
                    && entry.document.schema_version == 1
                    && entry.document.parameters.len() <= 32,
                "Unsupported workflow definition or digest"
            );
        }
        panel.entries = entries;
        panel.notice = if panel.entries.is_empty() {
            "No saved workflows found on this executing host"
        } else {
            "Up/Down select · Enter inspect · Esc close"
        }
        .into();
    } else if panel.phase == Phase::Preview {
        let fresh: Entry = serde_json::from_value(value["definition"].clone())
            .map_err(|_| anyhow::anyhow!("Unsupported host preview schema"))?;
        ensure!(
            fresh.digest == panel.entry().digest
                && fresh.scope == panel.entry().scope
                && fresh.document.id == panel.entry().document.id,
            "Workflow changed on executing host; close and inspect again"
        );
        panel.preview = value["prompt"]
            .as_str()
            .context("Host omitted workflow preview")?
            .to_owned();
        ensure!(
            panel.preview.len() <= 128 * 1024,
            "Host preview exceeds limit"
        );
        panel.phase = Phase::Confirm;
        panel.notice = "Read executing-host preview. y submits this digest to this voyage · Esc cancels. Recommendations do not change authority.".into();
    }
    Ok(())
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &App) {
    let Some(panel) = &app.workflows.panel else {
        return;
    };
    let full = frame.area();
    let area = Rect::new(
        full.x + 1,
        full.y + 1,
        full.width.saturating_sub(2),
        full.height.saturating_sub(2),
    );
    let mut text = format!(
        "Executing host: {}\nVoyage: {}\n\n",
        panel.host, panel.target.session
    );
    match panel.phase {
        Phase::Inventory => {
            for (i, entry) in panel.entries.iter().enumerate() {
                text.push_str(&format!(
                    "{} {} [{}] v{} — {}\n",
                    if i == panel.selected { "▶" } else { " " },
                    entry.document.id,
                    entry.scope(),
                    entry.document.version,
                    entry.document.description
                ));
            }
        }
        _ => {
            let entry = panel.entry();
            text.push_str(&format!(
                "{} [{}] v{}\n{}\nSHA-256 {}\n\n",
                entry.document.id,
                entry.scope(),
                entry.document.version,
                entry.document.description,
                entry.digest
            ));
            match panel.phase {
                Phase::Trust => {
                    text.push_str(&format!(
                        "Definition (untrusted content):\n{}\n\n",
                        entry.document.prompt
                    ));
                    text.push_str("t: trust this exact definition digest for host preview and entry\nNo settings or permissions are granted by this workflow.\n");
                }
                Phase::Inputs => {
                    let p = panel.parameter();
                    text.push_str(&format!(
                        "Input {}/{}: {} ({:?}, {}, {})\n{}\nBounds: {:?}..{:?} · max bytes {:?}\n",
                        panel.field + 1,
                        panel.fields.len(),
                        panel.fields[panel.field],
                        p.kind,
                        if p.required { "required" } else { "optional" },
                        if p.secret {
                            "PRIVATE — masked, transient, never saved"
                        } else {
                            "public — saved with invocation"
                        },
                        p.description,
                        p.minimum,
                        p.maximum,
                        p.max_length
                    ));
                    if !p.choices.is_empty() {
                        text.push_str(&format!("Choices: {}\n", Value::Array(p.choices.clone())));
                    }
                    if p.secret {
                        text.push_str("Value: [hidden]\n");
                    } else {
                        text.push_str(&format!("Value: {}\n", *panel.editor));
                    }
                    text.push_str("Enter validates and advances. Optional blank input is omitted.\nPrivate entry expires after five minutes; focus loss discards it.\n");
                    if p.secret && !p.required {
                        text.push_str("Optional private values are unsupported by the current host preview API; leave omitted.\n");
                    }
                }
                Phase::Preview => text.push_str("Waiting for executing-host preview…\n"),
                Phase::Confirm => text.push_str(&panel.preview),
                Phase::Inventory => {}
            }
        }
    }
    text.push_str(&format!(
        "\n\n{}\nPgUp/PgDn scroll · Esc cancel",
        panel.notice
    ));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(super::safe(&text))
            .wrap(Wrap { trim: false })
            .scroll((panel.scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Saved workflows · private input isolated"),
            ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        serde_json::from_value(serde_json::json!({
            "scope":"repository", "digest":"a".repeat(64),
            "document": {"schema_version":1, "id":"review-code", "version":"1",
                "description":"Review", "prompt":"Review {{count}} using {{token}}",
                "parameters":{
                    "count":{"type":"integer","required":true,"minimum":1,"maximum":3,"default":2},
                    "token":{"type":"string","secret":true,"required":true,"max_length":32}
                }}
        }))
        .unwrap()
    }
    fn panel() -> Panel {
        Panel {
            target: Target {
                route: super::super::state::Route {
                    id: Uuid::new_v4(),
                    generation: 1,
                },
                session: Uuid::new_v4(),
            },
            incarnation: Uuid::new_v4(),
            host: "remote".into(),
            entries: vec![entry()],
            selected: 0,
            phase: Phase::Inventory,
            fields: vec![],
            field: 0,
            public: BTreeMap::new(),
            private: BTreeMap::new(),
            editor: Zeroizing::new(String::new()),
            preview: String::new(),
            revision: 7,
            notice: String::new(),
            scroll: 0,
            deadline: Instant::now() + Duration::from_secs(300),
            request: None,
        }
    }
    #[test]
    fn trust_and_submit_require_unmodified_nonrepeat_keypress() {
        use crossterm::event::KeyEvent;
        assert!(plain_activation(&KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE
        )));
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        ] {
            assert!(!plain_activation(&KeyEvent::new(
                KeyCode::Char('y'),
                modifiers
            )));
        }
        let mut key = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Repeat;
        assert!(!plain_activation(&key));
    }
    #[test]
    fn typed_inputs_validate_bounds_and_choices_without_echoing_values() {
        let e = entry();
        let p = &e.document.parameters["count"];
        assert!(validate_input(p, "1").is_ok());
        assert!(validate_input(p, "4").is_err());
        assert!(
            validate_input(p, "secret-sentinel")
                .unwrap_err()
                .to_string()
                .find("sentinel")
                .is_none()
        );
        let mut p = p.clone();
        p.kind = ParameterType::Boolean;
        p.minimum = None;
        p.maximum = None;
        p.choices = vec![Value::Bool(true)];
        assert!(validate_input(&p, "true").is_ok());
        assert!(validate_input(&p, "false").is_err());
        assert!(validate_input(&p, "yes").is_err());
    }
    #[test]
    fn editor_segregates_private_values_and_preserves_public_defaults() {
        let mut p = panel();
        p.select();
        p.phase = Phase::Inputs;
        p.load_field();
        assert_eq!(&*p.editor, "2");
        p.commit_field().unwrap();
        p.field = 1;
        p.load_field();
        p.editor.push_str("private-sentinel");
        p.commit_field().unwrap();
        assert_eq!(p.public["count"], "2");
        assert!(!p.public.contains_key("token"));
        assert_eq!(&*p.private["token"], "private-sentinel");
        assert!(p.editor.is_empty());
        let preview = serde_json::to_string(&p.preview_command()).unwrap();
        assert!(!preview.contains("private-sentinel"));
        let pending = p.pending(42, Some(Uuid::new_v4()));
        let bytes = serde_json::to_string(&pending).unwrap();
        assert!(!bytes.contains("private-sentinel"));
        assert!(pending.draft.is_empty());
        assert!(pending.preserve_draft);
        p.clear_inputs();
        assert!(p.private.is_empty());
        assert!(p.public.is_empty());
    }
    #[test]
    fn exact_public_envelope_survives_restart_for_resolution_not_replay() {
        let p = panel();
        let pending = p.pending(42, Some(Uuid::new_v4()));
        let restored: Pending =
            serde_json::from_str(&serde_json::to_string(&pending).unwrap()).unwrap();
        match restored.resolution() {
            VoyageCommand::Resolve {
                command_id,
                original: Some(original),
            } => {
                assert_eq!(command_id, pending.command_id);
                assert_eq!(
                    serde_json::to_value(original).unwrap(),
                    serde_json::to_value(pending.original).unwrap()
                );
            }
            _ => panic!("must reconcile exact envelope, not submit"),
        }
    }
    #[test]
    fn stale_digest_preview_is_never_confirmable() {
        let mut p = panel();
        p.phase = Phase::Preview;
        let mut definition = serde_json::to_value(&p.entry().document).unwrap();
        definition["id"] = "review-code".into();
        let response = serde_json::json!({"definition":{"scope":"repository","digest":"b".repeat(64),"document":definition},"prompt":"changed"});
        assert!(apply_response(&mut p, Ok(response)).is_err());
        assert!(p.phase == Phase::Preview);
        assert!(p.preview.is_empty());
    }
    #[test]
    fn host_preview_must_match_scope_id_and_digest() {
        let mut p = panel();
        p.phase = Phase::Preview;
        let response = serde_json::json!({"definition":{"scope":"repository","digest":p.entry().digest,"document":p.entry().document},"prompt":"host-rendered public prompt"});
        apply_response(&mut p, Ok(response)).unwrap();
        assert!(p.phase == Phase::Confirm);
        assert_eq!(p.preview, "host-rendered public prompt");
    }
    #[test]
    fn optional_private_binding_refuses_preview_mismatch() {
        let mut p = panel();
        p.select();
        p.field = 1;
        p.entries[0]
            .document
            .parameters
            .get_mut("token")
            .unwrap()
            .required = false;
        p.editor.push_str("private-sentinel");
        let error = p.commit_field().unwrap_err().to_string();
        assert!(error.contains("not supported"));
        assert!(!error.contains("private-sentinel"));
        assert!(p.private.is_empty());
        p.editor = Zeroizing::new(String::new());
        assert!(p.commit_field().is_ok());
    }
    #[test]
    fn malformed_inventory_and_transport_failure_do_not_fabricate_success() {
        let mut p = panel();
        assert!(
            apply_response(
                &mut p,
                Ok(serde_json::json!({"section":"tools","value":[]}))
            )
            .is_err()
        );
        assert!(apply_response(&mut p, Err("Host unavailable")).is_err());
        assert!(p.phase == Phase::Inventory);
        apply_response(
            &mut p,
            Ok(serde_json::json!({"section":"workflows","value":[]})),
        )
        .unwrap();
        assert!(p.entries.is_empty());
        assert!(p.notice.contains("No saved workflows"));
    }
}
