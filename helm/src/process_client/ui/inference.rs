//! One inference action path for composer controls and slash commands.
//! Catalog entries are suggestions, never claims of model/account support.
mod access;
mod account_choices;
mod chooser;
mod preload;
mod render;
use super::{
    App, Event, KeyCode, KeyModifiers, Result, drafts,
    observe::Update,
    safe,
    state::{Pending, Target},
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub(super) struct Settings {
    #[serde(default)]
    pub account: Option<voyage_protocol::accounts::AccountBinding>,
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub service_tiers: Vec<String>,
    #[serde(default)]
    pub resolution: Option<voyage_protocol::inference::InferenceResolution>,
}
impl Settings {
    pub(super) fn label(&self, field: Field) -> String {
        let (requested, resolved) = match field {
            Field::Account => {
                return self
                    .account
                    .as_ref()
                    .map(|a| a.account_id.to_string())
                    .unwrap_or_else(|| "Host default (unresolved)".into());
            }
            Field::Model => return self.model.clone(),
            Field::Thinking => (
                &self.reasoning_effort,
                self.resolution.as_ref().map(|r| &r.thinking),
            ),
            Field::Service => (
                &self.service_tier,
                self.resolution.as_ref().map(|r| &r.service),
            ),
        };
        if let Some(value) = requested {
            return format!("{value} (explicit)");
        }
        match resolved {
            Some(r) if r.support == voyage_protocol::inference::Support::Unsupported => {
                "Not configurable".into()
            }
            Some(r) if r.default.is_some() => format!(
                "{} (catalog default)",
                r.default.as_deref().unwrap_or("unknown")
            ),
            _ => "Provider-managed (unknown)".into(),
        }
    }
    pub(super) fn resolve(&mut self, models: &[crate::provider::ModelInfo]) {
        if let Ok(provider) =
            serde_json::from_value(serde_json::Value::String(self.provider.clone()))
        {
            let resolution = crate::provider::resolve_inference_values(
                &provider,
                &self.model,
                self.reasoning_effort.as_deref(),
                self.service_tier.as_deref(),
                models.iter().find(|m| m.id == self.model),
            );
            self.reasoning_efforts = resolution.thinking.values.clone();
            self.service_tiers = resolution.service.values.clone();
            self.resolution = Some(resolution);
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Destination {
    Live(Target),
    Draft(Uuid),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Account,
    Model,
    Thinking,
    Service,
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Account => "Account",
            Self::Model => "Model",
            Self::Thinking => "Thinking",
            Self::Service => "Service",
        }
    }
    fn command(self) -> &'static str {
        match self {
            Self::Account => "/account",
            Self::Model => "/model",
            Self::Thinking => "/thinking",
            Self::Service => "/service",
        }
    }
}
pub(super) fn parse(text: &str) -> Option<(Field, &str)> {
    let (command, value) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    Some((
        match command {
            "/account" => Field::Account,
            "/model" => Field::Model,
            "/thinking" => Field::Thinking,
            "/service" => Field::Service,
            _ => return None,
        },
        value.trim(),
    ))
}
fn catalog_payload(
    value: serde_json::Value,
    account: Option<&voyage_protocol::accounts::AccountBinding>,
) -> std::result::Result<serde_json::Value, String> {
    if let Some(account) = account {
        if value["account"] != serde_json::to_value(account).unwrap_or_default() {
            return Err("Account catalog changed".into());
        }
        Ok(serde_json::json!({"value":value["models"],"account_label":value["account_label"]}))
    } else {
        Ok(value.get("result").cloned().unwrap_or(value))
    }
}
fn model_load_error(failure: Option<voyage_protocol::model_discovery::Failure>) -> &'static str {
    use voyage_protocol::model_discovery::Failure;
    match failure {
        Some(Failure::Authentication) => "Couldn’t load models. Check your account’s sign-in.",
        Some(Failure::RateLimit) => "Too many requests. Try again shortly.",
        Some(Failure::Network) => "Couldn’t connect. Check the connection and try again.",
        Some(Failure::InvalidResponse | Failure::DisplayValidation) => {
            "Couldn’t read the model list. Try again."
        }
        Some(Failure::Timeout) => "Loading took too long. Try again.",
        Some(Failure::Configuration) => "Check your account settings, then try again.",
        Some(Failure::Workspace) => "This folder is unavailable. Choose another folder.",
        Some(Failure::Policy) => "Your access settings don’t allow loading models.",
        _ => "Couldn’t load models. Try again.",
    }
}
#[derive(Default)]
pub(super) struct Controls {
    account_load: Option<account_choices::Load>,
    cache: preload::Cache,
    catalog_job: Option<(Uuid, std::time::Instant, tokio::task::JoinHandle<()>)>,
    pub(super) return_to_model: Option<Destination>,
    return_choice: Option<(Settings, chooser::Draft)>,
    options_hit: std::cell::Cell<Option<(ratatui::layout::Rect, Destination)>>,
    chooser_hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, chooser::Control)>>,
    picker: Option<Picker>,
    access: access::AccessControls,
    draft_generations: std::collections::BTreeMap<Uuid, Uuid>,
    visible: std::cell::Cell<bool>,
    cancel_hit: std::cell::Cell<Option<ratatui::layout::Rect>>,
    picker_area: std::cell::Cell<Option<ratatui::layout::Rect>>,

    hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, Destination, Field)>>,
    choices: std::cell::RefCell<Vec<(ratatui::layout::Rect, usize)>>,
}
#[derive(Clone)]
pub(super) struct Picker {
    id: Uuid,
    chooser: chooser::Draft,
    incarnation: Option<Uuid>,
    pub(super) destination: Destination,
    original: Settings,
    models: Vec<crate::provider::ModelInfo>,
    field: Field,
    query: String,
    selected: usize,
    options: Vec<String>,
    loading: bool,
    models_loaded: bool,
    notice: String,
    confirmation: Option<Settings>,
    // Slash commands are retained until their outcome, clicks never touch text.
    command_text: String,
    preserve_draft: bool,
}
impl Picker {
    fn install_models(&mut self, models: Option<Vec<crate::provider::ModelInfo>>) {
        self.loading = false;
        let available = models.is_some();
        self.models_loaded = available;
        let selected = self.options().get(self.selected).cloned();
        self.models = models.unwrap_or_default();
        self.original.resolve(&self.models);
        self.options = match self.field {
            Field::Account => Vec::new(),
            Field::Model => self
                .models
                .iter()
                .map(|m| m.id.clone())
                .chain(std::iter::once(self.original.model.clone()))
                .collect(),
            Field::Thinking => self.original.reasoning_efforts.clone(),
            Field::Service => self.original.service_tiers.clone(),
        };
        if self.field != Field::Model {
            self.options.insert(0, "inherit".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        self.options.retain(|v| {
            safe(v) == *v && !v.chars().any(char::is_whitespace) && seen.insert(v.clone())
        });
        self.selected = selected
            .and_then(|s| self.options().iter().position(|v| *v == s))
            .unwrap_or(0);
        self.notice = if !available {
            "Couldn’t load models. Try again."
        } else if self.field == Field::Model {
            ""
        } else {
            match self.original.resolution.as_ref().map(|r| if self.field == Field::Service { r.service.support } else { r.thinking.support }) {
                Some(voyage_protocol::inference::Support::Advertised) => "Model-advertised choices; account acceptance remains provider-authoritative.",
                Some(voyage_protocol::inference::Support::Unsupported) if self.field != Field::Model => "Explicit overrides are not supported by this adapter/model. Inherit removes an existing override.",
                _ => "Support/default not exposed: choices are encodable suggestions, not account entitlements.",
            }
        }.into();
        if self.field == Field::Service {
            self.notice.push_str(" Priority may cost more. Inherit uses catalog defaults; default disables catalog tier selection (API: standard).");
        }
    }

    fn options(&self) -> Vec<String> {
        if self.confirmation.is_some() {
            return vec![
                "Cancel".into(),
                "Change model and reset both overrides to catalog/provider defaults".into(),
                "Keep overrides (runtime/provider validates compatibility)".into(),
            ];
        }
        let query = self.query.to_lowercase();
        let mut options: Vec<_> = self
            .options
            .iter()
            .filter(|value| value.to_lowercase().contains(&query))
            .cloned()
            .collect();
        // An explicit unknown value is always possible; do not mark it supported.
        if !self.query.trim().is_empty() && !options.iter().any(|v| v == self.query.trim()) {
            options.insert(0, self.query.trim().to_owned());
        }
        options
    }
}
impl App {
    pub(super) fn inference_picker_open(&self) -> bool {
        self.accounts.open()
            || self.inference.picker.is_some()
            || self.inference.access.draft.is_some()
    }

    fn inference_destination(&self) -> Option<Destination> {
        self.active_draft
            .map(Destination::Draft)
            .or_else(|| self.selected.map(Destination::Live))
    }
    pub(super) fn inference_settings(&self, destination: Destination) -> Result<Settings> {
        match destination {
            Destination::Draft(id) => self.draft_inference_settings(id),
            Destination::Live(target) => {
                let view = self.views.get(&target).context("voyage unavailable")?;
                let snapshot = view
                    .snapshot
                    .as_ref()
                    .context("waiting for an authenticated snapshot")?;
                snapshot.inference.clone().context("This runtime does not advertise inference controls; reconnect to an updated runtime")
            }
        }
    }
    pub(super) fn inference_command(
        &mut self,
        destination: Destination,
        text: &str,
        preserve_draft: bool,
    ) -> Result<()> {
        let (field, value) = parse(text).context("invalid inference command")?;
        self.inference.choices.borrow_mut().clear();
        if field == Field::Account {
            self.open_model_options()?;
            return self.load_chooser_accounts();
        }
        let original = self.inference_settings(destination)?;
        match destination {
            Destination::Live(target) => {
                let view = &self.views[&target];
                ensure!(
                    !view.deleted() && !view.archived(),
                    "restore this voyage before changing inference"
                );
                ensure!(
                    view.pending.is_none(),
                    "Inference not sent: another command is pending; Helm checks it automatically. Text preserved"
                );
            }
            Destination::Draft(id) => {
                self.ensure_draft_inference_editable(id)?;
            }
        };
        let mut options = match field {
            Field::Account => Vec::new(),
            Field::Model => vec![original.model.clone()],
            Field::Thinking => original.reasoning_efforts.clone(),
            Field::Service => original.service_tiers.clone(),
        };
        if field != Field::Model {
            options.insert(0, "inherit".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        options.retain(|value| {
            !value.is_empty()
                && safe(value) == *value
                && !value.chars().any(char::is_whitespace)
                && seen.insert(value.clone())
        });
        let mut picker = Picker {
            chooser: chooser::Draft::new(&original),
            incarnation: match destination {Destination::Live(t)=>Some(self.views[&t].process.incarnation),_=>None},
            id: Uuid::new_v4(),
            destination,
            original,
            models: Vec::new(),
            field,
            query: String::new(),
            selected: 0,
            options,
            loading: false,
            models_loaded: false,
            notice: "Loading model metadata. Inherit uses advertised catalog defaults, otherwise provider-managed. Service default is an explicit adapter-specific choice. Unknown support is not entitlement.".into(),
            confirmation: None,
            command_text: match destination {
                Destination::Live(target)
                    if !preserve_draft && self.views[&target].draft.text.trim() == text =>
                {
                    self.views[&target].draft.text.clone()
                }
                _ => text.into(),
            },
            preserve_draft,
        };
        if field == Field::Service {
            picker.notice = format!("Priority may increase cost. {}", picker.notice);
        }
        if !value.is_empty() {
            if field == Field::Model {
                picker.chooser.select(value.to_owned());
                picker.options.push(value.to_owned());
            } else {
                return self.select_inference(picker, value);
            }
        }
        self.inference.picker = Some(picker);
        if !self.use_warm_models() {
            self.load_inference_models()?;
        }
        Ok(())
    }
    pub(super) fn cancel_model_catalog(&mut self) {
        self.inference.cache.foreground = None;
        if let Some((_, _, job)) = self.inference.catalog_job.take() {
            job.abort();
            self.retired_observers.push(job);
        }
    }
    fn load_inference_models(&mut self) -> Result<()> {
        self.cancel_model_catalog();
        let picker = self
            .inference
            .picker
            .as_mut()
            .context("picker unavailable")?;
        picker.id = Uuid::new_v4();
        let id = picker.id;
        let destination = picker.destination;
        let account = if picker.field == Field::Model {
            picker.chooser.account.clone()
        } else {
            picker.original.account.clone()
        };
        picker.loading = true;
        picker.notice = "Loading models…".into();
        self.inference.choices.borrow_mut().clear();
        let provider = if picker.field == Field::Model {
            picker.chooser.provider.clone()
        } else {
            picker.original.provider.clone()
        };
        let setup = self
            .model_scope(destination, account.clone(), provider)
            .and_then(|scope| self.model_command(&scope).map(|command| (scope, command)));
        let (scope, command) = match setup {
            Ok(v) => v,
            Err(_) => {
                self.inference_models(
                    id,
                    None,
                    None,
                    Err("Choose an account or reconnect before loading models".into()),
                );
                return Ok(());
            }
        };
        let route = scope.route;
        let preload = self.take_model_preload(&scope);
        self.inference.cache.foreground = Some((id, scope));
        if let Destination::Draft(draft) = destination {
            self.inference.draft_generations.insert(draft, id);
        }
        let client = self.clients[route].clone();
        let sender = self.sender.clone();
        let job = tokio::spawn(async move {
            let result = if let Some(mut job) = preload {
                (&mut job.receiver)
                    .await
                    .unwrap_or_else(|_| Err("Model preload stopped".into()))
            } else {
                preload::fetch(client, command, account).await
            };
            let _ = sender
                .send(Update::InferenceModels {
                    route: Some(route),
                    id,
                    context: None,
                    generation: matches!(destination, Destination::Draft(_)).then_some(id),
                    result,
                })
                .await;
        });
        self.inference.catalog_job = Some((
            id,
            std::time::Instant::now() + std::time::Duration::from_secs(12),
            job,
        ));
        Ok(())
    }
    fn poll_model_catalog(&mut self) {
        let Some((id, deadline, _)) = self.inference.catalog_job.as_ref() else {
            return;
        };
        let id = *id;
        if !self
            .inference
            .picker
            .as_ref()
            .is_some_and(|p| p.id == id && p.loading)
        {
            self.cancel_model_catalog();
            return;
        }
        let stale = self
            .inference
            .cache
            .foreground
            .as_ref()
            .is_some_and(|(_, scope)| !self.model_scope_current(scope));
        if stale || std::time::Instant::now() >= *deadline {
            // Back off even when the foreground deadline wins the response race.
            self.cache_model_response(id, &Err("Model loading stopped".into()));
            self.cancel_model_catalog();
            if let Some(p) = self.inference.picker.as_mut().filter(|p| p.id == id) {
                p.loading = false;
                p.models_loaded = false;
                p.id = Uuid::new_v4();
                p.notice = "Loading took too long. Try again.".into();
            }
        }
    }
    pub(super) fn sync_live_inference_picker(&mut self, target: Target) {
        let Some(latest) = self
            .views
            .get(&target)
            .and_then(|v| v.snapshot.as_ref())
            .and_then(|s| s.inference.as_ref())
        else {
            return;
        };
        let Some(picker) = self.inference.picker.as_mut().filter(|p| {
            p.destination == Destination::Live(target) && !p.loading && p.field != Field::Model
        }) else {
            return;
        };
        if picker.original.account != latest.account
            || picker.original.model != latest.model
            || picker.original.provider != latest.provider
            || picker.original.reasoning_effort != latest.reasoning_effort
            || picker.original.service_tier != latest.service_tier
        {
            picker.notice = "Settings changed in another view. Reopen this picker before applying; nothing sent.".into();
            return;
        }
        if picker.original.resolution != latest.resolution {
            picker.original = latest.clone();
            picker.models.clear();
            picker.options = if picker.field == Field::Thinking {
                latest.reasoning_efforts.clone()
            } else {
                latest.service_tiers.clone()
            };
            picker.options.insert(0, "inherit".into());
            picker.selected = 0;
            self.inference.choices.borrow_mut().clear();
            picker.notice = "Executing-host capabilities refreshed. Unknown support is not entitlement; inherited service defaults may affect cost.".into();
        }
    }
    pub(super) fn inference_models(
        &mut self,
        id: Uuid,
        _context: Option<[u8; 32]>,
        generation: Option<Uuid>,
        result: std::result::Result<serde_json::Value, String>,
    ) {
        if !self.cache_model_response(id, &result) {
            if let Some(p) = self.inference.picker.as_mut().filter(|p| p.id == id) {
                p.loading = false;
                p.notice = "Connection changed. Reload the model list.".into();
            }
            return;
        }
        if self
            .inference
            .catalog_job
            .as_ref()
            .is_some_and(|(job_id, _, _)| *job_id == id)
        {
            self.cancel_model_catalog();
        }
        let stale = self
            .inference
            .picker
            .as_ref()
            .filter(|p| p.id == id)
            .is_some_and(|p| {
                self.inference_settings(p.destination)
                    .map_or(true, |current| {
                        current.account != p.original.account
                            || current.provider != p.original.provider
                            || current.model != p.original.model
                    })
            });
        if stale {
            if let Some(picker) = self.inference.picker.as_mut() {
                picker.loading = false;
                picker.notice = "Account or model changed. Reopen the model list.".into();
            }
            return;
        }
        if let Some((destination, settings)) = self
            .inference
            .picker
            .as_ref()
            .filter(|p| p.id == id)
            .map(|p| {
                let mut settings = p.original.clone();
                if p.field == Field::Model {
                    settings.account = p.chooser.account.clone();
                }
                (p.destination, settings)
            })
            && let Some(label) = result
                .as_ref()
                .ok()
                .and_then(|v| v["account_label"].as_str())
        {
            self.cache_model_account_label(destination, &settings, label);
        }
        let Some(picker) = self.inference.picker.as_mut().filter(|p| p.id == id) else {
            return;
        };
        if let Destination::Draft(draft_id) = picker.destination
            && self.inference.draft_generations.get(&draft_id).copied() != generation
        {
            picker.loading = false;
            picker.notice = "Settings changed. Reload the model list.".into();
            return;
        }
        self.inference.choices.borrow_mut().clear();
        let diagnostic = result
            .as_ref()
            .err()
            .and_then(|e| voyage_protocol::model_discovery::Failure::from_diagnostic(e));
        let failed = result.is_err();
        let models = result
            .ok()
            .and_then(|value| {
                let value = value.get("value").unwrap_or(&value);
                let value = value.get("inventory").unwrap_or(value);
                serde_json::from_value::<Vec<crate::provider::ModelInfo>>(value.clone()).ok()
            })
            .filter(|models| crate::provider::validate_models(models, &[]).is_ok());
        let account_changed =
            picker.field == Field::Model && picker.chooser.account != picker.original.account;
        let loaded = models.clone();
        let selected = picker.options().get(picker.selected).cloned();
        picker.install_models(models);
        if account_changed {
            picker.options = loaded
                .as_ref()
                .map(|v| v.iter().map(|m| m.id.clone()).collect())
                .unwrap_or_default();
            if loaded.is_some() && !picker.options.contains(&picker.chooser.model) {
                picker.chooser.model = loaded
                    .as_ref()
                    .and_then(|v| v.iter().find(|m| m.is_default).or(v.first()))
                    .map(|m| m.id.clone())
                    .unwrap_or_default();
            }
        }
        picker.selected = selected
            .and_then(|s| picker.options().iter().position(|v| *v == s))
            .unwrap_or(0);
        if failed {
            picker.notice = model_load_error(diagnostic).into();
        }
    }
    pub(super) fn refresh_draft_capabilities(&mut self) {
        self.account_tick();
        self.poll_model_catalog();
        self.poll_chooser_accounts();
        self.resume_model_after_account();
        self.warm_selected_models();
    }
    fn select_inference(&mut self, mut picker: Picker, value: &str) -> Result<()> {
        ensure!(
            !value.is_empty()
                && value.len() <= 256
                && !value.chars().any(char::is_whitespace)
                && safe(value) == value,
            "Select one model ID or setting value (at most 256 bytes)"
        );
        let mut settings = picker.original.clone();
        match picker.field {
            Field::Account => unreachable!("account has a dedicated private picker"),
            Field::Model => settings.model = value.into(),
            Field::Thinking => {
                settings.reasoning_effort =
                    (!matches!(value, "inherit" | "default")).then(|| value.into())
            }
            Field::Service => settings.service_tier = (value != "inherit").then(|| value.into()),
        }
        settings.resolve(&picker.models);
        if picker.field == Field::Model
            && settings.model != picker.original.model
            && (settings.reasoning_effort.is_some() || settings.service_tier.is_some())
        {
            self.inference.choices.borrow_mut().clear();
            picker.confirmation = Some(settings);
            picker.selected = 0;
            picker.notice = "The new model may be incompatible with your overrides. Nothing has changed. Choose an atomic reset or explicitly keep them for runtime/provider validation.".into();
            self.inference.picker = Some(picker);
            return Ok(());
        }
        self.apply_inference(picker, settings)
    }
    fn apply_inference(&mut self, picker: Picker, settings: Settings) -> Result<()> {
        let latest = self.inference_settings(picker.destination)?;
        ensure!(
            latest.model == picker.original.model
                && latest.reasoning_effort == picker.original.reasoning_effort
                && latest.service_tier == picker.original.service_tier
                && latest.provider == picker.original.provider
                && latest.account == picker.original.account,
            "Inference settings changed while selecting; reopen the selector. Nothing sent; text preserved"
        );
        match picker.destination {
            Destination::Draft(id) => {
                self.save_draft_inference(
                    id,
                    &settings,
                    (!picker.preserve_draft).then_some(picker.command_text.as_str()),
                )?;
                self.status = "Inference applied to draft · model/account support is validated when sending. Text preserved.".into();
            }
            Destination::Live(target) => {
                ensure!(
                    self.clients.available(target.route),
                    "Vessel unavailable · Ctrl+G to manage / retry; settings retained"
                );
                let account_host = self.account_host(target.route);
                ensure!(
                    settings.account.is_none() || account_host.is_some(),
                    "Review /account on this authenticated Vessel before changing account-bound settings"
                );
                let view = self.views.get_mut(&target).context("voyage unavailable")?;
                ensure!(
                    view.pending.is_none(),
                    "Another command is pending; Helm checks it automatically. Text preserved"
                );
                let snapshot = view.snapshot.as_ref().context("snapshot unavailable")?;
                // This is a session-scoped setting, not a live-resource action.
                // Use the latest observed revision after confirming the selected
                // values did not change; ordinary checkpoints need not close a picker.
                let command_id = Uuid::new_v4();
                let expires_at_ms = u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_millis(),
                )?
                .saturating_add(60_000);
                let command = if let Some(account) = settings.account.clone() {
                    VoyageCommand::SetAccountInference {
                        command_id,
                        expected_revision: snapshot.revision,
                        expires_at_ms,
                        model: settings.model,
                        reasoning_effort: settings.reasoning_effort,
                        service_tier: settings.service_tier,
                        account,
                    }
                } else {
                    VoyageCommand::SetInference {
                        command_id,
                        expected_revision: snapshot.revision,
                        expires_at_ms,
                        model: settings.model,
                        reasoning_effort: settings.reasoning_effort,
                        service_tier: settings.service_tier,
                    }
                };
                let active = snapshot.run.as_ref().is_some_and(|r| r.active());
                view.pending = Some(Pending {
                    account_host,
                    command_id,
                    incarnation: view.process.incarnation,
                    draft: picker.command_text,
                    preserve_draft: picker.preserve_draft,
                    original: Some(Box::new(command.clone())),
                    receipt_only: false,
                });
                if let Err(error) = drafts::save(&self.clients[target.route], view) {
                    view.pending = None;
                    return Err(
                        error.context("cannot persist inference command identity; nothing sent")
                    );
                }
                self.dispatch(target, command_id, command);
                self.status = if active { "Inference pending for next turn (current turn unchanged) · text preserved · Checking automatically" } else { "Inference pending · text preserved · Checking automatically" }.into();
            }
        }
        Ok(())
    }
    pub(super) fn inference_input(&mut self, event: &Event) -> Result<bool> {
        if self
            .inference
            .picker
            .as_ref()
            .is_some_and(|p| p.field == Field::Model)
        {
            return self.model_chooser_input(event);
        }
        use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
        if self.account_input(event)? {
            return Ok(true);
        }
        if self.composer_access_input(event)? {
            return Ok(true);
        }
        if let Event::Resize(..) = event {
            self.inference.visible.set(false);
            self.inference.cancel_hit.set(None);
            self.inference.picker_area.set(None);
            self.inference.hits.borrow_mut().clear();
            self.inference.choices.borrow_mut().clear();
        }
        if self.inference.picker.is_none() {
            if self.help
                || self.explore.is_some()
                || self.sidebar.menu.is_some()
                || self.interactions.borrow().focused
            {
                return Ok(false);
            }
            if let Event::Mouse(mouse) = event
                && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            {
                let field = self
                    .inference
                    .hits
                    .borrow()
                    .iter()
                    .find(|(r, destination, _)| {
                        Some(*destination) == self.inference_destination()
                            && r.contains((mouse.column, mouse.row).into())
                    })
                    .map(|(_, _, f)| *f);
                if let Some(field) = field
                    && let Some(destination) = self.inference_destination()
                {
                    self.inference_command(destination, field.command(), true)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        if matches!(event, Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Up(_) | MouseEventKind::Moved))
        {
            return Ok(true);
        }
        if !self.inference.visible.get()
            && !matches!(event, Event::Key(key) if key.code == KeyCode::Esc || (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))))
        {
            self.status = "Inference selector is not visible yet; enlarge to at least 40 columns × 18 rows, or Esc to cancel. Nothing sent.".into();
            return Ok(true);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .inference
                .cancel_hit
                .get()
                .is_some_and(|r| r.contains((mouse.column, mouse.row).into()))
        {
            self.inference.picker = None;
            self.inference.cancel_hit.set(None);
            self.inference.choices.borrow_mut().clear();
            return Ok(true);
        }
        let mut picker = self.inference.picker.take().expect("picker");
        let mut choose = None;
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc {
                    return Ok(true);
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c' | 'q'))
                {
                    self.quit = true;
                    return Ok(true);
                }
                let count = picker.options().len();
                match key.code {
                    KeyCode::Up => {
                        picker.selected = (picker.selected.min(count.saturating_sub(1))
                            + count.saturating_sub(1))
                            % count.max(1)
                    }
                    KeyCode::Down => picker.selected = (picker.selected + 1) % count.max(1),
                    KeyCode::Enter => choose = Some(picker.selected),
                    KeyCode::Backspace if picker.confirmation.is_none() => {
                        picker.query.pop();
                        picker.selected = 0;
                    }
                    KeyCode::Char(ch)
                        if picker.confirmation.is_none()
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && picker.query.len() + ch.len_utf8() <= 256 =>
                    {
                        picker.query.push(ch);
                        picker.selected = 0;
                    }
                    _ => {}
                }
            }
            Event::Paste(text) if picker.confirmation.is_none() => {
                let text = safe(text);
                if picker.query.len() + text.len() <= 256 {
                    picker.query.push_str(&text);
                    picker.selected = 0;
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) && self
                    .inference
                    .picker_area
                    .get()
                    .is_some_and(|r| r.contains((mouse.column, mouse.row).into())) =>
            {
                let count = picker.options().len();
                picker.selected = if mouse.kind == MouseEventKind::ScrollUp {
                    picker.selected.saturating_sub(1)
                } else {
                    (picker.selected + 1).min(count.saturating_sub(1))
                };
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                choose = self
                    .inference
                    .choices
                    .borrow()
                    .iter()
                    .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                    .map(|(_, i)| *i)
            }
            _ => {}
        }
        self.inference.choices.borrow_mut().clear();
        self.inference.choices.borrow_mut().clear();
        if let Some(index) = choose {
            if let Some(mut settings) = picker.confirmation.take() {
                match index {
                    0 => return Ok(true),
                    1 => {
                        settings.reasoning_effort = None;
                        settings.service_tier = None;
                    }
                    2 => {}
                    _ => {
                        self.inference.picker = Some(picker);
                        return Ok(true);
                    }
                }
                self.apply_inference(picker, settings)?;
                return Ok(true);
            }
            if let Some(value) = picker.options().get(index).cloned() {
                self.select_inference(picker, &value)?;
                return Ok(true);
            }
        }
        self.inference.picker = Some(picker);
        Ok(true)
    }
}

#[cfg(test)]
mod catalog_payload_tests {
    use super::*;
    #[test]
    fn unbound_controls_unwrap_voyage_envelope_without_losing_inventory() {
        let payload = serde_json::json!({"session_id":Uuid::new_v4(),"incarnation":Uuid::new_v4(),"result":{"section":"models","value":[{"id":"test-model","display_name":"Test"}]}});
        let value = catalog_payload(payload, None).unwrap();
        assert_eq!(value["value"][0]["id"], "test-model");
        let direct = serde_json::json!({"section":"models","value":[]});
        assert_eq!(catalog_payload(direct.clone(), None).unwrap(), direct);
    }
}
