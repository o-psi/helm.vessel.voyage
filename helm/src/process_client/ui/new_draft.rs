//! Helm-owned proposals. No runtime exists until the user sends the first turn.
mod images;
mod input;
mod launch;
mod render;
mod storage;
mod workspaces;
use super::state::Route;
pub(super) use workspaces::WorkspacePicker;

use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VoyageCommand};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Saved {
    pub(super) id: Uuid,
    route: String,
    pub(super) workspace: PathBuf,
    config: Option<crate::Config>,
    #[serde(default)]
    pub(super) account_host: Option<Uuid>,
    #[serde(default)]
    pub(super) account_settings: Option<super::inference::Settings>,
    explicit: voyage_runtime::policy_profile::Overrides,
    selection: Option<voyage_runtime::policy_profile::selection::SelectionRequest>,
    confirmation: Option<String>,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    markers: Option<Vec<composer::ImageMarker>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<super::attachments::Image>,
    pub(super) start: Option<VesselCommand>,
    start_attempted: bool,
    process: Option<ProcessInfo>,
    turn: Uuid,
    submit: Option<VoyageCommand>,
    attempted: bool,
    finished: bool,
    receipt: Option<serde_json::Value>,
}

pub(super) struct Draft {
    pub(super) saved: Saved,
    composer: composer::Composer,
    pub(super) route: Route,
    pub(super) busy: bool,
    _lock: std::fs::File,
}

impl Saved {
    pub(super) fn start_resolution(&self) -> Result<(Uuid, VesselCommand)> {
        let original = self
            .start
            .as_ref()
            .context("original creation envelope missing; recovery retained")?;
        Ok(match original {
            VesselCommand::StartAccount {
                config_path,
                command_id,
                session_id,
                workspace,
                account,
                model,
                reasoning_effort,
                service_tier,
            } => (
                *command_id,
                VesselCommand::ResolveStartAccount {
                    config_path: config_path.clone(),
                    command_id: *command_id,
                    session_id: *session_id,
                    workspace: workspace.clone(),
                    account: account.clone(),
                    model: model.clone(),
                    reasoning_effort: reasoning_effort.clone(),
                    service_tier: service_tier.clone(),
                },
            ),
            VesselCommand::Start {
                command_id,
                workspace,
                ..
            } => (
                *command_id,
                VesselCommand::ResolveStart {
                    command_id: *command_id,
                    session_id: self.id,
                    workspace: workspace.clone(),
                    config_path: None,
                },
            ),
            VesselCommand::StartConfigured {
                command_id,
                workspace,
                config_path,
                ..
            } => (
                *command_id,
                VesselCommand::ResolveStart {
                    command_id: *command_id,
                    session_id: self.id,
                    workspace: workspace.clone(),
                    config_path: Some(config_path.clone()),
                },
            ),
            _ => anyhow::bail!("Unsupported original creation envelope; recovery retained"),
        })
    }
    fn validate_identity(&self) -> Result<()> {
        anyhow::ensure!(
            !self.id.is_nil() && !self.turn.is_nil(),
            "Invalid saved draft identity; original retained"
        );
        if let Some(start) = &self.start {
            let (session, command, workspace) = match start {
                VesselCommand::Start {
                    session_id,
                    command_id,
                    workspace,
                }
                | VesselCommand::StartAccount {
                    session_id,
                    command_id,
                    workspace,
                    ..
                }
                | VesselCommand::StartConfigured {
                    session_id,
                    command_id,
                    workspace,
                    ..
                } => (*session_id, *command_id, workspace),
                _ => anyhow::bail!("Unsupported saved start envelope; original retained"),
            };
            anyhow::ensure!(
                session == self.id && !command.is_nil() && workspace == &self.workspace,
                "Saved start identity mismatch; original retained"
            );
        }
        if let Some(VesselCommand::StartAccount {
            account,
            model,
            reasoning_effort,
            service_tier,
            ..
        }) = &self.start
        {
            let settings = self
                .account_settings
                .as_ref()
                .context("Saved account creation settings missing")?;
            anyhow::ensure!(
                self.account_host.is_some()
                    && settings.account.as_ref() == Some(account)
                    && &settings.model == model
                    && &settings.reasoning_effort == reasoning_effort
                    && &settings.service_tier == service_tier,
                "Saved account creation envelope changed; original retained"
            );
        }
        anyhow::ensure!(
            !self.start_attempted || self.start.is_some(),
            "Saved attempted start envelope missing; original retained"
        );
        if let Some(process) = &self.process {
            anyhow::ensure!(
                process.session_id == self.id,
                "Saved process identity mismatch; original retained"
            );
        }
        if let Some(submit) = &self.submit {
            anyhow::ensure!(
                submit.mutation_id() == Some(self.turn)
                    && matches!(
                        submit,
                        VoyageCommand::Submit { .. } | VoyageCommand::SubmitContent { .. }
                    ),
                "Saved first-turn identity mismatch; original retained"
            );
        }
        anyhow::ensure!(
            !self.attempted || (self.submit.is_some() && self.process.is_some()),
            "Saved attempted turn envelope missing; original retained"
        );
        if let Some(receipt) = &self.receipt {
            anyhow::ensure!(
                receipt["command_id"] == self.turn.to_string() && self.process.is_some(),
                "Saved receipt identity mismatch; original retained"
            );
        }
        Ok(())
    }

    fn retain_policy(&mut self) {
        if let Some(config) = &self.config {
            self.explicit = config.policy_explicit.clone();
            self.selection = config.policy_profile.as_ref().map(|p| p.request().clone());
            self.confirmation = config
                .policy_profile
                .as_ref()
                .map(|p| p.confirmation().to_owned());
        }
    }

    fn launch_config(&self) -> Result<Option<crate::Config>> {
        let Some(mut config) = self.config.clone() else {
            return Ok(None);
        };
        config.policy_explicit = self.explicit.clone();
        config.policy_profile = self
            .selection
            .clone()
            .map(|request| {
                voyage_runtime::policy_profile::selection::Selection::bind(
                    &config,
                    &self.workspace,
                    request,
                    self.confirmation.as_deref(),
                )
            })
            .transpose()?;
        Ok(Some(config))
    }
}

impl App {
    /// Caller must abort/observe old route tasks before reactivation. Reload the
    /// durable transition because a stale completion event may have been dropped.
    pub(super) fn reactivate_drafts(&mut self, route: Route) -> Result<()> {
        anyhow::ensure!(self.clients.current(route), "Draft activation is stale");
        for draft in self
            .new_drafts
            .values_mut()
            .filter(|draft| draft.route.id == route.id)
        {
            if let Some(saved) = storage::reload(draft.saved.id)? {
                anyhow::ensure!(
                    saved.route == storage::route(&self.clients[route])?,
                    "Saved draft connection identity changed; original retained"
                );
                draft.saved = saved;
                draft.composer = super::attachments::restore_draft(
                    draft.saved.text.clone(),
                    draft.saved.markers.clone(),
                    &draft.saved.images,
                )?;
            }
            draft.route = route;
            draft.busy = false;
        }
        self.recover_new_drafts()?;
        Ok(())
    }

    /// Call only after every aborted first-send task has been joined. Aborting
    /// a handle alone does not establish that the last durable write completed.
    pub(super) fn cancel_new_drafts(&mut self, connection: Uuid) -> Result<()> {
        for draft in self
            .new_drafts
            .values_mut()
            .filter(|draft| draft.route.id == connection)
        {
            if let Some(saved) = storage::reload(draft.saved.id)? {
                anyhow::ensure!(
                    saved.route == draft.saved.route,
                    "Saved draft route identity changed"
                );
                draft.saved = saved;
                draft.composer = super::attachments::restore_draft(
                    draft.saved.text.clone(),
                    draft.saved.markers.clone(),
                    &draft.saved.images,
                )?;
            }
            draft.busy = false;
        }
        Ok(())
    }

    pub(super) fn recover_new_drafts(&mut self) -> Result<()> {
        // Locked in-memory drafts are skipped; preserve all unsent text.
        self.new_drafts
            .extend(storage::recover(self.clients.iter())?);
        Ok(())
    }

    pub fn create(&mut self, workspace: Option<&str>) -> Result<()> {
        let route = self
            .active_draft
            .and_then(|id| self.new_drafts.get(&id).map(|d| d.route))
            .or_else(|| self.selected.map(|t| t.route))
            .or_else(|| self.clients.first_route())
            .context("No Vessel connection is available")?;
        self.create_on_route(route, workspace)
    }

    pub(super) fn create_on_route(&mut self, route: Route, workspace: Option<&str>) -> Result<()> {
        anyhow::ensure!(
            self.clients.current(route),
            "Reconnect this Vessel before creating a draft"
        );
        let client = &self.clients[route];
        let workspace = if client.is_local() {
            match workspace {
                Some(path) => PathBuf::from(path),
                None => self
                    .active_draft
                    .and_then(|id| self.new_drafts.get(&id))
                    .filter(|draft| draft.route == route)
                    .map(|draft| draft.saved.workspace.clone())
                    .unwrap_or(std::env::current_dir()?),
            }
        } else {
            let choices = workspaces::authorized(client)?;
            match workspace {
                Some(path) => workspaces::select(&choices, path)?.path.clone(),
                None => {
                    let preferred = self
                        .active_draft
                        .and_then(|id| self.new_drafts.get(&id))
                        .filter(|draft| draft.route == route)
                        .and_then(|draft| choices.iter().find(|w| w.path == draft.saved.workspace))
                        .map(|w| w.id)
                        .or_else(|| {
                            self.vessels.as_ref().and_then(|manager| {
                                manager
                                    .borrow()
                                    .records()
                                    .iter()
                                    .find(|c| c.id == route.id)
                                    .and_then(|c| c.workspace_preference)
                            })
                        })
                        .or_else(|| client.managed().and_then(|c| c.workspace_preference));
                    self.workspace_picker = Some(WorkspacePicker::new(route, choices, preferred));
                    return Ok(());
                }
            }
        };
        anyhow::ensure!(
            workspace.is_absolute(),
            "workspace must be absolute on the executing host"
        );
        if let Some((&id, _)) = self.new_drafts.iter().find(|(candidate, d)| {
            let reading = self
                .clipboard_pending
                .as_ref()
                .is_some_and(|p| p.destination == super::paste::Destination::Draft(**candidate));
            !reading
                && d.route == route
                && d.saved.workspace == workspace
                && d.saved.start.is_none()
                && d.composer.text.is_empty()
                && d.saved.images.is_empty()
        }) {
            self.select_draft(id);
            return Ok(());
        }
        let mut saved = Saved {
            id: Uuid::new_v4(),
            account_host: None,
            account_settings: None,
            route: storage::route(client)?,
            workspace,
            config: if client.is_local() {
                self.new_chat_config.clone()
            } else {
                None
            },
            text: String::new(),
            images: Vec::new(),
            markers: None,
            start: None,
            start_attempted: false,
            process: None,
            turn: Uuid::new_v4(),
            submit: None,
            attempted: false,
            finished: false,
            receipt: None,
            explicit: Default::default(),
            selection: None,
            confirmation: None,
        };
        saved.retain_policy();
        let draft = storage::create(saved, route)?;
        let id = draft.saved.id;
        self.new_drafts.insert(id, draft);
        self.select_draft(id);
        self.status = "Draft in Helm. Send your first message to start the voyage.".into();
        Ok(())
    }

    pub(super) fn select_draft(&mut self, id: Uuid) {
        self.active_draft = Some(id);
        self.selected = None;
        self.archives = false;
        self.sidebar.menu = None;
        self.sidebar.focus = sidebar::Focus::Composer;
        self.completion = Default::default();
    }

    pub(super) fn send_new_draft(&mut self) -> Result<()> {
        let id = self.active_draft.context("select a draft")?;
        self.send_draft(id)
    }

    pub(super) fn send_draft(&mut self, id: Uuid) -> Result<()> {
        self.drive_draft(id, false)
    }

    pub(super) fn recover_draft(&mut self, id: Uuid) -> Result<()> {
        self.drive_draft(id, true)
    }

    fn drive_draft(&mut self, id: Uuid, observe_only: bool) -> Result<()> {
        self.ensure_paste_finished(super::paste::Destination::Draft(id))?;
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        anyhow::ensure!(!draft.busy, "First send is already being checked");
        anyhow::ensure!(
            !draft.composer.text.trim().is_empty() || !draft.saved.images.is_empty(),
            "Write your first message before starting a voyage"
        );
        anyhow::ensure!(draft.composer.text.len() <= 65536, "draft limit is 64 KiB");
        super::attachments::validate_set(&draft.saved.images)?;
        anyhow::ensure!(
            self.clients.available(draft.route),
            "This draft belongs to an inactive connection. Reconnect its original Vessel access to recover; no command was redirected"
        );
        let client = self.clients[draft.route].clone();
        if draft.saved.start.is_none() {
            if observe_only {
                return Ok(());
            }
            let command_id = Uuid::new_v4();
            let workspace = draft.saved.workspace.clone();
            draft.saved.start = Some(if let Some(settings) = &draft.saved.account_settings {
                let config_path = draft.saved.launch_config()?.map(|config| {
                    anyhow::ensure!(client.is_local(), "Configured account creation is owner-local only; remote configuration remains host-owned");
                    super::super::frontend::launch::persist(&config, &draft.saved.workspace, &client.directory)
                }).transpose()?;
                VesselCommand::StartAccount {
                    config_path,
                    command_id,
                    session_id: id,
                    workspace,
                    account: settings
                        .account
                        .clone()
                        .context("Choose an available account with /account before sending")?,
                    model: settings.model.clone(),
                    reasoning_effort: settings.reasoning_effort.clone(),
                    service_tier: settings.service_tier.clone(),
                }
            } else {
                anyhow::bail!(
                    "Resolve and review the executing-host account with /account before sending"
                );
            });
        }
        draft.saved.text = draft.composer.text.clone();
        draft.saved.markers =
            (!draft.saved.images.is_empty()).then(|| draft.composer.markers.clone());
        storage::save(&draft.saved)?;
        let lock = draft._lock.try_clone()?;
        draft.busy = true;
        let mut saved = draft.saved.clone();
        let sender = self.sender.clone();
        let route = draft.route;
        self.status = "Starting your voyage…".into();
        let job = tokio::spawn(async move {
            let _lock = lock;
            let result = if observe_only {
                launch::observe(&client, &mut saved).await
            } else {
                launch::advance(&client, &mut saved).await
            }
            .map_err(|e| e.to_string());
            let _ = sender
                .send(Update::FirstSend {
                    route,
                    saved: Box::new(saved),
                    result,
                })
                .await;
        });
        self.route_tasks.entry(route.id).or_default().push(job);
        Ok(())
    }

    pub(super) fn first_send_update(
        &mut self,
        saved: Saved,
        result: Result<Option<serde_json::Value>, String>,
    ) {
        let id = saved.id;
        let Some(draft) = self.new_drafts.get_mut(&id) else {
            return;
        };
        draft.busy = false;
        self.first_send_checks
            .insert(id, Instant::now() + Duration::from_secs(5));
        draft.saved = saved;
        match result {
            Ok(Some(receipt)) => {
                let process = draft.saved.process.clone().expect("accepted first send has process");
                let target = Target { route: draft.route, session: id };
                // Stage the durable handoff without exposing a second recovery
                // owner. A failed finished-record save must not make this view
                // editable and then overwrite it on the next recovery attempt.
                let mut handoff = View::new(process.clone());
                if let Some(view) = self.views.get(&target) {
                    handoff.draft = view.draft.clone();
                    handoff.images = view.images.clone();
                    handoff.pending = view.pending.clone();
                } else if let Err(error) = drafts::load(&self.clients[target.route], &mut handoff) {
                    self.status = format!("First-send handoff cannot load the saved view: {error}. Existing recovery data retained.");
                    return;
                }
                if !handoff.images.is_empty() && handoff.images != draft.saved.images {
                    self.status = "First-send handoff found another saved image draft; recovery data retained.".into();
                    return;
                }
                handoff.images = draft.saved.images.clone();
                if handoff.draft.text.is_empty() {
                    match super::attachments::restore_draft(draft.saved.text.clone(), draft.saved.markers.clone(), &handoff.images) {
                        Ok(composer) => handoff.draft = composer,
                        Err(_) => { self.status = "First-send image draft invalid; recovery data retained".into(); return; }
                    }
                }
                if handoff.pending.is_none() {
                    handoff.pending = Some(state::Pending { account_host: None, command_id: draft.saved.turn, incarnation: process.incarnation, draft: draft.saved.text.clone(), preserve_draft: false, original: draft.saved.submit.clone().map(Box::new), receipt_only: false });
                }
                if let Err(error) = drafts::save(&self.clients[target.route], &handoff) {
                    self.status = format!("First-send receipt found; draft handoff could not be saved: {error}. Helm will retry recovery automatically.");
                    return;
                }
                let command_id = draft.saved.turn;
                draft.saved.finished = true;
                if let Err(error) = storage::save(&draft.saved) {
                    draft.saved.finished = false;
                    self.status = format!("First-send receipt found; recovery record could not be saved: {error}");
                    return;
                }
                let view = self.views.entry(target).or_insert_with(|| View::new(process.clone()));
                view.process = process;
                view.draft = handoff.draft;
                view.images = handoff.images;
                view.pending = handoff.pending;
                self.new_drafts.remove(&id);
                if self.active_draft == Some(id) { self.active_draft = None; self.selected = Some(target); }
                self.update(Update::Command { target, command_id, refused: false, result: Ok(receipt) });
            }
            Ok(None) => self.status = "First send is not confirmed. Helm checks receipts without replay. If creation is confirmed but no turn was attempted, press Enter to continue; your exact text is preserved.".into(),
            Err(error) => self.status = if draft.saved.start.is_none() {
                format!("{} · Voyage not started. Your draft is editable.", safe(&error))
            } else {
                format!("{} · Text and exact command identity saved. Helm observes recovery without replay.", safe(&error))
            },
        }
    }
}

pub(in crate::process_client) async fn start_plain(
    client: &Client,
    config: crate::Config,
    prompt: String,
) -> Result<ProcessInfo> {
    anyhow::ensure!(
        client.is_local(),
        "Plain configured launch is local only; remote settings remain host-owned"
    );
    let workspace = config.resolve_workspace(None)?;
    let mut saved = Saved {
        id: Uuid::new_v4(),
        account_host: None,
        account_settings: None,
        route: storage::route(client)?,
        workspace,
        config: Some(config),
        text: prompt,
        images: Vec::new(),
        markers: None,
        start: None,
        start_attempted: false,
        process: None,
        turn: Uuid::new_v4(),
        submit: None,
        attempted: false,
        finished: false,
        receipt: None,
        explicit: Default::default(),
        selection: None,
        confirmation: None,
    };
    saved.retain_policy();
    let mut draft = storage::create(
        saved,
        Route {
            id: client.id(),
            generation: client.generation(),
        },
    )?;
    let config_path = super::super::frontend::launch::persist(
        draft.saved.config.as_ref().expect("local config"),
        &draft.saved.workspace,
        &client.directory,
    )?;
    draft.saved.start = Some(VesselCommand::StartConfigured {
        command_id: Uuid::new_v4(),
        session_id: draft.saved.id,
        workspace: draft.saved.workspace.clone(),
        config_path,
    });
    storage::save(&draft.saved)?;
    let result = launch::advance(client, &mut draft.saved).await;
    let receipt = result
        .with_context(|| {
            format!(
                "First send saved as local draft {}. Open interactive Helm for automatic recovery",
                draft.saved.id
            )
        })?
        .context("First-send outcome unknown; open interactive Helm to recover the saved draft")?;
    anyhow::ensure!(
        receipt["status"] != "rejected",
        "First send refused; open interactive Helm to recover your text"
    );
    draft.saved.finished = true;
    storage::save(&draft.saved)?;
    draft
        .saved
        .process
        .context("first send omitted process identity")
}

impl App {
    /// Read only safe configuration metadata. Never construct a provider or resolve a key here.
    pub(super) fn draft_account_seed(&self, id: Uuid) -> Option<super::inference::Settings> {
        let config = self.new_drafts.get(&id)?.saved.config.as_ref()?;
        Some(super::inference::Settings {
            account: config.account.clone(),
            model: config.model.clone(),
            provider: serde_json::to_value(&config.provider)
                .ok()?
                .as_str()?
                .to_owned(),
            reasoning_effort: config.reasoning_effort.clone(),
            service_tier: config.service_tier.clone(),
            ..Default::default()
        })
    }
    pub(super) fn set_draft_account(
        &mut self,
        id: Uuid,
        host: Uuid,
        settings: super::inference::Settings,
    ) -> Result<()> {
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        anyhow::ensure!(
            draft.saved.start.is_none() && !draft.busy,
            "Creation pending; original account retained"
        );
        anyhow::ensure!(
            draft.saved.account_host.is_none_or(|h| h == host),
            "Authenticated host changed; create a new draft after review"
        );
        let mut saved = draft.saved.clone();
        saved.account_host = Some(host);
        saved.account_settings = Some(settings);
        storage::save(&saved)?;
        draft.saved = saved;
        Ok(())
    }
    pub(super) fn ensure_draft_inference_editable(&self, id: Uuid) -> Result<()> {
        let draft = self.new_drafts.get(&id).context("draft unavailable")?;
        anyhow::ensure!(
            draft.saved.start.is_none() && !draft.busy,
            "Wait for automatic first-send confirmation before changing this draft"
        );
        anyhow::ensure!(
            draft.saved.account_settings.is_some(),
            "Resolve executing-host settings with /account first"
        );
        Ok(())
    }
    pub(super) fn draft_inference_settings(&self, id: Uuid) -> Result<super::inference::Settings> {
        self.new_drafts
            .get(&id)
            .context("draft unavailable")?
            .saved
            .account_settings
            .clone()
            .context("Resolve executing-host inference with /account first")
    }
    pub(super) fn draft_inference_catalog(&self, id: Uuid) -> Result<(crate::Config, PathBuf)> {
        let draft = self.new_drafts.get(&id).context("draft unavailable")?;
        Ok((
            draft
                .saved
                .config
                .clone()
                .context("Remote draft catalog unavailable; use an explicit model ID")?,
            draft.saved.workspace.clone(),
        ))
    }
    pub(super) fn save_draft_inference(
        &mut self,
        id: Uuid,
        settings: &super::inference::Settings,
        command: Option<&str>,
    ) -> Result<()> {
        self.ensure_draft_inference_editable(id)?;
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        // Validate a clone, persist the complete replacement, then update memory.
        let mut saved = draft.saved.clone();
        saved.account_settings = Some(settings.clone());
        let clear_command = command.is_some_and(|text| draft.composer.text.trim() == text);
        if clear_command {
            saved.text.clear();
        }
        storage::save(&saved)?;
        draft.saved = saved;
        if clear_command {
            draft.composer.take();
        }
        Ok(())
    }
}
