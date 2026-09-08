//! Helm-owned proposals. No runtime exists until the user sends the first turn.
mod images;
mod input;
mod launch;
mod render;
mod storage;

use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VoyageCommand};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Saved {
    pub(super) id: Uuid,
    route: String,
    workspace: PathBuf,
    config: Option<crate::Config>,
    explicit: voyage_runtime::policy_profile::Overrides,
    selection: Option<voyage_runtime::policy_profile::selection::SelectionRequest>,
    confirmation: Option<String>,
    text: String,
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
    route: usize,
    pub(super) busy: bool,
    _lock: std::fs::File,
}

impl Saved {
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
    pub(super) fn recover_new_drafts(&mut self) -> Result<()> {
        self.new_drafts = storage::recover(&self.clients)?;
        Ok(())
    }

    pub fn create(&mut self, workspace: Option<&str>) -> Result<()> {
        let route = self
            .active_draft
            .and_then(|id| self.new_drafts.get(&id).map(|d| d.route))
            .or_else(|| self.selected.map(|t| t.route))
            .unwrap_or(0);
        let client = &self.clients[route];
        anyhow::ensure!(
            client.access_file.is_none(),
            "This connection grants access to an existing voyage; choose a Vessel connection to create one"
        );
        let workspace = match workspace {
            Some(path) => PathBuf::from(path),
            None if self.active_draft.is_some() => self.new_drafts
                [&self.active_draft.expect("active draft")]
                .saved
                .workspace
                .clone(),
            None if client.is_local() => std::env::current_dir()?,
            None => anyhow::bail!("remote creation needs /new /absolute/workspace"),
        };
        anyhow::ensure!(
            workspace.is_absolute(),
            "workspace must be absolute on the executing host"
        );
        if let Some((&id, _)) = self.new_drafts.iter().find(|(_, d)| {
            d.route == route
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
            route: storage::route(client)?,
            workspace,
            config: if client.is_local() {
                self.new_chat_config.clone()
            } else {
                None
            },
            text: String::new(),
            images: Vec::new(),
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
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        anyhow::ensure!(!draft.busy, "First send is already being checked");
        anyhow::ensure!(
            !draft.composer.text.trim().is_empty() || !draft.saved.images.is_empty(),
            "Write your first message before starting a voyage"
        );
        anyhow::ensure!(draft.composer.text.len() <= 65536, "draft limit is 64 KiB");
        super::attachments::validate_set(&draft.saved.images)?;
        let client = self.clients[draft.route].clone();
        if draft.saved.start.is_none() {
            let command_id = Uuid::new_v4();
            let workspace = draft.saved.workspace.clone();
            draft.saved.start = Some(if let Some(config) = &draft.saved.launch_config()? {
                VesselCommand::StartConfigured {
                    command_id,
                    session_id: id,
                    workspace: workspace.clone(),
                    config_path: super::super::frontend::launch::persist(
                        config,
                        &workspace,
                        &client.directory,
                    )?,
                }
            } else {
                VesselCommand::Start {
                    command_id,
                    session_id: id,
                    workspace,
                }
            });
        }
        draft.saved.text = draft.composer.text.clone();
        storage::save(&draft.saved)?;
        let lock = draft._lock.try_clone()?;
        draft.busy = true;
        let mut saved = draft.saved.clone();
        let sender = self.sender.clone();
        self.status = "Starting your voyage…".into();
        tokio::spawn(async move {
            let _lock = lock;
            let result = launch::advance(&client, &mut saved)
                .await
                .map_err(|e| e.to_string());
            let _ = sender
                .send(Update::FirstSend {
                    saved: Box::new(saved),
                    result,
                })
                .await;
        });
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
                    handoff.draft.text = draft.saved.text.clone();
                    handoff.draft.cursor = handoff.draft.text.len();
                }
                if handoff.pending.is_none() {
                    handoff.pending = Some(state::Pending { command_id: draft.saved.turn, incarnation: process.incarnation, draft: draft.saved.text.clone(), preserve_draft: false, original: draft.saved.submit.clone().map(Box::new), receipt_only: false });
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
            Ok(None) => self.status = "First send is not confirmed. Helm continues setup and checks delivery automatically. Your text is preserved.".into(),
            Err(error) => self.status = if draft.saved.start.is_none() {
                format!("{} · Voyage not started. Your draft is editable.", safe(&error))
            } else {
                format!("{} · Text and identity saved. Helm continues setup and checks delivery automatically.", safe(&error))
            },
        }
    }
}

pub(in crate::process_client) async fn start_plain(
    client: &Client,
    config: crate::Config,
    prompt: String,
) -> Result<ProcessInfo> {
    let workspace = config.resolve_workspace(None)?;
    let mut saved = Saved {
        id: Uuid::new_v4(),
        route: storage::route(client)?,
        workspace,
        config: Some(config),
        text: prompt,
        images: Vec::new(),
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
    let mut draft = storage::create(saved, 0)?;
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
    pub(super) fn ensure_draft_inference_editable(&self, id: Uuid) -> Result<()> {
        let draft = self.new_drafts.get(&id).context("draft unavailable")?;
        anyhow::ensure!(
            draft.saved.start.is_none() && !draft.busy,
            "Wait for automatic first-send confirmation before changing this draft"
        );
        anyhow::ensure!(
            draft.saved.config.is_some(),
            "Remote draft inference uses executing-host settings; configure that host before sending"
        );
        Ok(())
    }
    pub(super) fn draft_inference_settings(&self, id: Uuid) -> Result<super::inference::Settings> {
        let config = self
            .new_drafts
            .get(&id)
            .context("draft unavailable")?
            .saved
            .config
            .as_ref()
            .context("Remote drafts use executing-host inference settings")?;
        let provider = serde_json::to_value(&config.provider)?
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        let known = self.draft_known_model(id, &provider, &config.model);
        let resolution = crate::provider::resolve_inference(config, known);
        let reasoning_efforts = resolution.thinking.values.clone();
        let service_tiers = resolution.service.values.clone();
        Ok(super::inference::Settings {
            model: config.model.clone(),
            reasoning_effort: config.reasoning_effort.clone(),
            service_tier: config.service_tier.clone(),
            provider,
            resolution: Some(resolution),
            reasoning_efforts,
            service_tiers,
        })
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
        let config = saved
            .config
            .as_mut()
            .context("draft configuration unavailable")?;
        config.model = settings.model.clone();
        config.reasoning_effort = settings.reasoning_effort.clone();
        config.service_tier = settings.service_tier.clone();
        crate::provider::validate_inference_settings(config)?;
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
