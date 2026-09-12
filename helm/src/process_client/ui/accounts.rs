//! Human-only account controls. Host credentials are never resolved by Helm.
//! Sensitive replies use a private one-shot, not the ordinary observer/event queue.
mod render;
mod storage;
mod usage;
use super::{
    App, Event, KeyCode, KeyModifiers, Result, drafts,
    inference::{Destination, Settings},
    safe,
    state::{Pending, Route},
};
use anyhow::{Context, ensure};
use serde::Deserialize;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use uuid::Uuid;
use voyage_protocol::{
    accounts::*,
    vessel::{VesselCommand, VoyageCommand},
};

#[derive(Default)]
pub(super) struct Controls {
    picker: Option<Picker>,
    labels: std::collections::BTreeMap<(Route, Uuid), String>,
    hosts: std::collections::BTreeMap<Route, Uuid>,
    // No Debug/serialization and no Update variant: this channel is private to the view.
    reply: Option<(Uuid, oneshot::Receiver<Result<Reply>>)>,
    usage_reply: Option<(Uuid, oneshot::Receiver<Result<Reply>>)>,
    pub(super) initializing: std::collections::BTreeSet<Uuid>,
    hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, usize)>>,
    visible: std::cell::Cell<bool>,
    browser_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Controls {
    pub(super) fn open(&self) -> bool {
        self.picker.is_some()
    }
}
#[derive(Deserialize)]
struct Catalogue {
    accounts: Vec<AccountDescriptor>,
    connections: Vec<ConnectionDescriptor>,
    #[serde(default)]
    default_account: Option<AccountBinding>,
    #[serde(default)]
    default_revision: u64,
    #[serde(default)]
    can_set_default: bool,
}
struct Loaded {
    host: Uuid,
    catalogue: Catalogue,
    defaults: Settings,
    enroll: bool,
}
enum Reply {
    Loaded(Loaded),
    Models(AccountBinding, Vec<crate::provider::ModelInfo>),
    Private(PrivateEnrollmentStatus),
    Mutation,
    Catalogue(Catalogue, Option<Uuid>),
    Usage(AccountUsageObservation),
    DefaultAccount(Catalogue),
}
enum Mode {
    List,
    Connections,
    Confirm(Settings),
    Alias(Uuid),
    Enrollment,
}
struct Picker {
    id: Uuid,
    destination: Destination,
    route: Route,
    workspace: PathBuf,
    host: Option<Uuid>,
    _lock: Option<std::fs::File>,
    catalogue: Catalogue,
    original: Settings,
    mode: Mode,
    query: String,
    selected: usize,
    edit: usize,
    usage: std::collections::HashMap<Uuid, AccountUsageObservation>,
    usage_requested: std::collections::HashSet<Uuid>,
    default_change_pending: bool,
    enroll: bool,
    notice: String,
    busy: bool,
    intent: Option<storage::Intent>,
    private: Option<PrivateEnrollmentStatus>,
    outcome: Option<EnrollmentState>,
    retry: Option<(Uuid, String)>,
    restart: bool,
    poll: Instant,
    connection: tokio::sync::watch::Receiver<crate::process_client::duplex::ConnectionState>,
    loss_generation: u64,
    disconnected: bool,
    models: Vec<crate::provider::ModelInfo>,
}
fn now() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}
fn enrollment_notice(v: &PrivateEnrollmentStatus) -> String {
    let message = match v.status.state {
        EnrollmentState::Starting => "Requesting a sign-in code…",
        EnrollmentState::Pending => "Waiting for you to enter the code in your browser.",
        EnrollmentState::Exchanging => "Checking browser approval and completing sign-in…",
        EnrollmentState::Succeeded => {
            "Signed in. Refreshing accounts; selection still requires confirmation."
        }
        EnrollmentState::Cancelled => "Sign-in cancelled. N requests a new sign-in.",
        EnrollmentState::Expired => "The code expired. N requests a new sign-in.",
        EnrollmentState::Denied => "Sign-in was denied. N requests a new sign-in.",
        EnrollmentState::Uncertain => "Sign-in stopped before completion could be confirmed.",
    };
    let mut text = message.to_owned();
    if let Some(failure) = &v.failure {
        let phase = match failure.phase {
            EnrollmentPhase::RequestCode => "requesting code",
            EnrollmentPhase::Poll => "checking browser approval",
            EnrollmentPhase::Exchange => "exchanging authorization",
            EnrollmentPhase::Publication => "saving account",
        };
        let kind = match failure.kind {
            EnrollmentFailureKind::Timeout => "timed out",
            EnrollmentFailureKind::Connection => "connection failed",
            EnrollmentFailureKind::Rejected => "provider rejected the request",
            EnrollmentFailureKind::InvalidResponse => "unexpected provider response",
            EnrollmentFailureKind::Storage => "account could not be saved",
            EnrollmentFailureKind::Interrupted => "operation was interrupted",
        };
        text.push_str(&format!(" Failed while {phase}: {kind}"));
        if let Some(status) = failure.http_status.filter(|s| (100..=599).contains(s)) {
            text.push_str(&format!(" (HTTP {status})"));
        }
        text.push('.');
    }
    text
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn selectable(a: &AccountDescriptor) -> bool {
    a.state == AccountState::Ready && a.availability == CredentialAvailability::Available
}
fn provider(t: Transport) -> &'static str {
    match t {
        Transport::ChatgptOauth => "chatgpt-oauth",
        Transport::OpenaiResponses => "openai-responses",
        Transport::OpenaiChat => "openai-chat",
        Transport::Anthropic => "anthropic",
    }
}
fn enrollment_id(intent: &storage::Intent) -> Option<Uuid> {
    match intent.command {
        VesselCommand::EnrollAccount { enrollment_id, .. } => Some(enrollment_id),
        _ => None,
    }
}
fn active_material(p: &PrivateEnrollmentStatus) -> bool {
    matches!(
        p.status.state,
        EnrollmentState::Pending | EnrollmentState::Exchanging
    ) && p.status.expires_at > now()
        && p.verification_uri.as_deref() == Some("https://auth.openai.com/codex/device")
        && p.user_code.as_ref().is_some_and(|c| {
            !c.is_empty()
                && c.len() <= 64
                && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
// Only the explicit O action calls this fixed provider URL. Test substitution is
// at the OS effect boundary; no network/provider or real browser is used by fixtures.
fn open_browser(pending: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    use std::sync::atomic::Ordering;
    ensure!(
        !pending.swap(true, Ordering::AcqRel),
        "Browser launcher is still finishing; use the displayed URL manually"
    );
    const URL: &str = "https://auth.openai.com/codex/device";
    #[cfg(test)]
    {
        pending.store(false, Ordering::Release);
        return super::account_test_support::browser(URL);
    }
    #[cfg(all(not(test), any(target_os = "linux", target_os = "macos")))]
    {
        #[cfg(target_os = "linux")]
        let mut cmd = tokio::process::Command::new("xdg-open");
        #[cfg(target_os = "macos")]
        let mut cmd = tokio::process::Command::new("open");
        let mut child = cmd
            .kill_on_drop(true)
            .arg(URL)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| {
                pending.store(false, Ordering::Release);
                anyhow::anyhow!("Browser could not be opened; use the displayed URL manually")
            })?;
        let pending = pending.clone();
        tokio::spawn(async move {
            let observed = matches!(
                tokio::time::timeout(Duration::from_secs(10), child.wait()).await,
                Ok(Ok(_))
            );
            // Bound only our launcher, not the user's browser/navigation. A
            // failed/unobserved reap keeps the repeated-open guard fail-closed.
            let reaped = observed
                || matches!(
                    tokio::time::timeout(Duration::from_secs(2), child.kill()).await,
                    Ok(Ok(()))
                );
            if reaped {
                pending.store(false, Ordering::Release);
            }
            // kill_on_drop also protects runtime shutdown/cancellation.
        });
        Ok(())
    }
    #[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos"))))]
    {
        pending.store(false, Ordering::Release);
        anyhow::bail!(
            "Open the displayed provider URL manually; automatic browser launch is unavailable on this platform"
        )
    }
}
impl Picker {
    fn choices(&self) -> Vec<(String, Option<AccountBinding>)> {
        let mut items = Vec::new();
        for a in &self.catalogue.accounts {
            if !format!("{} {}", a.alias, a.label)
                .to_lowercase()
                .contains(&self.query.to_lowercase())
            {
                continue;
            }
            if let Some(c) = self
                .catalogue
                .connections
                .iter()
                .find(|c| c.id == a.connection_id)
            {
                for &t in &c.transports {
                    items.push((
                        format!(
                            "{}    {}    {}{}",
                            safe(&a.label),
                            if c.transports.len() > 1 {
                                format!("{} ({})", safe(&c.label), provider(t))
                            } else {
                                safe(&c.label)
                            },
                            match (a.state, a.availability) {
                                (AccountState::Ready, CredentialAvailability::Available) =>
                                    "Signed in",
                                (AccountState::SignInRequired, _) => "Sign in required",
                                (AccountState::Removed, _) => "Removed",
                                _ => "Credentials unavailable",
                            },
                            if self
                                .original
                                .account
                                .as_ref()
                                .is_some_and(|b| b.account_id == a.id && b.transport == t)
                            {
                                " · Current"
                            } else {
                                ""
                            }
                        ),
                        selectable(a).then(|| AccountBinding {
                            account_id: a.id,
                            connection_id: c.id,
                            identity_generation: a.identity_generation,
                            connection_revision: c.revision,
                            transport: t,
                        }),
                    ));
                }
            }
        }
        items.push(("+ Sign in to another account".into(), None));
        items.push(("Set up an API account…".into(), None));
        items
    }
    fn label(&self, s: &Settings) -> String {
        s.account
            .as_ref()
            .map(|b| {
                self.catalogue
                    .accounts
                    .iter()
                    .find(|a| a.id == b.account_id)
                    .map(|a| safe(&a.label))
                    .unwrap_or_else(|| b.account_id.to_string())
            })
            .unwrap_or_else(|| "No account selected".into())
    }
}
impl App {
    pub(super) fn account_host(&self, route: Route) -> Option<Uuid> {
        self.accounts.hosts.get(&route).copied()
    }
    pub(super) fn account_control_label(
        &self,
        destination: Destination,
        settings: &Settings,
    ) -> String {
        let route = match self.account_destination(destination) {
            Ok((r, _)) => r,
            Err(_) => return "unavailable".into(),
        };
        let label = settings
            .account
            .as_ref()
            .map(|a| {
                self.accounts
                    .labels
                    .get(&(route, a.account_id))
                    .cloned()
                    .unwrap_or_else(|| a.account_id.to_string())
            })
            .unwrap_or_else(|| "unresolved — review".into());
        if let Destination::Live(t) = destination {
            if let Some(current) = self
                .views
                .get(&t)
                .and_then(|v| v.snapshot.as_ref())
                .and_then(|s| s.inference_current.as_ref())
                .and_then(|s| s.account.as_ref())
            {
                if settings.account.as_ref() != Some(current) {
                    let running = self
                        .accounts
                        .labels
                        .get(&(route, current.account_id))
                        .cloned()
                        .unwrap_or_else(|| current.account_id.to_string());
                    return format!("Running {running}; next {label}");
                }
            }
        }
        label
    }
    pub(super) fn account_destination(&self, destination: Destination) -> Result<(Route, PathBuf)> {
        Ok(match destination {
            Destination::Draft(id) => {
                let d = self.new_drafts.get(&id).context("draft unavailable")?;
                (d.route, d.saved.workspace.clone())
            }
            Destination::Live(t) => (
                t.route,
                self.views
                    .get(&t)
                    .context("voyage unavailable")?
                    .process
                    .workspace
                    .clone(),
            ),
        })
    }
    pub(super) fn open_accounts(&mut self, destination: Destination, query: &str) -> Result<()> {
        ensure!(
            self.accounts.reply.is_none(),
            "An account request is finishing; reopen shortly. No operation repeated"
        );
        let (route, workspace) = self.account_destination(destination)?;
        ensure!(
            self.clients.available(route),
            "Vessel unavailable. Reconnect its original authenticated connection; selection retained"
        );
        if let Destination::Live(t) = destination {
            let v = &self.views[&t];
            ensure!(
                v.pending.is_none() && !v.archived() && !v.deleted(),
                "Resolve pending commands / restore voyage before selecting an account"
            );
        }
        if let Destination::Draft(id) = destination {
            let d = &self.new_drafts[&id];
            ensure!(
                d.saved.start.is_none() && !d.busy,
                "Creation pending; exact original account retained"
            );
        }
        let original = match destination {
            Destination::Draft(_) => self.inference_settings(destination).unwrap_or_default(),
            Destination::Live(_) => self.inference_settings(destination)?,
        };
        self.accounts.visible.set(false);
        self.accounts.hits.borrow_mut().clear();
        let id = Uuid::new_v4();
        let connection = self.clients[route].connection_state();
        let loss_generation = connection.borrow().loss_generation;
        self.accounts.picker = Some(Picker {
            id,
            destination,
            route,
            workspace: workspace.clone(),
            host: None,
            _lock: None,
            catalogue: Catalogue {
                accounts: vec![],
                connections: vec![],
                default_account: None,
                default_revision: 0,
                can_set_default: false,
            },
            original,
            mode: Mode::List,
            query: query.into(),
            selected: 0,
            edit: 0,
            usage: Default::default(),
            usage_requested: Default::default(),
            default_change_pending: false,
            enroll: false,
            notice: "Reading authenticated executing-host accounts…".into(),
            busy: true,
            intent: None,
            private: None,
            outcome: None,
            retry: None,
            restart: false,
            poll: Instant::now(),
            connection,
            loss_generation,
            disconnected: false,
            models: vec![],
        });
        let client = self.clients[route].clone();
        let (tx, rx) = oneshot::channel();
        self.accounts.reply = Some((id, rx));
        tokio::spawn(async move {
            let result: Result<Reply> = async {
                let caps = client.request(VesselCommand::Capabilities).await?;
                ensure!(
                    caps["features"]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|f| f == "provider_accounts")),
                    "This Vessel needs an account-aware upgrade"
                );
                let host: Uuid = serde_json::from_value(caps["vessel_id"].clone())?;
                ensure!(
                    !host.is_nil() && client.managed().is_none_or(|c| c.vessel_id == host),
                    "Authenticated Vessel identity mismatch"
                );
                let catalogue = serde_json::from_value(
                    client
                        .request(VesselCommand::Accounts {
                            workspace: workspace.clone(),
                            transport: None,
                        })
                        .await?,
                )?;
                let defaults = client
                    .request(VesselCommand::AccountDefaults { workspace })
                    .await
                    .and_then(|v| {
                        if v["code"] == "default_account_required" {
                            Ok(Settings::default())
                        } else {
                            Ok(serde_json::from_value(v)?)
                        }
                    })
                    .unwrap_or_default();
                let enroll = caps["features"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|f| f == "private_account_enrollment"))
                    && (client.is_local()
                        || caps["rights"]
                            .as_array()
                            .is_some_and(|v| v.iter().any(|f| f == "account_enroll")));
                Ok(Reply::Loaded(Loaded {
                    host,
                    catalogue,
                    defaults,
                    enroll,
                }))
            }
            .await;
            let _ = tx.send(result.map_err(|_| anyhow::anyhow!("Accounts unavailable or access denied. Reconnect/refresh access; host owner must grant account-use or device-enrollment access. No selection changed.")));
        });
        Ok(())
    }
    // Called from the regular UI tick; private responses never reach general updates.
    fn account_connection_tick(&mut self) {
        let Some(p) = self.accounts.picker.as_mut() else {
            return;
        };
        let state = *p.connection.borrow();
        if state.loss_generation != p.loss_generation
            || (!p.disconnected && !self.clients.available(p.route))
        {
            p.loss_generation = state.loss_generation;
            p.disconnected = true;
            p.private = None;
            p.usage.clear();
            p.busy = false;
            // Invalidate even an already queued one-shot. A reconnect cannot authorize
            // material fetched on an earlier socket, nor replay an enrollment mutation.
            p.id = Uuid::new_v4();
            self.accounts.reply = None;
            self.accounts.usage_reply = None;
            p.notice = "Connection lost. Sensitive material cleared. Enter reloads accounts when connected; Ctrl+G opens Vessels to reconnect.".into();
        }
    }
    pub(super) fn account_tick(&mut self) {
        self.account_connection_tick();
        if let Some((id, mut rx)) = self.accounts.usage_reply.take() {
            match rx.try_recv() {
                Ok(result) => {
                    let busy = self.accounts.picker.as_ref().is_some_and(|p| p.busy);
                    let notice = self.accounts.picker.as_ref().map(|p| p.notice.clone());
                    let _ = self.account_reply(id, result);
                    if let Some(p) = self.accounts.picker.as_mut() {
                        p.busy = busy;
                        if !matches!(p.mode, Mode::List) || busy {
                            if let Some(notice) = notice {
                                p.notice = notice;
                            }
                        }
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.accounts.usage_reply = Some((id, rx))
                }
                Err(_) => (),
            }
        }
        if let Some((id, mut rx)) = self.accounts.reply.take() {
            match rx.try_recv() {
                Ok(result) => {
                    if let Err(e) = self.account_reply(id, result) {
                        if let Some(p) = self.accounts.picker.as_mut() {
                            p.busy = false;
                            p.notice = safe(&e.to_string());
                        }
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => self.accounts.reply = Some((id, rx)),
                Err(_) => {
                    if let Some(p) = self.accounts.picker.as_mut() {
                        p.busy = false;
                        p.private = None;
                        p.notice = "Request interrupted. Original enrollment identity retained; reopen to inspect, not retry.".into();
                    }
                }
            }
        }
        if let Some(p) = self.accounts.picker.as_mut() {
            if !self.clients.available(p.route) {
                p.private = None;
                return;
            }
            if p.private
                .as_ref()
                .is_some_and(|v| v.status.expires_at <= now())
            {
                p.private = None;
            }
            if matches!(p.mode, Mode::List)
                && !p.busy
                && !p.disconnected
                && self.accounts.usage_reply.is_none()
            {
                let selected = p.choices().get(p.selected).and_then(|(_, b)| b.clone());
                if selected.is_some_and(|b| !p.usage_requested.contains(&b.account_id)) {
                    let _ = self.refresh_account_usage(false);
                    return;
                }
            }
            if matches!(p.mode, Mode::Enrollment)
                && !p.busy
                && !p.disconnected
                && p.poll.elapsed() >= Duration::from_secs(3)
            {
                let _ = self.poll_account_enrollment();
            }
        } else if self.accounts.reply.is_none() {
            if let Some(id) = self.active_draft {
                if self.new_drafts.get(&id).is_some_and(|d| {
                    d.saved.account_settings.is_none() && d.saved.start.is_none() && !d.busy
                }) && self.accounts.initializing.insert(id)
                {
                    // The unresolved draft is visibly reviewed, never silently retargeted later.
                    let _ = self.open_accounts(Destination::Draft(id), "");
                }
            }
        }
    }
    fn account_reply(&mut self, id: Uuid, result: Result<Reply>) -> Result<()> {
        let Some(mut p) = self.accounts.picker.take() else {
            return Ok(());
        };
        if p.id != id
            || p.disconnected
            || p.connection.borrow().loss_generation != p.loss_generation
            || !self.clients.current(p.route)
        {
            self.accounts.picker = Some(p);
            return Ok(());
        }
        p.busy = false;
        let mut refresh_account = None;
        let mut refresh = false;
        let result = (|| -> Result<()> {
            match result? {
                Reply::Loaded(v) => {
                    ensure!(
                        v.catalogue.accounts.len() <= 128 && v.catalogue.connections.len() <= 64,
                        "Account catalogue exceeds limits"
                    );
                    p._lock = Some(storage::lock(v.host, &p.workspace)?);
                    p.host = Some(v.host);
                    self.accounts.hosts.insert(p.route, v.host);
                    self.accounts.labels.retain(|(r, _), _| *r != p.route);
                    for a in &v.catalogue.accounts {
                        self.accounts.labels.insert((p.route, a.id), safe(&a.label));
                    }
                    p.catalogue = v.catalogue;
                    p.enroll = v.enroll;
                    let prefs = storage::load(v.host, &p.workspace)?;
                    p.intent = prefs.enrollment;
                    if let Some(i) = &p.intent {
                        ensure!(
                            i.host == v.host
                                && i.workspace == p.workspace
                                && enrollment_id(i).is_some(),
                            "Original enrollment host/workspace mismatch; retained for operator review"
                        );
                    }
                    if let Destination::Draft(d) = p.destination {
                        let saved = &self
                            .new_drafts
                            .get(&d)
                            .context("Draft closed while reading accounts")?
                            .saved;
                        ensure!(
                            saved.account_host.is_none_or(|h| h == v.host),
                            "Vessel identity changed; create a new draft after review"
                        );
                        if saved.account_settings.is_none() {
                            let explicit = self.draft_account_seed(d);
                            p.original = if let Some(mut seed) = explicit {
                                if seed.account.is_none() && seed.provider == v.defaults.provider {
                                    seed.account = v.defaults.account.clone();
                                }
                                seed
                            } else {
                                v.defaults
                            };
                            // Legacy per-Helm remembered accounts no longer override the host default.
                            self.set_draft_account(d, v.host, p.original.clone())?;
                        }
                    }
                    if p.intent.is_some() {
                        p.mode = Mode::Enrollment;
                        p.poll = Instant::now() - Duration::from_secs(4);
                    }
                    p.notice =
                        "Choose an account for this voyage; default is set separately.".into();
                }
                Reply::Catalogue(catalogue, focus) => {
                    ensure!(
                        catalogue.accounts.len() <= 128 && catalogue.connections.len() <= 64,
                        "Account catalogue exceeds limits"
                    );
                    self.accounts
                        .labels
                        .retain(|(route, _), _| *route != p.route);
                    for account in &catalogue.accounts {
                        self.accounts
                            .labels
                            .insert((p.route, account.id), safe(&account.label));
                    }
                    p.catalogue = catalogue;
                    p.usage.clear();
                    p.usage_requested.clear();
                    p.mode = Mode::List;
                    p.query.clear();
                    p.selected = p
                        .choices()
                        .iter()
                        .position(|(_, binding)| {
                            binding
                                .as_ref()
                                .is_some_and(|binding| Some(binding.account_id) == focus)
                        })
                        .unwrap_or(0);
                    p.notice = "Signed in. Select the account and review its model to use it. Your current account has not changed.".into();
                }
                Reply::Models(binding, models) => {
                    if let Mode::Confirm(s) = &p.mode {
                        if s.account.as_ref() == Some(&binding)
                            && crate::provider::validate_models(&models, &[]).is_ok()
                        {
                            p.models = models;
                        }
                    }
                }
                Reply::Usage(observation) => {
                    let valid = p.catalogue.accounts.iter().any(|a| {
                        a.id == observation.account.account_id
                            && a.identity_generation == observation.account.identity_generation
                            && a.capability_revision == observation.capability_revision
                    }) && p.catalogue.connections.iter().any(|c| {
                        c.id == observation.account.connection_id
                            && c.revision == observation.account.connection_revision
                    });
                    ensure!(
                        valid && !p.disconnected,
                        "Usage identity changed; reopen accounts"
                    );
                    p.usage.insert(observation.account.account_id, observation);
                    p.notice = "Usage observation updated. It does not guarantee access.".into();
                }
                Reply::DefaultAccount(catalogue) => {
                    p.default_change_pending = false;
                    p.catalogue = catalogue;
                    p.usage.clear();
                    p.usage_requested.clear();
                    p.notice =
                        "Default account observed from host. This voyage is unchanged.".into();
                }
                Reply::Mutation => {
                    p.poll = Instant::now() - Duration::from_secs(4);
                    p.notice = "Host operation observed; checking its private status. Enrollment does not select an account.".into();
                }
                Reply::Private(mut v) => {
                    ensure!(
                        p.intent.as_ref().and_then(enrollment_id) == Some(v.status.enrollment_id),
                        "Enrollment identity mismatch"
                    );
                    if !active_material(&v) {
                        v.user_code = None;
                        v.verification_uri = None;
                    }
                    let terminal = matches!(
                        v.status.state,
                        EnrollmentState::Succeeded
                            | EnrollmentState::Cancelled
                            | EnrollmentState::Expired
                            | EnrollmentState::Denied
                    );
                    p.outcome = Some(v.status.state);
                    p.notice = enrollment_notice(&v);
                    if let Some(storage::Intent {
                        command:
                            VesselCommand::EnrollAccount {
                                connection_id,
                                alias,
                                ..
                            },
                        ..
                    }) = &p.intent
                    {
                        p.retry = Some((*connection_id, alias.clone()));
                    }
                    if terminal {
                        p.private = None;
                        // Keep the safe outcome visible, but permit a new explicit enrollment after terminal observation.
                        let host = p.host.context("authenticated host missing")?;
                        let mut prefs = storage::load(host, &p.workspace)?;
                        prefs.enrollment = None;
                        storage::save(host, &p.workspace, &prefs)?;
                        p.intent = None;
                        if v.status.state == EnrollmentState::Succeeded {
                            p.restart = false;
                            p.retry = None;
                            refresh_account = v.status.account_id;
                            refresh = true;
                        } else if p.restart {
                            p.restart = false;
                            if let Some((connection, alias)) = &p.retry {
                                p.mode = Mode::Alias(*connection);
                                p.query = alias.clone();
                                p.notice = "Previous attempt closed. Enter requests a new code; use only that new code in the browser.".into();
                            }
                        }
                    } else {
                        p.private = Some(v);
                    }
                    p.poll = Instant::now();
                }
            }
            Ok(())
        })();
        if let Err(e) = &result {
            p.private = None;
            p.notice = safe(&e.to_string());
        }
        self.accounts.picker = Some(p);
        result?;
        if refresh {
            self.refresh_enrolled_accounts(refresh_account)?;
        }
        Ok(())
    }
    fn refresh_enrolled_accounts(&mut self, focus: Option<Uuid>) -> Result<()> {
        let p = self.accounts.picker.as_mut().context("view closed")?;
        let client = self.clients[p.route].clone();
        let workspace = p.workspace.clone();
        let host = p.host.context("host missing")?;
        let (tx, rx) = oneshot::channel();
        self.accounts.reply = Some((p.id, rx));
        p.busy = true;
        tokio::spawn(async move {
            let result: Result<Reply> = async {
                let caps = client.request(VesselCommand::Capabilities).await?;
                ensure!(
                    caps["vessel_id"] == host.to_string(),
                    "Host identity changed"
                );
                let value = client
                    .request(VesselCommand::Accounts {
                        workspace,
                        transport: None,
                    })
                    .await?;
                Ok(Reply::Catalogue(serde_json::from_value(value)?, focus))
            }
            .await;
            let _ = tx.send(result.map_err(|_| anyhow::anyhow!("Signed in, but the account list could not refresh. R retries the list; your account selection has not changed.")));
        });
        Ok(())
    }
    fn restart_enrollment(&mut self) -> Result<()> {
        let p = self.accounts.picker.as_mut().context("view closed")?;
        ensure!(
            p.enroll && !p.disconnected,
            "Reconnect before starting a new sign-in"
        );
        if p.intent.is_some() {
            p.restart = true;
            self.cancel_enrollment()
        } else if let Some((connection, alias)) = &p.retry {
            p.mode = Mode::Alias(*connection);
            p.query = alias.clone();
            p.notice = "Enter requests a new code. The previous code cannot be used.".into();
            Ok(())
        } else {
            Ok(())
        }
    }
    fn private_request(&mut self, command: VesselCommand, private: bool) -> Result<()> {
        let p = self
            .accounts
            .picker
            .as_mut()
            .context("account view closed")?;
        ensure!(
            !p.busy && self.accounts.reply.is_none(),
            "Account request is pending"
        );
        ensure!(
            self.clients.available(p.route),
            "Original Vessel disconnected"
        );
        p.busy = true;
        // A same-socket status read must not blink away a still-valid code. Any
        // mutation, authority failure or socket loss clears it independently.
        if !private || p.disconnected || p.connection.borrow().loss_generation != p.loss_generation
        {
            p.private = None;
        }
        p.disconnected = false;
        p.loss_generation = p.connection.borrow().loss_generation;
        let client = self.clients[p.route].clone();
        let host = p.host.context("authenticated host missing")?;
        let (tx, rx) = oneshot::channel();
        self.accounts.reply = Some((p.id, rx));
        tokio::spawn(async move {
            let result: Result<Reply> = async {
                let caps = client.request(VesselCommand::Capabilities).await?;
                ensure!(
                    caps["vessel_id"] == host.to_string(),
                    "Host identity changed"
                );
                let resolve = match &command {
                    VesselCommand::ResolveAccountEnrollment {
                        enrollment_id,
                        workspace,
                        ..
                    } => Some((*enrollment_id, workspace.clone())),
                    _ => None,
                };
                let mut v = client.request(command).await?;
                if private {
                    if let Some((enrollment_id, workspace)) = resolve {
                        v = client
                            .request(VesselCommand::PrivateAccountEnrollment {
                                enrollment_id,
                                workspace,
                            })
                            .await?;
                    }
                    Ok(Reply::Private(serde_json::from_value(v)?))
                } else {
                    Ok(Reply::Mutation)
                }
            }
            .await;
            // Never propagate remote diagnostics, response bodies or malformed private material.
            let _ = tx.send(result.map_err(|_: anyhow::Error| anyhow::anyhow!("Private sign-in request unavailable. Original identity retained; inspect again after reconnect. No fresh attempt was made.")));
        });
        Ok(())
    }
    fn poll_account_enrollment(&mut self) -> Result<()> {
        let p = self.accounts.picker.as_ref().context("view closed")?;
        let Some(intent) = &p.intent else {
            return Ok(());
        };
        let VesselCommand::EnrollAccount {
            command_id,
            enrollment_id,
            workspace,
            connection_id,
            alias,
            label,
        } = &intent.command
        else {
            anyhow::bail!("invalid original enrollment intent");
        };
        let command = VesselCommand::ResolveAccountEnrollment {
            command_id: *command_id,
            enrollment_id: *enrollment_id,
            workspace: workspace.clone(),
            connection_id: *connection_id,
            alias: alias.clone(),
            label: label.clone(),
        };
        self.private_request(command, true)
    }
    fn begin_enrollment(&mut self, connection: Uuid) -> Result<()> {
        let p = self.accounts.picker.as_mut().context("view closed")?;
        ensure!(
            p.enroll && p.intent.is_none(),
            "Sign-in not authorized or original attempt unresolved; use R to inspect it"
        );
        ensure!(
            valid_alias(&p.query),
            "Use a safe alias: 1–64 ASCII letters, digits, underscore or hyphen"
        );
        let host = p.host.context("host missing")?;
        let command = VesselCommand::EnrollAccount {
            command_id: Uuid::new_v4(),
            enrollment_id: Uuid::new_v4(),
            workspace: p.workspace.clone(),
            connection_id: connection,
            alias: p.query.clone(),
            label: p.query.clone(),
        };
        let mut prefs = storage::load(host, &p.workspace)?;
        ensure!(
            prefs.enrollment.is_none(),
            "An original enrollment is retained; reopen and inspect it first"
        );
        let intent = storage::Intent {
            host,
            workspace: p.workspace.clone(),
            command: command.clone(),
            cancel: None,
        };
        prefs.enrollment = Some(intent.clone());
        storage::save(host, &p.workspace, &prefs)?;
        p.intent = Some(intent);
        p.outcome = Some(EnrollmentState::Starting);
        p.retry = Some((connection, p.query.clone()));
        p.restart = false;
        p.notice = "Requesting a sign-in code from ChatGPT…".into();
        self.status.clear();
        p.mode = Mode::Enrollment;
        p.query.clear();
        self.private_request(command, false)
    }
    fn cancel_enrollment(&mut self) -> Result<()> {
        let p = self.accounts.picker.as_mut().context("view closed")?;
        p.private = None;
        p.notice = "Cancelling the previous sign-in…".into();
        let i = p.intent.as_mut().context("No pending enrollment")?;
        let id = enrollment_id(i).context("Invalid original enrollment")?;
        let command_id = *i.cancel.get_or_insert_with(Uuid::new_v4);
        let mut prefs = storage::load(i.host, &p.workspace)?;
        prefs.enrollment = Some(i.clone());
        storage::save(i.host, &p.workspace, &prefs)?;
        let command = VesselCommand::CancelAccountEnrollment {
            command_id,
            enrollment_id: id,
            workspace: p.workspace.clone(),
        };
        self.private_request(command, false)
    }
    fn choose_account(&mut self, binding: AccountBinding) -> Result<()> {
        let p = self.accounts.picker.as_mut().context("view closed")?;
        ensure!(
            !p.disconnected,
            "Socket changed; reopen the account picker before selecting"
        );
        let mut settings = p.original.clone();
        settings.provider = provider(binding.transport).into();
        settings.account = Some(binding.clone());
        p.mode = Mode::Confirm(settings);
        p.edit = 0;
        p.models.clear();
        p.notice = "Tab / Shift+Tab Move · Ctrl+U Clear field · F2 Use defaults\nEnter Apply · Esc Cancel settings (running work continues)".into();
        let client = self.clients[p.route].clone();
        let workspace = p.workspace.clone();
        let (tx, rx) = oneshot::channel();
        self.accounts.reply = Some((p.id, rx));
        p.busy = true;
        tokio::spawn(async move {
            let result: Result<Reply> = async {
                let v = client
                    .request(VesselCommand::AccountModels {
                        workspace,
                        account: binding.clone(),
                    })
                    .await?;
                ensure!(
                    v["account"] == serde_json::to_value(&binding)?,
                    "Catalog account identity mismatch"
                );
                Ok(Reply::Models(
                    binding,
                    serde_json::from_value(v["models"].clone())?,
                ))
            }
            .await;
            let _ = tx.send(result.map_err(|_: anyhow::Error| anyhow::anyhow!("Catalog unavailable. Support remains unknown; explicit settings receive executing-host validation. Review and confirm, or Esc to cancel.")));
        });
        Ok(())
    }
    fn apply_account(&mut self, settings: Settings) -> Result<()> {
        let p = self.accounts.picker.as_ref().context("view closed")?;
        ensure!(
            !p.busy && !p.disconnected,
            "Wait for the account catalog response; reopen the picker after a socket change"
        );
        ensure!(
            !settings.model.is_empty()
                && settings.model.len() <= 256
                && safe(&settings.model) == settings.model
                && !settings.model.chars().any(char::is_whitespace),
            "Enter one model ID"
        );
        let binding = settings
            .account
            .clone()
            .context("Select a concrete account")?;
        for value in [&settings.reasoning_effort, &settings.service_tier]
            .into_iter()
            .flatten()
        {
            ensure!(
                value.len() <= 64
                    && safe(value) == *value
                    && !value.chars().any(char::is_whitespace),
                "Invalid inference override"
            );
        }
        let current = self.inference_settings(p.destination)?;
        ensure!(
            current.account == p.original.account
                && current.provider == p.original.provider
                && current.model == p.original.model
                && current.reasoning_effort == p.original.reasoning_effort
                && current.service_tier == p.original.service_tier,
            "Settings changed while selecting; reopen. Nothing sent"
        );
        let host = p.host.context("authenticated host missing")?;
        let destination = p.destination;
        match destination {
            Destination::Draft(id) => self.set_draft_account(id, host, settings)?,
            Destination::Live(t) => {
                ensure!(
                    self.clients.available(t.route),
                    "Original Vessel unavailable"
                );
                let v = self.views.get_mut(&t).context("voyage unavailable")?;
                ensure!(
                    v.pending.is_none(),
                    "Another command is pending; original account retained"
                );
                let snapshot = v.snapshot.as_ref().context("snapshot unavailable")?;
                let command_id = Uuid::new_v4();
                let command = VoyageCommand::SetAccountInference {
                    command_id,
                    expected_revision: snapshot.revision,
                    expires_at_ms: now().saturating_mul(1000).saturating_add(60_000),
                    account: binding,
                    model: settings.model,
                    reasoning_effort: settings.reasoning_effort,
                    service_tier: settings.service_tier,
                };
                v.pending = Some(Pending {
                    account_host: Some(host),
                    command_id,
                    incarnation: v.process.incarnation,
                    draft: v.draft.text.clone(),
                    preserve_draft: true,
                    original: Some(Box::new(command.clone())),
                    receipt_only: false,
                });
                if let Err(e) = drafts::save(&self.clients[t.route], v) {
                    v.pending = None;
                    return Err(e.context("Cannot retain exact account command; nothing sent"));
                }
                self.dispatch(t, command_id, command);
            }
        }
        self.accounts.picker = None;
        self.status = "Account/inference selection staged; current run unchanged. Server validation remains authoritative. Composer preserved.".into();
        Ok(())
    }
    pub(super) fn account_input(&mut self, event: &Event) -> Result<bool> {
        match self.account_input_inner(event) {
            Err(error) if self.accounts.picker.is_some() => {
                // Global status is covered by this modal. Keep action failures visible here.
                self.accounts.picker.as_mut().unwrap().notice = safe(&error.to_string());
                Ok(true)
            }
            result => result,
        }
    }
    fn account_input_inner(&mut self, event: &Event) -> Result<bool> {
        use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
        self.account_connection_tick();
        let Some(p) = self.accounts.picker.as_mut() else {
            return Ok(false);
        };
        // Esc is closure, never cancellation. Drop private view material immediately.
        if matches!(event, Event::Key(k) if k.code == KeyCode::Esc) {
            self.accounts.picker = None;
            self.accounts.reply = None;
            self.accounts.usage_reply = None;
            self.accounts.visible.set(false);
            return Ok(true);
        }
        if matches!(event, Event::Resize(..)) {
            self.accounts.visible.set(false);
            self.accounts.hits.borrow_mut().clear();
            return Ok(true);
        }
        let key = match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => Some(*k),
            _ => None,
        };
        if let Some(k) = key {
            if (k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('g'))
                || (p.disconnected && !self.clients.available(p.route) && k.code == KeyCode::Enter)
            {
                self.accounts.picker = None;
                self.accounts.reply = None;
                self.accounts.usage_reply = None;
                self.accounts.visible.set(false);
                self.open_vessels();
                return Ok(true);
            }
            if p.disconnected && k.code == KeyCode::Enter {
                let destination = p.destination;
                // Drop the private view/lock before re-reading the original saved intent.
                self.accounts.picker = None;
                self.accounts.reply = None;
                self.accounts.usage_reply = None;
                self.open_accounts(destination, "")?;
                return Ok(true);
            }
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('c' | 'q'))
            {
                p.private = None;
                self.quit = true;
                return Ok(true);
            }
        }
        if !self.accounts.visible.get() {
            return Ok(true);
        }
        let mut select = None;
        if let Event::Mouse(m) = event {
            if m.kind == MouseEventKind::Down(MouseButton::Left) {
                select = self
                    .accounts
                    .hits
                    .borrow()
                    .iter()
                    .find(|(r, _)| r.contains((m.column, m.row).into()))
                    .map(|(_, i)| *i);
            }
        }
        if p.busy {
            return Ok(true);
        }
        let key = if matches!(p.mode, Mode::Confirm(_)) {
            if let Some(index) = select.take() {
                p.edit = index;
                if index >= 3 {
                    Some(crossterm::event::KeyEvent::new(
                        KeyCode::Enter,
                        KeyModifiers::NONE,
                    ))
                } else {
                    key
                }
            } else {
                key
            }
        } else {
            key
        };
        if matches!(p.mode, Mode::Connections) {
            if let Some(index) = select {
                if let Some(c) = p
                    .catalogue
                    .connections
                    .iter()
                    .filter(|c| {
                        c.transports.contains(&Transport::ChatgptOauth)
                            && c.endpoint == "https://chatgpt.com/backend-api/codex"
                    })
                    .nth(index)
                {
                    p.mode = Mode::Alias(c.id);
                    p.query.clear();
                    p.notice = "Type a new safe alias; Enter explicitly starts device sign-in. No browser opens automatically.".into();
                }
                return Ok(true);
            }
        }
        if let Some(k) = key {
            match &mut p.mode {
                Mode::Connections => {
                    let connections: Vec<_> = p
                        .catalogue
                        .connections
                        .iter()
                        .filter(|c| {
                            c.transports.contains(&Transport::ChatgptOauth)
                                && c.endpoint == "https://chatgpt.com/backend-api/codex"
                        })
                        .collect();
                    match k.code {
                        KeyCode::Up => p.selected = p.selected.saturating_sub(1),
                        KeyCode::Down => {
                            p.selected = (p.selected + 1).min(connections.len().saturating_sub(1))
                        }
                        KeyCode::Enter => {
                            if let Some(c) = connections.get(p.selected) {
                                p.mode = Mode::Alias(c.id);
                                p.query.clear();
                                p.notice = "Review host and ChatGPT subscription provider. Type a new safe alias; Enter explicitly starts device sign-in. No browser opens automatically.".into();
                            }
                        }
                        _ => (),
                    }
                    return Ok(true);
                }
                Mode::Enrollment => {
                    match k.code {
                        KeyCode::Char('r') => {
                            if p.intent.is_some() {
                                self.poll_account_enrollment()?;
                            } else if p.outcome == Some(EnrollmentState::Succeeded) {
                                self.refresh_enrolled_accounts(None)?;
                            }
                        }
                        KeyCode::Char('n') => {
                            self.restart_enrollment()?;
                        }
                        KeyCode::Char('c') if p.intent.is_some() => {
                            p.restart = false;
                            self.cancel_enrollment()?;
                        }
                        KeyCode::Char('o') => {
                            if p.private.as_ref().is_some_and(active_material) {
                                open_browser(&self.accounts.browser_pending)?;
                            }
                        }
                        _ => (),
                    }
                    return Ok(true);
                }
                Mode::Confirm(s) => {
                    match k.code {
                        KeyCode::Enter if p.edit == 4 => {
                            p.mode = Mode::List;
                            p.notice =
                                "Choose an account for this voyage; default is set separately."
                                    .into();
                            return Ok(true);
                        }
                        KeyCode::Enter => {
                            let s = s.clone();
                            self.apply_account(s)?;
                            return Ok(true);
                        }
                        KeyCode::Tab => p.edit = (p.edit + 1) % 5,
                        KeyCode::BackTab => p.edit = (p.edit + 4) % 5,
                        KeyCode::F(2) => {
                            s.reasoning_effort = None;
                            s.service_tier = None;
                        }
                        _ if p.edit >= 3 => (),
                        _ => {
                            let mut text = match p.edit {
                                0 => s.model.clone(),
                                1 => s.reasoning_effort.clone().unwrap_or_default(),
                                _ => s.service_tier.clone().unwrap_or_default(),
                            };
                            match k.code {
                                KeyCode::Char('u')
                                    if k.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    text.clear()
                                }
                                KeyCode::Backspace => {
                                    text.pop();
                                }
                                KeyCode::Char(c)
                                    if !k
                                        .modifiers
                                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                                        && !c.is_control()
                                        && !c.is_whitespace()
                                        && text.len() + c.len_utf8() <= 256 =>
                                {
                                    text.push(c)
                                }
                                _ => (),
                            }
                            match p.edit {
                                0 => s.model = text,
                                1 => s.reasoning_effort = (!text.is_empty()).then_some(text),
                                _ => s.service_tier = (!text.is_empty()).then_some(text),
                            }
                        }
                    }
                    return Ok(true);
                }
                Mode::Alias(connection) if k.code == KeyCode::Enter => {
                    let c = *connection;
                    self.begin_enrollment(c)?;
                    return Ok(true);
                }
                Mode::List
                    if k.code == KeyCode::Char('r') && p.intent.is_some() && p.query.is_empty() =>
                {
                    p.mode = Mode::Enrollment;
                    self.poll_account_enrollment()?;
                    return Ok(true);
                }
                _ => (),
            }
            match k.code {
                KeyCode::F(5) if matches!(p.mode, Mode::List) => {
                    self.refresh_account_usage(true)?;
                    return Ok(true);
                }
                KeyCode::F(6) if matches!(p.mode, Mode::List) => {
                    self.set_default_account()?;
                    return Ok(true);
                }
                KeyCode::Up => p.selected = p.selected.saturating_sub(1),
                KeyCode::Down => {
                    p.selected = (p.selected + 1).min(p.choices().len().saturating_sub(1))
                }
                KeyCode::Enter => select = Some(p.selected),
                KeyCode::Backspace => {
                    p.query.pop();
                    p.selected = 0;
                }
                KeyCode::Char(c)
                    if !k
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !c.is_control()
                        && p.query.len() + c.len_utf8() <= 128 =>
                {
                    p.query.push(c);
                    p.selected = 0;
                }
                _ => (),
            }
        }
        if let Some(index) = select {
            if !matches!(p.mode, Mode::List) {
                return Ok(true);
            }
            let choices = p.choices();
            if let Some((_, Some(binding))) = choices.get(index) {
                let binding = binding.clone();
                self.choose_account(binding)?;
            } else if index == choices.len().saturating_sub(2) {
                ensure!(
                    p.enroll,
                    "Device sign-in is not authorized/supported here. Ask the host owner for account-enroll access, or use the execution-host private terminal"
                );
                ensure!(
                    p.intent.is_none(),
                    "Original sign-in retained. Clear search and press R to inspect; no new attempt started"
                );
                ensure!(
                    p.catalogue
                        .connections
                        .iter()
                        .any(|c| c.transports.contains(&Transport::ChatgptOauth)
                            && c.endpoint == "https://chatgpt.com/backend-api/codex"),
                    "No authorized native device connection. Use the private execution-host terminal alternative"
                );
                p.mode = Mode::Connections;
                p.selected = 0;
                p.query.clear();
                p.notice = "Choose the authorized executing-host ChatGPT connection. API keys are enrolled in a private execution-host terminal, not this view.".into();
            } else if index == choices.len().saturating_sub(1) {
                p.notice = "On the executing host, use a private human terminal: vessel auth accounts connections; vessel auth accounts add --connection UUID --account ALIAS. It privately prompts for an API key; --env NAME binds a HOST environment name. Never paste a key into Helm chat. For device CLI: vessel auth accounts login --connection UUID --account ALIAS. Helm does not resolve host credentials.".into();
            } else {
                p.notice = "This account is unavailable; sign in or ask its host owner. No fallback selected.".into();
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn binding() -> AccountBinding {
        AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 7,
            connection_revision: 3,
            transport: Transport::OpenaiResponses,
        }
    }
    #[test]
    fn pending_recovery_retains_the_exact_atomic_selection_and_host() {
        let command_id = Uuid::new_v4();
        let host = Uuid::new_v4();
        let account = binding();
        let original = VoyageCommand::SetAccountInference {
            command_id,
            expected_revision: 9,
            expires_at_ms: 12345,
            account,
            model: "synthetic-model".into(),
            reasoning_effort: Some("high".into()),
            service_tier: Some("default".into()),
        };
        let p = Pending {
            command_id,
            account_host: Some(host),
            incarnation: Uuid::new_v4(),
            draft: "unchanged composer".into(),
            preserve_draft: true,
            original: Some(Box::new(original.clone())),
            receipt_only: false,
        };
        let restored: Pending = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(restored.account_host, Some(host));
        assert_eq!(restored.draft, p.draft);
        let VoyageCommand::Resolve {
            command_id: recovered,
            original: Some(envelope),
        } = restored.resolution()
        else {
            panic!("not observation-only exact recovery");
        };
        assert_eq!(recovered, command_id);
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            serde_json::to_value(original).unwrap()
        );
    }
    #[test]
    fn settings_roundtrip_concrete_account_without_credentials() {
        let s = Settings {
            account: Some(binding()),
            model: "synthetic".into(),
            provider: "openai-responses".into(),
            ..Default::default()
        };
        let encoded = serde_json::to_value(&s).unwrap();
        let restored: Settings = serde_json::from_value(encoded.clone()).unwrap();
        assert!(s == restored);
        assert!(encoded.get("token").is_none());
        assert!(encoded.get("api_key").is_none());
        let legacy: Settings = serde_json::from_value(serde_json::json!({"model":"old"})).unwrap();
        assert!(legacy.account.is_none());
    }
    #[test]
    fn unavailable_accounts_never_become_selectable_fallbacks() {
        let mut a = AccountDescriptor {
            id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            alias: "work".into(),
            label: "Work".into(),
            metadata_revision: 1,
            identity_generation: 1,
            credential_revision: 1,
            capability_revision: 1,
            availability: CredentialAvailability::Available,
            state: AccountState::Ready,
        };
        assert!(selectable(&a));
        for state in [AccountState::Removed, AccountState::SignInRequired] {
            a.state = state;
            assert!(!selectable(&a));
        }
        a.state = AccountState::Ready;
        for availability in [
            CredentialAvailability::Expired,
            CredentialAvailability::EnvironmentUnavailable,
            CredentialAvailability::EnvironmentChanged,
            CredentialAvailability::RefreshPendingOrUncertain,
            CredentialAvailability::Missing,
        ] {
            a.availability = availability;
            assert!(!selectable(&a));
        }
    }
    #[test]
    fn aliases_are_safe_not_shell_or_terminal_input() {
        for s in ["", "a b", "a\n", "../key", "x;open", "\u{1b}[31m", "é"] {
            assert!(!valid_alias(s));
        }
        assert!(valid_alias("work-API_2"));
        assert!(!valid_alias(&"a".repeat(65)));
    }
    #[test]
    fn private_material_requires_exact_provider_and_current_expiry() {
        let mut p = PrivateEnrollmentStatus {
            failure: None,
            status: EnrollmentStatus {
                enrollment_id: Uuid::new_v4(),
                state: EnrollmentState::Pending,
                account_id: None,
                expires_at: now() + 30,
                effects_may_have_occurred: false,
            },
            user_code: Some("ABCD-1234".into()),
            verification_uri: Some("https://auth.openai.com/codex/device".into()),
        };
        assert!(active_material(&p));
        p.status.expires_at = now();
        assert!(!active_material(&p));
        p.status.expires_at += 100;
        p.verification_uri = Some("https://auth.openai.com.evil/codex/device".into());
        assert!(!active_material(&p));
    }
    #[test]
    fn enrollment_intent_contains_no_private_display_material() {
        let host = Uuid::new_v4();
        let id = Uuid::new_v4();
        let intent = storage::Intent {
            host,
            workspace: "/workspace".into(),
            command: VesselCommand::EnrollAccount {
                command_id: Uuid::new_v4(),
                enrollment_id: id,
                workspace: "/workspace".into(),
                connection_id: Uuid::new_v4(),
                alias: "work".into(),
                label: "Work".into(),
            },
            cancel: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&intent).unwrap();
        assert!(!json.contains("user_code"));
        assert!(!json.contains("verification_uri"));
        let restored: storage::Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(enrollment_id(&restored), Some(id));
        assert_eq!(restored.cancel, intent.cancel);
    }
}

#[cfg(test)]
#[path = "accounts/app_tests.rs"]
pub(super) mod app_tests;
