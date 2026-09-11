//! Bind operator forms to exact observations and the existing durable command path.
use super::{
    App,
    observe::Update,
    operator::{Observation, Outcome, Panel},
    state::Target,
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

pub(super) struct Loaded {
    pub observation: Observation,
    pub request: Uuid,
    pub filter: Option<&'static str>,
    pub result: std::result::Result<[Value; 3], String>,
}
impl App {
    fn operator_observation(&self, target: Target) -> Result<Observation> {
        ensure!(
            self.clients.available(target.route),
            "Vessel unavailable; Ctrl+G reconnects without replay"
        );
        let view = self.views.get(&target).context("Voyage unavailable")?;
        ensure!(
            !view.archived() && !view.deleted(),
            "Restore this voyage first"
        );
        ensure!(
            view.pending.is_none(),
            "Previous action pending; wait for automatic confirmation"
        );
        let snapshot = view.snapshot.as_ref().context("Waiting for voyage state")?;
        Ok(Observation {
            target,
            incarnation: view.process.incarnation,
            revision: snapshot.revision,
            run_id: snapshot
                .run
                .as_ref()
                .filter(|r| r.active())
                .map(|r| r.run_id),
        })
    }
    pub(super) fn open_operator(
        &mut self,
        target: Target,
        filter: Option<&'static str>,
    ) -> Result<()> {
        let observation = self.operator_observation(target)?;
        let request = Uuid::new_v4();
        self.cancel_paste_for_private_panel();
        self.operator = None;
        self.operator_loading = Some((
            request,
            std::time::Instant::now() + std::time::Duration::from_secs(65),
        ));
        self.explore = None;
        self.help = false;
        self.status = "Loading authorized tool forms… Esc cancels loading, not voyage work.".into();
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let job = tokio::spawn(async move {
            let result: Result<[Value; 3]> = async {
                let mut values = Vec::new();
                for section in ["tools", "todos", "subagents"] {
                    let value = client
                        .voyage(
                            target.session,
                            observation.incarnation,
                            VoyageCommand::Controls {
                                run_id: observation.run_id,
                                section: section.into(),
                            },
                        )
                        .await?;
                    ensure!(
                        value["section"] == section,
                        "Control response section changed"
                    );
                    if let Some(run) = observation.run_id {
                        ensure!(
                            value["run_id"] == run.to_string(),
                            "Control run changed; reopen forms"
                        );
                    }
                    values.push(value);
                }
                // Archive discovery is optional on older owners, never a reason
                // to invent records or block unrelated tools. First page is bounded.
                let archive = client
                    .voyage(
                        target.session,
                        observation.incarnation,
                        VoyageCommand::Controls {
                            run_id: observation.run_id,
                            section: "subagents_archive".into(),
                        },
                    )
                    .await;
                match archive {
                    Ok(archive) if archive["section"] == "subagents_archive" => {
                        if let Some(agents) = archive["value"]["agents"].as_array()
                            && let Some(retained) = values[2]["value"].as_array_mut()
                        {
                            retained.extend(agents.iter().cloned());
                        }
                        values[2]["incomplete"] =
                            serde_json::json!(!archive["value"]["next_after"].is_null());
                    }
                    _ => values[2]["archive_unavailable"] = serde_json::json!(true),
                }
                Ok(values.try_into().expect("three controls"))
            }
            .await;
            let _ = sender
                .send(Update::Operator(Box::new(Loaded {
                    observation,
                    request,
                    filter,
                    result: result.map_err(|e| e.to_string()),
                })))
                .await;
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(job);
        Ok(())
    }
    pub(super) fn operator_loaded(&mut self, loaded: Loaded) {
        if self.operator_loading.map(|pending| pending.0) != Some(loaded.request) {
            return;
        }
        self.operator_loading = None;
        let result = (|| -> Result<Panel> {
            ensure!(
                self.selected == Some(loaded.observation.target) && self.active_draft.is_none(),
                "Selected voyage changed; reopen forms"
            );
            ensure!(
                self.operator_observation(loaded.observation.target)? == loaded.observation,
                "Voyage changed while loading; reopen forms"
            );
            let [tools, tasks, agents] = loaded.result.map_err(anyhow::Error::msg)?;
            let mut panel = Panel::new(loaded.observation, &tools, &tasks, &agents, loaded.filter)?;
            panel.context = format!(
                "{} · {}",
                self.views[&loaded.observation.target].title(),
                self.route_label(loaded.observation.target.route)
            );
            Ok(panel)
        })();
        match result {
            Ok(panel) => self.operator = Some(panel),
            Err(error) => {
                self.status = format!("Operator forms unavailable: {error}. Draft retained.")
            }
        }
    }
    pub(super) fn poll_operator(&mut self) {
        if self
            .operator_loading
            .is_some_and(|(_, deadline)| std::time::Instant::now() >= deadline)
        {
            self.operator_loading = None;
            self.status =
                "Tool form loading timed out. Reopen F8 to retry observation; no tool was sent."
                    .into();
        }
    }
    pub(super) fn operator_input(&mut self, event: &Event) -> bool {
        if let Event::Key(key) = event {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                return false;
            }
            if self.operator_loading.is_some() {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Esc {
                    self.operator_loading = None;
                    self.status = "Tool form loading cancelled; voyage work continues.".into();
                }
                return true;
            }
        }
        if self.operator_loading.is_some() {
            return true;
        }
        let Some(mut panel) = self.operator.take() else {
            return false;
        };
        match panel.input(event) {
            Outcome::Close => {}
            Outcome::Stay => self.operator = Some(panel),
            Outcome::Submit(request) => {
                let result = (|| -> Result<()> {
                    ensure!(
                        self.selected == Some(request.observation.target)
                            && self.active_draft.is_none(),
                        "Selected voyage changed; reopen forms"
                    );
                    ensure!(
                        self.operator_observation(request.observation.target)?
                            == request.observation,
                        "Voyage changed; reopen forms to review current state"
                    );
                    self.command_for(request.observation.target, request.command_text()?, true)
                })();
                if let Err(error) = result {
                    panel.set_error(format!("{error}. Nothing resubmitted; draft retained."));
                    self.operator = Some(panel);
                }
            }
        }
        true
    }
}
