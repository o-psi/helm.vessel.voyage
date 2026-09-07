//! Helm-owned proposals. No runtime exists until the user sends the first turn.
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
    id: Uuid,
    route: String,
    workspace: PathBuf,
    config: Option<crate::Config>,
    explicit: voyage_runtime::policy_profile::Overrides,
    selection: Option<voyage_runtime::policy_profile::selection::SelectionRequest>,
    confirmation: Option<String>,
    text: String,
    start: Option<VesselCommand>,
    start_attempted: bool,
    process: Option<ProcessInfo>,
    turn: Uuid,
    submit: Option<VoyageCommand>,
    attempted: bool,
    finished: bool,
    receipt: Option<serde_json::Value>,
}

pub(super) struct Draft {
    saved: Saved,
    composer: composer::Composer,
    route: usize,
    busy: bool,
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

    pub(super) fn send_new_draft(&mut self, check_only: bool) -> Result<()> {
        let id = self.active_draft.context("select a draft")?;
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        anyhow::ensure!(!draft.busy, "First send is already being checked");
        anyhow::ensure!(
            !draft.composer.text.trim().is_empty(),
            "Write your first message before starting a voyage"
        );
        anyhow::ensure!(draft.composer.text.len() <= 65536, "draft limit is 64 KiB");
        let client = self.clients[draft.route].clone();
        if draft.saved.start.is_none() {
            if check_only {
                return Ok(());
            }
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
        self.status = if check_only {
            "Checking first-send outcome…"
        } else {
            "Starting your voyage…"
        }
        .into();
        tokio::spawn(async move {
            let _lock = lock;
            let result = launch::advance(&client, &mut saved, check_only)
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
        draft.saved = saved;
        match result {
            Ok(Some(receipt)) => {
                let process = draft.saved.process.clone().expect("accepted first send has process");
                let target = Target { route: draft.route, session: id };
                let mut view = self.views.remove(&target).unwrap_or_else(|| View::new(process.clone()));
                view.process = process;
                view.draft.text = draft.saved.text.clone();
                view.draft.cursor = view.draft.text.len();
                view.pending = Some(state::Pending { command_id: draft.saved.turn, incarnation: view.process.incarnation, draft: draft.saved.text.clone(), preserve_draft: false, original: draft.saved.submit.clone().map(Box::new), receipt_only: false });
                if let Err(error) = drafts::save(&self.clients[target.route], &view) {
                    self.status = format!("First-send receipt found; draft handoff could not be saved: {error}. F4 retries recovery.");
                    return;
                }
                self.views.insert(target, view);
                let command_id = draft.saved.turn;
                draft.saved.finished = true;
                if let Err(error) = storage::save(&draft.saved) {
                    draft.saved.finished = false;
                    self.status = format!("First-send receipt found; recovery record could not be saved: {error}");
                    return;
                }
                self.new_drafts.remove(&id);
                if self.active_draft == Some(id) { self.active_draft = None; self.selected = Some(target); }
                self.update(Update::Command { target, command_id, refused: false, result: Ok(receipt) });
            }
            Ok(None) => self.status = "First send is not confirmed. F4 checks; Enter continues setup or checks delivery. Your text is preserved.".into(),
            Err(error) => self.status = format!("{} · Text and identity saved. F4 checks; Enter continues setup or checks delivery.", safe(&error)),
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
    let result = launch::advance(client, &mut draft.saved, false).await;
    let receipt = result
        .with_context(|| {
            format!(
                "First send saved as local draft {}. Open interactive Helm to check with F4",
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
