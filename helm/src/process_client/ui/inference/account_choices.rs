//! Account metadata selection is part of the chooser; no secrets or mutations.
use super::*;
use serde::Deserialize;
use voyage_protocol::accounts::{
    AccountBinding, AccountDescriptor, AccountState, ConnectionDescriptor, CredentialAvailability,
};
#[derive(Clone)]
pub(super) struct Choice {
    pub label: String,
    pub binding: AccountBinding,
    pub provider: String,
    pub ready: bool,
}
#[derive(Deserialize)]
pub(super) struct Catalogue {
    accounts: Vec<AccountDescriptor>,
    connections: Vec<ConnectionDescriptor>,
    #[serde(default)]
    pub default_account: Option<AccountBinding>,
}
pub(super) struct Load {
    pub destination: Destination,
    pub incarnation: Option<Uuid>,
    pub receiver: tokio::sync::oneshot::Receiver<Result<(Uuid, Catalogue, Option<Settings>)>>,
    pub task: tokio::task::JoinHandle<()>,
}
impl Catalogue {
    fn choices(&self) -> Result<Vec<Choice>> {
        ensure!(
            self.accounts.len() <= 128 && self.connections.len() <= 64,
            "Account list exceeds limits"
        );
        let mut out = Vec::new();
        for a in &self.accounts {
            if let Some(c) = self.connections.iter().find(|c| c.id == a.connection_id) {
                for t in &c.transports {
                    let binding = AccountBinding {
                        account_id: a.id,
                        connection_id: c.id,
                        identity_generation: a.identity_generation,
                        connection_revision: c.revision,
                        transport: *t,
                    };
                    out.push(Choice {
                        label: format!(
                            "{}{}",
                            safe(&a.label),
                            if self.default_account.as_ref() == Some(&binding) {
                                " · Default"
                            } else {
                                ""
                            }
                        ),
                        binding,
                        provider: match t {
                            voyage_protocol::accounts::Transport::ChatgptOauth => "chatgpt-oauth",
                            voyage_protocol::accounts::Transport::OpenaiResponses => {
                                "openai-responses"
                            }
                            voyage_protocol::accounts::Transport::OpenaiChat => "openai-chat",
                            voyage_protocol::accounts::Transport::Anthropic => "anthropic",
                        }
                        .into(),
                        ready: a.state == AccountState::Ready
                            && a.availability == CredentialAvailability::Available,
                    });
                }
            }
        }
        Ok(out)
    }
}
impl App {
    pub(in crate::process_client::ui) fn cancel_chooser_accounts(&mut self) {
        if let Some(load) = self.inference.account_load.take() {
            load.task.abort();
            self.retired_observers.push(load.task);
        }
    }
    pub(in crate::process_client::ui) fn load_chooser_accounts(&mut self) -> Result<()> {
        self.cancel_chooser_accounts();
        let p = self
            .inference
            .picker
            .as_mut()
            .context("Choose a model first")?;
        p.chooser.accounts_open = true;
        p.chooser.accounts_loading = true;
        p.chooser.account_row = 0;
        p.chooser.focus = chooser::Control::Account;
        let destination = p.destination;
        let incarnation = p.incarnation;
        let (route, workspace) = self.account_destination(destination)?;
        let client = self.clients[route].clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let caps = client
                    .request(voyage_protocol::vessel::VesselCommand::Capabilities)
                    .await?;
                let host: Uuid = serde_json::from_value(caps["vessel_id"].clone())?;
                ensure!(
                    !host.is_nil() && client.managed().is_none_or(|c| c.vessel_id == host),
                    "Account host changed"
                );
                let value = client
                    .request(voyage_protocol::vessel::VesselCommand::Accounts {
                        workspace: workspace.clone(),
                        transport: None,
                    })
                    .await?;
                let catalogue = serde_json::from_value::<Catalogue>(value)?;
                let defaults = if catalogue.default_account.is_some() {
                    Some(serde_json::from_value::<Settings>(
                        client
                            .request(voyage_protocol::vessel::VesselCommand::AccountDefaults {
                                workspace: workspace.clone(),
                            })
                            .await?,
                    )?)
                } else {
                    None
                };
                Ok::<_, anyhow::Error>((host, catalogue, defaults))
            })
            .await
            .map_err(|_| anyhow::anyhow!("Account list timed out"))
            .and_then(|r| {
                r.map_err(|_| {
                    anyhow::anyhow!("Account list unavailable; reconnect or review access")
                })
            });
            let _ = tx.send(result);
        });
        self.inference.account_load = Some(Load {
            destination,
            incarnation,
            receiver: rx,
            task,
        });
        Ok(())
    }
    pub(super) fn poll_chooser_accounts(&mut self) {
        let Some(mut load) = self.inference.account_load.take() else {
            return;
        };
        let current = self.inference.picker.as_ref().is_some_and(|p| {
            p.destination == load.destination
                && p.incarnation == load.incarnation
                && match load.destination {
                    Destination::Live(t) => {
                        self.selected == Some(t)
                            && self.active_draft.is_none()
                            && self
                                .views
                                .get(&t)
                                .is_some_and(|v| Some(v.process.incarnation) == load.incarnation)
                    }
                    Destination::Draft(d) => {
                        self.active_draft == Some(d) && self.new_drafts.contains_key(&d)
                    }
                }
                && p.chooser.accounts_open
        });
        if !current {
            load.task.abort();
            self.retired_observers.push(load.task);
            return;
        }
        let result = match load.receiver.try_recv() {
            Ok(r) => r,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.inference.account_load = Some(load);
                return;
            }
            Err(_) => Err(anyhow::anyhow!("Account list interrupted")),
        };
        self.retired_observers.push(load.task);
        self.inference.chooser_hits.borrow_mut().clear();
        if let Ok((host, _, _)) = &result {
            let route = match load.destination {
                Destination::Live(t) => t.route,
                Destination::Draft(d) => self.new_drafts[&d].route,
            };
            self.cache_account_host(route, *host);
        }
        let parsed = result.and_then(|(host, c, defaults)| {
            Ok((host, c.default_account.clone(), c.choices()?, defaults))
        });
        let mut start_models = false;
        let automatic = self.inference.picker.as_ref().unwrap().chooser.automatic;
        match parsed {
            Ok((host, default, choices, defaults)) => {
                let destination = load.destination;
                let initializing = self.inference.picker.as_ref().unwrap().chooser.initializing;
                if initializing {
                    let seed = self.inference.picker.as_ref().unwrap().original.clone();
                    let mut resolved = seed.clone();
                    if resolved.account.is_none()
                        && let Some(defaults) = defaults
                        && (resolved.provider.is_empty() || resolved.provider == defaults.provider)
                    {
                        resolved.account = defaults.account;
                        resolved.provider = defaults.provider;
                        if resolved.model.is_empty() {
                            resolved.model = defaults.model;
                        }
                    }
                    if let Destination::Draft(d) = destination
                        && let Err(e) = self.set_draft_account(d, host, resolved.clone())
                    {
                        let p = self.inference.picker.as_mut().unwrap();
                        p.chooser.accounts_loading = false;
                        p.notice = safe(&e.to_string());
                        return;
                    }
                    let p = self.inference.picker.as_mut().unwrap();
                    p.original = resolved.clone();
                    p.chooser = chooser::Draft::new(&resolved);
                    p.chooser.accounts_open = true;
                    if let Some(c) = choices
                        .iter()
                        .find(|c| c.ready && Some(&c.binding) == resolved.account.as_ref())
                    {
                        p.chooser.account_label = c.label.clone();
                        p.chooser.accounts_open = false;
                        start_models = true;
                    }
                }
                let p = self.inference.picker.as_mut().unwrap();
                p.chooser.initializing = false;
                p.chooser.accounts_loading = false;
                p.chooser.requires_default = default.is_none();
                p.chooser.account_row = choices
                    .iter()
                    .position(|c| Some(&c.binding) == p.chooser.account.as_ref())
                    .unwrap_or(0);
                p.chooser.accounts = choices;
                p.notice = if p.chooser.accounts.is_empty() {
                    "No accounts available. Sign in or add an account."
                } else {
                    "Choose an account; no message is sent."
                }
                .into();
            }
            Err(e) => {
                let p = self.inference.picker.as_mut().unwrap();
                p.chooser.accounts_loading = false;
                p.notice = safe(&e.to_string());
            }
        }
        if start_models && automatic {
            self.inference.picker = None;
            return;
        }
        if start_models
            && let Err(e) = self.load_inference_models()
            && let Some(p) = self.inference.picker.as_mut()
        {
            p.loading = false;
            p.notice = safe(&e.to_string());
        }
    }
    pub(super) fn choose_inline_account(&mut self, mut p: Picker, index: usize) -> Result<()> {
        let choice = p
            .chooser
            .accounts
            .get(index)
            .context("Account row changed")?
            .clone();
        ensure!(choice.ready, "This account needs sign-in; nothing selected");
        self.cancel_model_catalog();
        p.chooser.account = Some(choice.binding.clone());
        p.chooser.provider = choice.provider;
        p.chooser.account_label = choice.label;
        p.chooser.accounts_open = false;
        // Keep explicit overrides as candidates; changing account requires the
        // same inline Reset/Keep review as changing model, never silent erasure.
        p.chooser.review = false;
        p.chooser.keep = false;
        p.query.clear();
        p.models.clear();
        p.options.clear();
        p.selected = 0;
        p.notice="Account selected for review only. Choose its model, then Use model. Current work is unchanged.".into();
        self.inference.picker = Some(p);
        self.load_inference_models()
    }
}

impl App {
    pub(super) fn draw_inline_accounts(
        &self,
        frame: &mut ratatui::Frame<'_>,
        area: ratatui::layout::Rect,
        p: &Picker,
    ) {
        use super::chooser::Control;
        use ratatui::{layout::Rect, widgets::Paragraph};
        let controls = &self.inference.chooser_hits;
        let back = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget(Paragraph::new("[Back to models] · Select account"), back);
        controls.borrow_mut().push((back, Control::Account));
        let n = area.height.saturating_sub(5) as usize;
        let first = p.chooser.account_row.saturating_sub(n.saturating_sub(1));
        if p.chooser.accounts_loading {
            frame.render_widget(
                Paragraph::new("Loading accounts…"),
                Rect::new(area.x, area.y + 1, area.width, 1),
            );
        }
        if !p.chooser.accounts_loading && p.chooser.accounts.is_empty() {
            frame.render_widget(
                Paragraph::new(if p.notice.is_empty() {
                    "No available accounts. Sign in or add one."
                } else {
                    &p.notice
                })
                .wrap(ratatui::widgets::Wrap { trim: false }),
                Rect::new(area.x, area.y + 1, area.width, n as u16),
            );
        }
        for (i, c) in p.chooser.accounts.iter().enumerate().skip(first).take(n) {
            let r = Rect::new(area.x, area.y + 1 + (i - first) as u16, area.width, 1);
            let label = format!(
                "{} {}{}",
                if Some(&c.binding) == p.chooser.account.as_ref() {
                    "●"
                } else {
                    " "
                },
                c.label,
                if c.ready { "" } else { " · sign-in required" }
            );
            frame.render_widget(
                Paragraph::new(label).style(if i == p.chooser.account_row {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Primary.style()
                }),
                r,
            );
            controls.borrow_mut().push((r, Control::AccountRow(i)));
        }
        let sign = Rect::new(area.x, area.bottom() - 3, area.width, 1);
        frame.render_widget(
            Paragraph::new("[Sign in / add account]").style(crate::theme::Role::Focus.style()),
            sign,
        );
        controls.borrow_mut().push((sign, Control::SignIn));
        frame.render_widget(
            Paragraph::new("Selection is a draft. Use model applies it."),
            Rect::new(area.x, area.bottom() - 2, area.width, 1),
        );
        let retry = Rect::new(area.right().saturating_sub(10), area.bottom() - 1, 10, 1);
        frame.render_widget(
            Paragraph::new("[Retry]").style(crate::theme::Role::Focus.style()),
            retry,
        );
        controls.borrow_mut().push((retry, Control::Retry));
        let cancel = Rect::new(area.x, area.bottom() - 1, 10, 1);
        frame.render_widget(
            Paragraph::new("[Cancel]").style(crate::theme::Role::Focus.style()),
            cancel,
        );
        controls.borrow_mut().push((cancel, Control::Cancel));
    }
}

impl App {
    pub(in crate::process_client::ui) fn stage_account_from_private_list(
        &mut self,
        binding: AccountBinding,
        label: String,
        original: Settings,
        destination: Destination,
    ) -> Result<()> {
        self.inference_command(destination, "/model", true)?;
        let mut p = self
            .inference
            .picker
            .take()
            .context("Chooser unavailable")?;
        p.original = original;
        p.chooser.accounts = vec![Choice {
            label,
            binding: binding.clone(),
            provider: match binding.transport {
                voyage_protocol::accounts::Transport::ChatgptOauth => "chatgpt-oauth",
                voyage_protocol::accounts::Transport::OpenaiResponses => "openai-responses",
                voyage_protocol::accounts::Transport::OpenaiChat => "openai-chat",
                voyage_protocol::accounts::Transport::Anthropic => "anthropic",
            }
            .into(),
            ready: true,
        }];
        self.choose_inline_account(p, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_accounts_are_not_selectable_and_labels_have_no_control_text() {
        let account = Uuid::new_v4();
        let conn = Uuid::new_v4();
        let c:Catalogue=serde_json::from_value(serde_json::json!({"accounts":[{"id":account,"connection_id":conn,"alias":"x","label":"x\u{001b}[31m","metadata_revision":1,"identity_generation":1,"credential_revision":1,"capability_revision":1,"availability":"missing","state":"sign_in_required"}],"connections":[{"id":conn,"revision":1,"label":"API","endpoint":"https://api.openai.com/v1","transports":["openai_responses"]}]})).unwrap();
        let choices = c.choices().unwrap();
        assert_eq!(choices.len(), 1);
        assert!(!choices[0].ready);
        assert!(!choices[0].label.contains('\u{1b}'));
    }
}
