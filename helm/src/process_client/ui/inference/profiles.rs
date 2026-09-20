//! Vessel-owned named execution settings. Selection copies values into the voyage;
//! profile edits never change an existing voyage's saved settings.
use super::*;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph},
};
use voyage_protocol::{
    execution_profiles::{ExecutionProfile, ProfileCatalogue},
    vessel::VesselCommand,
};

#[derive(Default)]
pub(super) struct Controls {
    pub(super) panel: Option<Panel>,
    pub(super) editing: Option<(Uuid, String)>,
    job: Option<
        tokio::sync::oneshot::Receiver<
            Result<(Uuid, ProfileCatalogue, Vec<account_choices::Choice>)>,
        >,
    >,
    labels: Vec<ExecutionProfile>,
    accounts: Vec<account_choices::Choice>,
    pending_save: Option<ExecutionProfile>,
    automatic: bool,
    hits: std::cell::RefCell<Vec<(Rect, KeyCode)>>,
    rows: std::cell::RefCell<Vec<(Rect, usize)>>,
}
pub(super) struct Panel {
    destination: Destination,
    catalogue: Option<ProfileCatalogue>,
    selected: usize,
    name: Option<(Uuid, String, Option<ExecutionProfile>)>,
    delete_confirm: bool,
    notice: String,
}
fn settings(profile: &ExecutionProfile) -> Settings {
    Settings {
        account: Some(profile.account.clone()),
        provider: match profile.account.transport {
            voyage_protocol::accounts::Transport::ChatgptOauth => "chatgpt-oauth",
            voyage_protocol::accounts::Transport::OpenaiResponses => "openai-responses",
            voyage_protocol::accounts::Transport::OpenaiChat => "openai-chat",
            voyage_protocol::accounts::Transport::Anthropic => "anthropic",
        }
        .into(),
        model: profile.model.clone(),
        reasoning_effort: profile.reasoning_effort.clone(),
        service_tier: profile.service_tier.clone(),
        ..Default::default()
    }
}
fn copy_name(name: &str) -> String {
    let mut end = name.len().min(75);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} copy", name[..end].trim_end())
}
fn matches(profile: &ExecutionProfile, value: &Settings) -> bool {
    value.account.as_ref() == Some(&profile.account)
        && value.model == profile.model
        && value.reasoning_effort == profile.reasoning_effort
        && value.service_tier == profile.service_tier
}
impl App {
    pub(in crate::process_client::ui) fn open_initial_profiles(&mut self) -> Result<()> {
        self.open_profiles()?;
        self.inference.profiles.automatic = true;
        Ok(())
    }
    pub(in crate::process_client::ui) fn open_profiles(&mut self) -> Result<()> {
        self.inference.profiles.automatic = false;
        ensure!(
            self.inference.profiles.job.is_none(),
            "A profile operation is still pending"
        );
        let destination = self
            .inference_destination()
            .context("Start a voyage draft first")?;
        let (_, workspace) = self.account_destination(destination)?;
        self.cancel_paste_for_private_panel();
        self.inference.profiles.panel = Some(Panel {
            destination,
            catalogue: None,
            selected: 0,
            name: None,
            delete_confirm: false,
            notice: "Loading profiles…".into(),
        });
        self.inference.profiles.editing = None;
        self.request_profiles(VesselCommand::Profiles { workspace })
    }
    fn request_profiles(&mut self, command: VesselCommand) -> Result<()> {
        ensure!(
            self.inference.profiles.job.is_none(),
            "A profile operation is still pending"
        );
        let destination = self
            .inference
            .profiles
            .panel
            .as_ref()
            .context("Profiles closed")?
            .destination;
        let (route, workspace) = self.account_destination(destination)?;
        let client = self.clients[route].clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let caps = client.request(VesselCommand::Capabilities).await?;
                let host: Uuid = serde_json::from_value(caps["vessel_id"].clone())?;
                ensure!(
                    !host.is_nil() && client.managed().is_none_or(|c| c.vessel_id == host),
                    "Profile host changed"
                );
                // Metadata is optional: unavailable account labels must never turn
                // a successfully saved profile into an apparent failed mutation.
                let accounts = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    client.request(VesselCommand::Accounts {
                        workspace,
                        transport: None,
                    }),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .and_then(|value| serde_json::from_value::<account_choices::Catalogue>(value).ok())
                .and_then(|catalogue| catalogue.choices().ok())
                .unwrap_or_default();
                let catalogue = serde_json::from_value(client.request(command).await?)?;
                Ok((host, catalogue, accounts))
            })
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Profile request timed out; reload to check the outcome before retrying"
                ))
            });
            let _ = tx.send(result);
        });
        self.inference.profiles.job = Some(rx);
        Ok(())
    }
    pub(super) fn poll_profiles(&mut self) {
        let Some(mut rx) = self.inference.profiles.job.take() else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.inference.profiles.job = Some(rx);
                return;
            }
            Err(_) => Err(anyhow::anyhow!(
                "Profile request interrupted; reload to check its outcome"
            )),
        };
        let Some(panel) = self.inference.profiles.panel.as_ref() else {
            return;
        };
        if self.inference_destination() != Some(panel.destination) {
            self.inference.profiles.panel = None;
            return;
        }
        let destination = panel.destination;
        match result {
            Ok((host, catalogue, accounts)) => {
                self.inference.profiles.accounts = accounts;
                self.inference.profiles.pending_save = None;
                if let Ok((route, _)) = self.account_destination(destination) {
                    self.cache_account_host(route, host);
                }
                self.inference.profiles.labels = catalogue.profiles.clone();
                let panel = self.inference.profiles.panel.as_mut().unwrap();
                panel.selected = panel
                    .selected
                    .min(catalogue.profiles.len().saturating_sub(1));
                panel.notice = if catalogue.profiles.is_empty() { if catalogue.can_manage { "Create a profile to choose execution settings." } else { "No profiles available for your accounts. Ask the Vessel owner to create one." } } else { "Select a profile to copy its settings. Editing profiles leaves existing voyages unchanged." }.into();
                let default = catalogue
                    .default_profile_id
                    .and_then(|id| catalogue.profiles.iter().find(|p| p.id == id))
                    .cloned();
                panel.catalogue = Some(catalogue);
                if self.inference.profiles.automatic {
                    self.inference.profiles.automatic = false;
                    if let Some(profile) = default {
                        if let Err(e) = self.use_profile(destination, profile) {
                            self.status = safe(&e.to_string());
                        }
                    }
                }
            }
            Err(e) => {
                let panel = self.inference.profiles.panel.as_mut().unwrap();
                panel.notice = safe(&e.to_string());
                if let Some(profile) = self.inference.profiles.pending_save.take() {
                    panel.name = Some((profile.id, profile.name.clone(), Some(profile)));
                    panel.notice.push_str(
                        " · Settings retained. Ctrl+R reloads profiles before saving again.",
                    );
                }
            }
        }
    }
    fn profile_account_label(&self, profile: &ExecutionProfile) -> String {
        self.inference
            .profiles
            .accounts
            .iter()
            .find(|choice| choice.binding == profile.account)
            .map(|choice| {
                format!(
                    "{}{}",
                    safe(&choice.label),
                    if choice.ready { "" } else { " (unavailable)" }
                )
            })
            .unwrap_or_else(|| format!("Unavailable account ({})", profile.account.account_id))
    }
    pub(super) fn profile_label(&self, value: &Settings) -> String {
        self.inference
            .profiles
            .labels
            .iter()
            .find(|p| matches(p, value))
            .map(|p| format!("{} · {}", safe(&p.name), safe(&p.model)))
            .unwrap_or_else(|| format!("Saved settings · {}", safe(&value.model)))
    }
    fn edit_profile(
        &mut self,
        destination: Destination,
        id: Uuid,
        name: String,
        seed: Option<ExecutionProfile>,
    ) -> Result<()> {
        self.inference.profiles.editing = Some((id, name));
        self.open_model_options()?;
        if let (Some(profile), Some(p)) = (seed, self.inference.picker.as_mut()) {
            p.chooser = chooser::Draft::new(&settings(&profile));
            p.chooser.keep = true;
        }
        ensure!(
            self.inference_destination() == Some(destination),
            "Voyage changed"
        );
        self.load_chooser_accounts()?;
        Ok(())
    }
    pub(super) fn save_profile_settings(&mut self, value: Settings) -> Result<()> {
        let (id, name) = self
            .inference
            .profiles
            .editing
            .clone()
            .context("Profile editor closed")?;
        let panel = self
            .inference
            .profiles
            .panel
            .as_ref()
            .context("Profiles closed")?;
        let catalogue = panel.catalogue.as_ref().context("Reload profiles first")?;
        ensure!(
            catalogue.can_manage,
            "Profile management requires executing-host administration access"
        );
        let (_, workspace) = self.account_destination(panel.destination)?;
        let profile = ExecutionProfile {
            id,
            name,
            account: value.account.context("Choose a provider account")?,
            model: value.model,
            reasoning_effort: value.reasoning_effort,
            service_tier: value.service_tier,
        };
        let command = VesselCommand::SaveProfile {
            command_id: Uuid::new_v4(),
            workspace,
            expected_revision: catalogue.revision,
            profile: profile.clone(),
            make_default: catalogue.profiles.is_empty(),
        };
        self.request_profiles(command)?;
        self.inference.profiles.pending_save = Some(profile);
        self.inference.profiles.editing = None;
        self.cancel_model_catalog();
        self.cancel_chooser_accounts();
        Ok(())
    }
    fn use_profile(&mut self, destination: Destination, profile: ExecutionProfile) -> Result<()> {
        let value = settings(&profile);
        if let Destination::Draft(id) = destination {
            let (route, _) = self.account_destination(destination)?;
            let host = self
                .account_host(route)
                .context("Authenticated host unavailable")?;
            self.set_draft_account(id, host, value)?;
        } else {
            self.inference_command(destination, "/model", true)?;
            let picker = self
                .inference
                .picker
                .take()
                .context("Profile selection unavailable")?;
            self.cancel_model_catalog();
            self.apply_inference(picker, value)?;
        }
        self.inference.profiles.panel = None;
        self.status = format!(
            "Profile {} selected · settings copied into this voyage",
            safe(&profile.name)
        );
        Ok(())
    }
    pub(super) fn profiles_input(&mut self, event: &Event) -> Result<bool> {
        if self.inference.profiles.editing.is_some() {
            if self.inference.picker.is_some()
                || self.accounts.open()
                || self.inference.return_to_model.is_some()
            {
                return Ok(false);
            }
            self.inference.profiles.editing = None;
        }
        let Some(mut panel) = self.inference.profiles.panel.take() else {
            return Ok(false);
        };
        if self.inference_destination() != Some(panel.destination) {
            return Ok(false);
        }
        let key = match event {
            Event::Key(key) => *key,
            Event::Mouse(mouse)
                if mouse.kind
                    == crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left,
                    ) =>
            {
                if !panel.delete_confirm
                    && panel.name.is_none()
                    && self.inference.profiles.job.is_none()
                    && let Some(index) = self
                        .inference
                        .profiles
                        .rows
                        .borrow()
                        .iter()
                        .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                        .map(|(_, index)| *index)
                {
                    panel.selected = index;
                    self.inference.profiles.panel = Some(panel);
                    return Ok(true);
                }
                let action = self
                    .inference
                    .profiles
                    .hits
                    .borrow()
                    .iter()
                    .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                    .map(|(_, k)| *k);
                self.inference.profiles.panel = Some(panel);
                if let Some(code) = action {
                    return self.profiles_input(&Event::Key(crossterm::event::KeyEvent::new(
                        code,
                        KeyModifiers::NONE,
                    )));
                }
                return Ok(true);
            }
            _ => {
                self.inference.profiles.panel = Some(panel);
                return Ok(true);
            }
        };
        if key.kind != crossterm::event::KeyEventKind::Press {
            self.inference.profiles.panel = Some(panel);
            return Ok(true);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'q'))
        {
            self.quit = true;
            self.inference.profiles.panel = Some(panel);
            return Ok(true);
        }
        if self.inference.profiles.job.is_some() {
            self.inference.profiles.panel = Some(panel);
            return Ok(true);
        }
        if key.code == KeyCode::Esc {
            if panel.name.take().is_some() || panel.delete_confirm {
                panel.delete_confirm = false;
                self.inference.profiles.panel = Some(panel);
            }
            return Ok(true);
        }
        if panel.delete_confirm && key.code != KeyCode::Char('y') {
            self.inference.profiles.panel = Some(panel);
            return Ok(true);
        }
        if panel.name.is_some()
            && key.code == KeyCode::Char('r')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            let (_, workspace) = self.account_destination(panel.destination)?;
            self.inference.profiles.panel = Some(panel);
            if let Err(error) = self.request_profiles(VesselCommand::Profiles { workspace }) {
                self.status = safe(&error.to_string());
            }
            return Ok(true);
        }
        if let Some((id, name, seed)) = panel.name.as_mut() {
            match key.code {
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && name.len() + c.len_utf8() <= 80
                        && !c.is_control() =>
                {
                    name.push(c)
                }
                KeyCode::Backspace => {
                    name.pop();
                }
                KeyCode::Enter if !name.trim().is_empty() => {
                    let (id, name, seed) = (*id, name.trim().to_owned(), seed.clone());
                    let destination = panel.destination;
                    panel.name = None;
                    self.inference.profiles.panel = Some(panel);
                    if let Err(e) = self.edit_profile(destination, id, name, seed) {
                        self.inference.profiles.editing = None;
                        self.status = safe(&e.to_string());
                    }
                    return Ok(true);
                }
                _ => {}
            }
            self.inference.profiles.panel = Some(panel);
            return Ok(true);
        }
        let profile = panel
            .catalogue
            .as_ref()
            .and_then(|c| c.profiles.get(panel.selected))
            .cloned();
        let manage = panel.catalogue.as_ref().is_some_and(|c| c.can_manage);
        let mut command = None;
        let mut select = false;
        match key.code {
            KeyCode::Up => panel.selected = panel.selected.saturating_sub(1),
            KeyCode::Down => {
                panel.selected = (panel.selected + 1).min(
                    panel
                        .catalogue
                        .as_ref()
                        .map(|c| c.profiles.len().saturating_sub(1))
                        .unwrap_or(0),
                )
            }
            KeyCode::Enter if !panel.delete_confirm => select = true,
            KeyCode::Char('n') if manage => {
                panel.name = Some((Uuid::new_v4(), String::new(), None))
            }
            KeyCode::Char('e' | 'd') if manage => {
                if let Some(p) = profile.clone() {
                    panel.name = Some((
                        if key.code == KeyCode::Char('d') {
                            Uuid::new_v4()
                        } else {
                            p.id
                        },
                        if key.code == KeyCode::Char('d') {
                            copy_name(&p.name)
                        } else {
                            p.name.clone()
                        },
                        Some(p),
                    ));
                }
            }
            KeyCode::Char('x') if manage && profile.is_some() => {
                panel.delete_confirm = true;
                panel.notice = "Delete this profile? Press y to confirm or Esc to cancel. Existing voyages keep their settings.".into();
            }
            KeyCode::Char('y') if manage && panel.delete_confirm => {
                if let Some(p) = &profile {
                    let (_, workspace) = self.account_destination(panel.destination)?;
                    command = Some(VesselCommand::DeleteProfile {
                        command_id: Uuid::new_v4(),
                        workspace,
                        expected_revision: panel.catalogue.as_ref().unwrap().revision,
                        profile_id: p.id,
                    });
                    panel.delete_confirm = false;
                }
            }
            KeyCode::Char('f') if manage => {
                if let Some(p) = &profile {
                    let (_, workspace) = self.account_destination(panel.destination)?;
                    command = Some(VesselCommand::SetDefaultProfile {
                        command_id: Uuid::new_v4(),
                        workspace,
                        expected_revision: panel.catalogue.as_ref().unwrap().revision,
                        profile_id: p.id,
                    });
                }
            }
            KeyCode::Char('r') => {
                let (_, workspace) = self.account_destination(panel.destination)?;
                command = Some(VesselCommand::Profiles { workspace });
            }
            _ => {}
        }
        let destination = panel.destination;
        self.inference.profiles.panel = Some(panel);
        let result = if select {
            profile
                .map(|p| self.use_profile(destination, p))
                .unwrap_or(Ok(()))
        } else if let Some(command) = command {
            self.request_profiles(command)
        } else {
            Ok(())
        };
        if let Err(e) = result {
            if let Some(panel) = self.inference.profiles.panel.as_mut() {
                panel.notice = safe(&e.to_string());
            }
        }
        Ok(true)
    }
    pub(super) fn draw_profiles(&self, frame: &mut Frame<'_>) -> bool {
        let Some(panel) = self.inference.profiles.panel.as_ref() else {
            return false;
        };
        if self.inference.profiles.editing.is_some() {
            return false;
        }
        let screen = frame.area();
        let width = screen.width.saturating_sub(2).min(100);
        let height = screen.height.saturating_sub(2).min(26);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Execution profiles ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.inference.profiles.hits.borrow_mut().clear();
        self.inference.profiles.rows.borrow_mut().clear();
        let mut lines = vec![];
        if let Some((_, name, _)) = &panel.name {
            lines.push(format!("Profile name: {name}▏"));
            lines.push("Enter: edit execution settings · Ctrl+R: reload · Esc: cancel".into());
            lines.push(panel.notice.clone());
        } else {
            if let Some(c) = &panel.catalogue {
                let available = inner.height.saturating_sub(7) as usize;
                let start = panel.selected.saturating_sub(available.saturating_sub(1));
                for (index, p) in c.profiles.iter().enumerate().skip(start).take(available) {
                    self.inference.profiles.rows.borrow_mut().push((
                        Rect::new(inner.x, inner.y + lines.len() as u16, inner.width, 1),
                        index,
                    ));
                    lines.push(format!(
                        "{} {}{} · {}",
                        if panel.selected == index { "›" } else { " " },
                        safe(&p.name),
                        if c.default_profile_id == Some(p.id) {
                            " (default)"
                        } else {
                            ""
                        },
                        safe(&p.model)
                    ));
                }
                if let Some(p) = c.profiles.get(panel.selected) {
                    lines.push(format!(
                        "Thinking: {} · Service tier: {}",
                        safe(p.reasoning_effort.as_deref().unwrap_or("inherit")),
                        safe(p.service_tier.as_deref().unwrap_or("inherit"))
                    ));
                    lines.push(format!("Account: {}", self.profile_account_label(p)));
                }
                lines.push("↑/↓ select · Enter use profile · r reload · Esc close".into());
                if c.can_manage {
                    lines
                        .push("n create · e edit · d duplicate · x delete · f make default".into());
                }
            }
            lines.push(if self.inference.profiles.job.is_some() {
                "Saving / loading profiles…".into()
            } else {
                panel.notice.clone()
            });
        }
        frame.render_widget(Paragraph::new(lines.join("\n")), inner);
        if inner.height > 3 && panel.name.is_none() {
            let mut x = inner.x;
            let y = area.bottom().saturating_sub(2);
            let manage = panel.catalogue.as_ref().is_some_and(|c| c.can_manage);
            let controls = [
                ("Use", KeyCode::Enter),
                ("New", KeyCode::Char('n')),
                ("Edit", KeyCode::Char('e')),
                ("Copy", KeyCode::Char('d')),
                ("Delete", KeyCode::Char('x')),
                ("Default", KeyCode::Char('f')),
                ("Reload", KeyCode::Char('r')),
                ("Close", KeyCode::Esc),
            ];
            for (label, key) in controls {
                if !manage && matches!(key, KeyCode::Char('n' | 'e' | 'd' | 'x' | 'f')) {
                    continue;
                }
                let width = label.len() as u16 + 3;
                if x + width > inner.right() {
                    break;
                }
                let rect = Rect::new(x, y, width, 1);
                frame.render_widget(
                    Paragraph::new(format!("[{label}] ")).style(crate::theme::Role::Focus.style()),
                    rect,
                );
                self.inference.profiles.hits.borrow_mut().push((rect, key));
                x += width;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_client::ui::account_test_support::Fixture;
    fn setup(manage: bool) -> (Fixture, App, Target, ExecutionProfile) {
        let (fixture, mut app, target) = crate::process_client::ui::coverage_support::app();
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .inference = Some(Settings {
            model: "current".into(),
            ..Default::default()
        });
        let profile = ExecutionProfile {
            id: Uuid::new_v4(),
            name: "Everyday".into(),
            model: "example-model".into(),
            account: voyage_protocol::accounts::AccountBinding {
                account_id: Uuid::new_v4(),
                connection_id: Uuid::new_v4(),
                identity_generation: 1,
                connection_revision: 1,
                transport: voyage_protocol::accounts::Transport::OpenaiResponses,
            },
            reasoning_effort: Some("high".into()),
            service_tier: None,
        };
        app.inference.profiles.panel = Some(Panel {
            destination: Destination::Live(target),
            catalogue: Some(ProfileCatalogue {
                revision: 1,
                profiles: vec![profile.clone()],
                default_profile_id: Some(profile.id),
                can_manage: manage,
            }),
            selected: 0,
            name: None,
            delete_confirm: false,
            notice: String::new(),
        });
        (fixture, app, target, profile)
    }
    fn key(app: &mut App, code: KeyCode) {
        assert!(
            app.profiles_input(&Event::Key(crossterm::event::KeyEvent::new(
                code,
                KeyModifiers::NONE
            )))
            .unwrap()
        );
    }
    #[test]
    fn settings_are_copied_and_labels_require_all_four_values() {
        let (_fixture, mut app, _, mut profile) = setup(true);
        let saved = settings(&profile);
        app.inference.profiles.labels = vec![profile.clone()];
        assert!(app.profile_label(&saved).starts_with("Everyday"));
        profile.model = "edited".into();
        app.inference.profiles.labels = vec![profile];
        assert_eq!(saved.model, "example-model");
        assert!(app.profile_label(&saved).starts_with("Saved settings"));
    }
    #[test]
    fn duplicate_and_delete_confirmation_leave_voyage_untouched() {
        let (_fixture, mut app, target, profile) = setup(true);
        key(&mut app, KeyCode::Char('d'));
        let name = app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .as_ref()
            .unwrap();
        assert_ne!(name.0, profile.id);
        assert_eq!(name.1, "Everyday copy");
        key(&mut app, KeyCode::Esc);
        let mut second = profile.clone();
        second.id = Uuid::new_v4();
        second.name = "Another".into();
        app.inference
            .profiles
            .panel
            .as_mut()
            .unwrap()
            .catalogue
            .as_mut()
            .unwrap()
            .profiles
            .push(second);
        key(&mut app, KeyCode::Char('x'));
        key(&mut app, KeyCode::Down);
        assert_eq!(app.inference.profiles.panel.as_ref().unwrap().selected, 0);
        assert!(
            app.inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .delete_confirm
        );
        key(&mut app, KeyCode::Esc);
        assert!(
            !app.inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .delete_confirm
        );
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert_eq!(
            app.inference_settings(Destination::Live(target))
                .unwrap()
                .model,
            "current"
        );
    }
    #[test]
    fn duplicate_name_is_bounded_at_a_unicode_boundary() {
        let name = copy_name(&"é".repeat(40));
        assert!(name.len() <= 80);
        assert!(name.ends_with(" copy"));
    }
    #[test]
    fn read_only_catalogue_cannot_enter_management() {
        let (_fixture, mut app, _, _) = setup(false);
        for code in ['n', 'e', 'd', 'x', 'f', 'y'] {
            key(&mut app, KeyCode::Char(code));
        }
        let panel = app.inference.profiles.panel.as_ref().unwrap();
        assert!(panel.name.is_none());
        assert!(!panel.delete_confirm);
        assert!(app.inference.profiles.job.is_none());
    }
    #[test]
    fn profile_account_summary_uses_safe_names_and_explicit_unavailability() {
        let (_fixture, mut app, _, profile) = setup(true);
        assert!(
            app.profile_account_label(&profile)
                .starts_with("Unavailable account")
        );
        app.inference
            .profiles
            .accounts
            .push(account_choices::Choice {
                label: "Work account".into(),
                binding: profile.account.clone(),
                provider: "openai-responses".into(),
                ready: true,
            });
        assert_eq!(app.profile_account_label(&profile), "Work account");
        app.inference.profiles.accounts[0].ready = false;
        assert_eq!(
            app.profile_account_label(&profile),
            "Work account (unavailable)"
        );
        app.inference.profiles.accounts[0]
            .binding
            .identity_generation += 1;
        assert!(
            app.profile_account_label(&profile)
                .starts_with("Unavailable account")
        );
        assert_eq!(profile.account.identity_generation, 1);
    }
    #[test]
    fn failed_save_retains_profile_values_for_repair() {
        let (_fixture, mut app, _, profile) = setup(true);
        app.inference.profiles.pending_save = Some(profile.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.inference.profiles.job = Some(rx);
        assert!(
            tx.send(Err(anyhow::anyhow!("Profile revision changed")))
                .is_ok()
        );
        app.poll_profiles();
        let panel = app.inference.profiles.panel.as_ref().unwrap();
        assert_eq!(panel.name.as_ref().unwrap().2.as_ref(), Some(&profile));
        assert!(panel.notice.contains("Settings retained"));
    }
    #[test]
    fn changed_destination_discards_profile_response() {
        let (_fixture, mut app, _, _) = setup(true);
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.inference.profiles.job = Some(rx);
        assert!(
            tx.send(Ok((Uuid::new_v4(), ProfileCatalogue::default(), vec![])))
                .is_ok()
        );
        app.selected = None;
        app.poll_profiles();
        assert!(app.inference.profiles.panel.is_none());
        assert!(app.inference.profiles.labels.is_empty());
    }
}
