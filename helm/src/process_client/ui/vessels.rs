//! Private human-only connection setup. Route ALL terminal events here while open,
//! including paste. Neither setup input nor backend diagnostics belong in chat/logs.
mod app;
mod failure;
mod input;
pub(super) use app::classify_error;
use failure::SetupFailure;
mod render;

use crate::process_client::connections::{
    Connection, ConnectionPreview, Preferences, Registry, Scope,
};
use crossterm::event::Event as TerminalEvent;
use ratatui::{Frame, layout::Rect};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;
use zeroize::Zeroizing;

pub enum Action {
    Activate(Connection),
    ConnectLocal,
    DisconnectLocal,
    Disconnect(Uuid),
    Forget(Uuid),
    New(Option<Uuid>),
    Filter(Option<Uuid>),
    FilterLocal,
}
#[derive(Clone, Copy, Debug)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Offline,
    AccessExpired,
    AccessRevoked,
    AccessUnavailable,
    IdentityChanged,
    UnsupportedVersion,
}
impl ConnectionState {
    fn label(self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Offline => "Offline",
            Self::AccessExpired => "Access expired",
            Self::AccessRevoked => "Access revoked",
            Self::AccessUnavailable => "Access unavailable (expired or revoked)",
            Self::IdentityChanged => "Identity changed",
            Self::UnsupportedVersion => "Unsupported version",
        }
    }
}
// Intentionally not Debug: never expose private backend diagnostics.
pub struct Event {
    operation: Uuid,
    result: Result<ConnectionPreview, SetupFailure>,
}
#[derive(Clone, Copy)]
enum Button {
    Add,
    Import,
    Connect,
    Disconnect,
    Rename,
    Replace,
    Auto,
    Forget,
    Restore,
    Resume,
    Submit,
    Save,
    Back,
    New,
    Filter,
    All,
    Field(usize),
    Select(usize),
}
enum Page {
    List,
    Form {
        import: bool,
        replacement: Option<Uuid>,
    },
    Preview {
        preview: Box<ConnectionPreview>,
        replacement: Option<Uuid>,
    },
    Rename(Uuid),
    Forget(Uuid),
}
struct Panel {
    open: bool,
    page: Page,
    selected: usize,
    scroll: usize,
    reveal_selection: bool,
    field: usize,
    fields: [Zeroizing<String>; 3],
    autoconnect: bool,
    notice: String,
    busy: Option<(Uuid, Option<Uuid>)>,
    hits: Vec<(Rect, Button)>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            page: Page::List,
            selected: 0,
            scroll: 0,
            reveal_selection: false,
            field: 0,
            fields: std::array::from_fn(|_| Zeroizing::new(String::new())),
            autoconnect: true,
            notice: String::new(),
            busy: None,
            hits: Vec::new(),
        }
    }
}
pub struct Manager {
    registry: Arc<Registry>,
    principal: Option<Uuid>,
    local_id: Option<Uuid>,
    records: Vec<Connection>,
    pending: Vec<Uuid>,
    states: HashMap<Uuid, (u64, ConnectionState)>,
    panel: Panel,
    jobs: Vec<tokio::task::JoinHandle<()>>,
}
impl Manager {
    pub fn open(root: PathBuf) -> anyhow::Result<Self> {
        let registry = Arc::new(Registry::open(root)?);
        let principal = registry.principal_id().ok();
        let mut manager = Self {
            registry,
            principal,
            local_id: None,
            records: Vec::new(),
            pending: Vec::new(),
            states: HashMap::new(),
            panel: Panel::default(),
            jobs: Vec::new(),
        };
        manager.reload()?;
        Ok(manager)
    }
    pub fn stop_tasks(&mut self) -> Vec<tokio::task::JoinHandle<()>> {
        let jobs = std::mem::take(&mut self.jobs);
        for job in &jobs {
            job.abort();
        }
        jobs
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    /// Persist only an explicitly selected, authorized remote workspace.
    pub fn remember_workspace(&mut self, id: Uuid, workspace: Uuid) -> anyhow::Result<()> {
        self.reload()?;
        let c = self
            .records
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| anyhow::anyhow!("Saved connection is unavailable"))?;
        anyhow::ensure!(
            matches!(&c.scope, Scope::Workspaces { workspace_ids } if workspace_ids.contains(&workspace)),
            "Workspace is not authorized by this connection"
        );
        self.registry.update(
            id,
            c.revision,
            Preferences {
                alias: c.alias.clone(),
                autoconnect: c.autoconnect,
                workspace_preference: Some(workspace),
            },
        )?;
        self.reload()?;
        Ok(())
    }
    pub fn set_local(&mut self, id: Uuid) {
        self.local_id = Some(id);
    }
    pub fn records(&self) -> &[Connection] {
        &self.records
    }
    pub fn autoconnect(&self) -> impl Iterator<Item = &Connection> {
        self.records
            .iter()
            .filter(|c| c.autoconnect && !c.forgotten)
    }
    pub fn is_open(&self) -> bool {
        self.panel.open
    }
    pub fn open_panel(&mut self) {
        self.panel.open = true;
        if self.reload().is_err() {
            self.panel.notice =
                "Cannot read private connections. Check storage permissions.".into();
        }
    }
    /// Caller must first validate its current Route generation.
    pub fn set_state(&mut self, id: Uuid, generation: u64, state: ConnectionState) {
        if self
            .states
            .get(&id)
            .is_none_or(|(old, _)| generation >= *old)
        {
            self.states.insert(id, (generation, state));
        }
    }
    pub fn handle_event(
        &mut self,
        event: &TerminalEvent,
        sender: &mpsc::Sender<super::observe::Update>,
    ) -> Vec<Action> {
        self.input(event, sender)
    }
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.draw(frame, area);
    }
    fn reload(&mut self) -> anyhow::Result<()> {
        let snapshot = self.registry.load()?;
        self.records = snapshot.connections;
        self.pending = snapshot
            .pending_pairs
            .into_iter()
            .filter(|p| !p.completed)
            .map(|p| p.id)
            .collect();
        self.panel.selected = self
            .panel
            .selected
            .min(self.records.len() + self.pending.len());
        Ok(())
    }
    fn selected(&self) -> Option<&Connection> {
        self.panel
            .selected
            .checked_sub(1)
            .and_then(|i| self.records.get(i))
    }
    fn clear_fields(&mut self) {
        self.panel.fields = std::array::from_fn(|_| Zeroizing::new(String::new()));
        self.panel.field = 0;
    }
    fn back(&mut self) {
        self.clear_fields();
        self.panel.page = Page::List;
        self.panel.scroll = 0;
    }
    pub fn apply(&mut self, event: Event) -> Vec<Action> {
        if self.panel.busy.map(|b| b.0) != Some(event.operation) {
            return Vec::new();
        }
        let replacement = self.panel.busy.take().and_then(|b| b.1);
        match event.result {
            Ok(preview) => {
                self.panel.page = Page::Preview {
                    preview: Box::new(preview),
                    replacement,
                };
                self.panel.scroll = 0;
                self.panel.notice =
                    "Review authenticated identity and authority before saving.".into();
            }
            Err(error) => {
                self.panel.notice = format!(
                    "{} Recover pending pairing after an uncertain response; do not redeem twice.",
                    error.message()
                );
            }
        }
        if self.reload().is_err() {
            self.panel
                .notice
                .push_str(" Private registry reload failed.");
        }
        Vec::new()
    }
    fn prepare(&mut self, sender: &mpsc::Sender<super::observe::Update>, resume: Option<Uuid>) {
        if self.panel.busy.is_some() {
            return;
        }
        let (import, replacement) = match self.panel.page {
            Page::Form {
                import,
                replacement,
            } => (import, replacement),
            _ if resume.is_some() => (false, None),
            _ => return,
        };
        if resume.is_none() && self.panel.fields[0].trim().is_empty() {
            self.panel.notice = "Enter an HTTPS endpoint or access-file path.".into();
            return;
        }
        if resume.is_none()
            && !import
            && (!self.panel.fields[0].starts_with("https://") || self.panel.fields[1].is_empty())
        {
            self.panel.notice = "Pairing requires HTTPS and an owner-issued invitation.".into();
            return;
        }
        let registry = Arc::clone(&self.registry);
        let first = Zeroizing::new(std::mem::take(&mut *self.panel.fields[0]));
        let invitation = Zeroizing::new(std::mem::take(&mut *self.panel.fields[1]));
        let operation = Uuid::new_v4();
        self.panel.busy = Some((operation, replacement));
        self.panel.notice =
            "Validating privately… Closing does not cancel pairing; pending recovery is retained."
                .into();
        let sender = sender.clone();
        let mut running = Vec::new();
        for job in self.jobs.drain(..) {
            if job.is_finished() {
                use futures_util::FutureExt;
                let _ = job.now_or_never();
            } else {
                running.push(job);
            }
        }
        self.jobs = running;
        let job = tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                if let Some(id) = resume {
                    registry.resume_pair(id).await
                } else if import {
                    registry.prepare_import(PathBuf::from(first.as_str())).await
                } else {
                    registry
                        .prepare_pair(first.to_string(), invitation.to_string())
                        .await
                }
            })
            .await;
            let result = match result {
                Ok(Ok(preview)) => Ok(preview),
                Ok(Err(error)) => Err(SetupFailure::from_error(&error)),
                Err(_) => Err(SetupFailure::Timeout),
            };
            let _ = sender
                .send(super::observe::Update::Vessels(Event { operation, result }))
                .await;
        });
        self.jobs.push(job);
    }
    fn preferences(&mut self, id: Uuid, alias: Option<String>, toggle: bool) {
        let Some(c) = self.records.iter().find(|c| c.id == id) else {
            return;
        };
        let preferences = Preferences {
            alias: alias.unwrap_or_else(|| c.alias.clone()),
            autoconnect: if toggle {
                !c.autoconnect
            } else {
                c.autoconnect
            },
            workspace_preference: c.workspace_preference.clone(),
        };
        self.panel.notice = match self.registry.update(id, c.revision, preferences) {
            Ok(_) => "Preferences saved.".into(),
            Err(_) => {
                "Not saved: storage unavailable or changed in another window. Reload and retry."
                    .into()
            }
        };
        let _ = self.reload();
    }
}
fn text(value: &str) -> String {
    value.chars().filter(|c| !c.is_control() && !matches!(c, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect()
}
