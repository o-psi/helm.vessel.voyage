//! One inference action path for composer controls and slash commands.
//! Catalog entries are suggestions, never claims of model/account support.
mod access;
mod render;
use super::{
    App, Event, KeyCode, KeyModifiers, Result, drafts,
    observe::Update,
    safe,
    state::{Pending, Target},
};
use anyhow::{Context, ensure};
use serde::Deserialize;
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

#[derive(Clone, Default, Deserialize, PartialEq)]
pub(super) struct Settings {
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
    fn label(&self, field: Field) -> String {
        let (requested, resolved) = match field {
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
    fn resolve(&mut self, models: &[crate::provider::ModelInfo]) {
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
    Model,
    Thinking,
    Service,
}
impl Field {
    fn name(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Thinking => "Thinking",
            Self::Service => "Service",
        }
    }
    fn command(self) -> &'static str {
        match self {
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
            "/model" => Field::Model,
            "/thinking" => Field::Thinking,
            "/service" => Field::Service,
            _ => return None,
        },
        value.trim(),
    ))
}
#[derive(Default)]
pub(super) struct Controls {
    picker: Option<Picker>,
    access: access::AccessControls,
    catalogs: std::collections::BTreeMap<Uuid, DraftCatalog>,
    draft_generations: std::collections::BTreeMap<Uuid, Uuid>,
    draft_checks: std::collections::BTreeMap<Uuid, std::time::Instant>,
    visible: std::cell::Cell<bool>,
    hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, Destination, Field)>>,
    choices: std::cell::RefCell<Vec<(ratatui::layout::Rect, usize)>>,
}
struct DraftCatalog {
    provider: String,
    context: [u8; 32],
    observed: std::time::Instant,
    models: Vec<crate::provider::ModelInfo>,
}
struct Picker {
    id: Uuid,
    destination: Destination,
    original: Settings,
    models: Vec<crate::provider::ModelInfo>,
    field: Field,
    query: String,
    selected: usize,
    options: Vec<String>,
    loading: bool,
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
        self.models = models.unwrap_or_default();
        self.original.resolve(&self.models);
        self.options = match self.field {
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
        self.selected = 0;
        self.notice = if !available {
            "Catalog unavailable or context changed. Defaults/support are unknown. Explicit values receive runtime validation; reopen to retry."
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
        self.inference.picker.is_some() || self.inference.access.draft.is_some()
    }

    fn inference_destination(&self) -> Option<Destination> {
        self.active_draft
            .map(Destination::Draft)
            .or_else(|| self.selected.map(Destination::Live))
    }
    fn inference_settings(&self, destination: Destination) -> Result<Settings> {
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
            id: Uuid::new_v4(),
            destination,
            original,
            models: Vec::new(),
            field,
            query: String::new(),
            selected: 0,
            options,
            loading: false,
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
            return self.select_inference(picker, value);
        }
        self.inference.picker = Some(picker);
        self.load_inference_models()?;
        Ok(())
    }
    fn load_inference_models(&mut self) -> Result<()> {
        let picker = self
            .inference
            .picker
            .as_mut()
            .context("picker unavailable")?;
        let id = picker.id;
        let destination = picker.destination;
        picker.loading = true;
        let sender = self.sender.clone();
        match destination {
            Destination::Live(target) => {
                let view = &self.views[&target];
                let incarnation = view.process.incarnation;
                let run_id = view
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.run.as_ref())
                    .filter(|r| r.active())
                    .map(|r| r.run_id);
                let client = self.clients[target.route].clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(25),
                        client.voyage(
                            target.session,
                            incarnation,
                            VoyageCommand::Controls {
                                run_id,
                                section: "models".into(),
                            },
                        ),
                    )
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                    let _ = sender
                        .send(Update::InferenceModels {
                            route: Some(target.route),
                            id,
                            context: None,
                            generation: None,
                            result,
                        })
                        .await;
                });
            }
            Destination::Draft(draft_id) => {
                let (config, workspace) = self.draft_inference_catalog(draft_id)?;
                self.inference.draft_generations.insert(draft_id, id);
                self.inference
                    .draft_checks
                    .insert(draft_id, std::time::Instant::now());
                tokio::spawn(async move {
                    let context = crate::provider::inference_context(&config).await;
                    let mut result =
                        crate::process_client::frontend::models::discover(&config, &workspace)
                            .await
                            .and_then(|models| Ok(serde_json::to_value(models)?))
                            .map_err(|e| e.to_string());
                    if context.is_none()
                        || context != crate::provider::inference_context(&config).await
                    {
                        result = Err("catalog context changed".into());
                    }
                    let _ = sender
                        .send(Update::InferenceModels {
                            route: None,
                            id,
                            context,
                            generation: Some(id),
                            result,
                        })
                        .await;
                });
            }
        }
        Ok(())
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
        if picker.original.model != latest.model
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
        context: Option<[u8; 32]>,
        generation: Option<Uuid>,
        result: std::result::Result<serde_json::Value, String>,
    ) {
        let Some(picker) = self.inference.picker.as_mut().filter(|p| p.id == id) else {
            return;
        };
        if let Destination::Draft(draft_id) = picker.destination
            && self.inference.draft_generations.get(&draft_id).copied() != generation
        {
            return;
        }
        self.inference.choices.borrow_mut().clear();
        let models = result
            .ok()
            .and_then(|value| {
                let value = value.get("value").unwrap_or(&value);
                let value = value.get("inventory").unwrap_or(value);
                serde_json::from_value::<Vec<crate::provider::ModelInfo>>(value.clone()).ok()
            })
            .filter(|models| crate::provider::validate_models(models, &[]).is_ok());
        if let Destination::Draft(draft_id) = picker.destination {
            self.inference.catalogs.remove(&draft_id);
            if let (Some(context), Some(models)) = (context, models.as_ref()) {
                self.inference.catalogs.insert(
                    draft_id,
                    DraftCatalog {
                        provider: picker.original.provider.clone(),
                        context,
                        observed: std::time::Instant::now(),
                        models: models.clone(),
                    },
                );
            }
        }
        picker.install_models(models);
    }
    pub(super) fn refresh_draft_capabilities(&mut self) {
        let Some(id) = self.active_draft else {
            return;
        };
        if self
            .inference
            .picker
            .as_ref()
            .is_some_and(|p| p.destination == Destination::Draft(id) && p.loading)
            || self
                .inference
                .draft_checks
                .get(&id)
                .is_some_and(|at| at.elapsed().as_secs() < 30)
        {
            return;
        }
        if self.ensure_draft_inference_editable(id).is_err() {
            return;
        }
        let Ok((config, workspace)) = self.draft_inference_catalog(id) else {
            return;
        };
        self.inference
            .draft_checks
            .retain(|id, _| self.new_drafts.contains_key(id));
        self.inference
            .draft_generations
            .retain(|id, _| self.new_drafts.contains_key(id));
        self.inference.catalogs.retain(|id, c| {
            self.new_drafts.contains_key(id) && c.observed.elapsed().as_secs() < 300
        });
        self.inference
            .draft_checks
            .insert(id, std::time::Instant::now());
        let generation = Uuid::new_v4();
        self.inference.draft_generations.insert(id, generation);
        let previous = self
            .inference
            .catalogs
            .get(&id)
            .map(|c| (c.context, c.observed));
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let context = crate::provider::inference_context(&config).await;
            if let (Some(context), Some((key, at))) = (context, previous)
                && key == context
                && at.elapsed().as_secs() < 300
            {
                return;
            }
            let provider = serde_json::to_value(&config.provider)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let result =
                crate::process_client::frontend::models::discover(&config, &workspace).await;
            let models = if context.is_some()
                && context == crate::provider::inference_context(&config).await
            {
                result.ok()
            } else {
                None
            };
            let _ = sender
                .send(Update::DraftInferenceModels {
                    id,
                    provider,
                    context,
                    generation,
                    models,
                })
                .await;
        });
    }
    pub(super) fn draft_inference_models(
        &mut self,
        id: Uuid,
        provider: String,
        context: Option<[u8; 32]>,
        generation: Uuid,
        models: Option<Vec<crate::provider::ModelInfo>>,
    ) {
        if !self.new_drafts.contains_key(&id)
            || self.inference.draft_generations.get(&id) != Some(&generation)
        {
            return;
        }
        self.inference.catalogs.remove(&id);
        let models = models.filter(|models| {
            context.is_some() && crate::provider::validate_models(models, &[]).is_ok()
        });
        if let (Some(context), Some(models)) = (context, models.as_ref()) {
            self.inference.catalogs.insert(
                id,
                DraftCatalog {
                    provider,
                    context,
                    observed: std::time::Instant::now(),
                    models: models.clone(),
                },
            );
        }
        if let Some(picker) = self
            .inference
            .picker
            .as_mut()
            .filter(|p| p.destination == Destination::Draft(id))
        {
            self.inference.choices.borrow_mut().clear();
            picker.install_models(models);
        }
    }
    pub(super) fn draft_known_model(
        &self,
        id: Uuid,
        provider: &str,
        model: &str,
    ) -> Option<&crate::provider::ModelInfo> {
        self.inference
            .catalogs
            .get(&id)
            .filter(|c| c.provider == provider && c.observed.elapsed().as_secs() < 300)
            .and_then(|c| c.models.iter().find(|m| m.id == model))
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
                && latest.provider == picker.original.provider,
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
                let command = VoyageCommand::SetInference {
                    command_id,
                    expected_revision: snapshot.revision,
                    expires_at_ms,
                    model: settings.model,
                    reasoning_effort: settings.reasoning_effort,
                    service_tier: settings.service_tier,
                };
                let active = snapshot.run.as_ref().is_some_and(|r| r.active());
                view.pending = Some(Pending {
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
        use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
        if self.composer_access_input(event)? {
            return Ok(true);
        }
        if let Event::Resize(..) = event {
            self.inference.visible.set(false);
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
        if !self.inference.visible.get()
            && !matches!(event, Event::Key(key) if key.code == KeyCode::Esc || (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))))
        {
            self.status = "Inference selector is not visible yet; enlarge to at least 40 columns × 18 rows, or Esc to cancel. Nothing sent.".into();
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
