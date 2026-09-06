//! Capture identity before dispatch and retain uncertain drafts across reconnects.
use super::{
    App, drafts,
    observe::Update,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::process::{RuntimeCommand, VesselCommand};

impl App {
    pub fn create(&mut self, workspace: Option<&str>) -> Result<()> {
        let route = self.selected.map_or(0, |target| target.route);
        let client = self.clients[route].clone();
        let workspace = match workspace {
            Some(path) => std::path::PathBuf::from(path),
            None if client.ssh.is_none() => std::env::current_dir()?,
            None => anyhow::bail!("remote creation needs /new /absolute/workspace"),
        };
        ensure!(
            workspace.is_absolute(),
            "workspace must be absolute on the executing host"
        );
        let session_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();
        self.status = format!(
            "Starting {session_id}; if delivery is lost, inspect this identity before retrying"
        );
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let result = client
                .request(VesselCommand::Start {
                    command_id,
                    session_id,
                    workspace,
                })
                .await
                .and_then(|value| Ok(serde_json::from_value(value)?))
                .map_err(|error| format!("Create {session_id}: {error}"));
            let _ = sender.send(Update::Created { route, result }).await;
        });
        Ok(())
    }

    pub fn send(&mut self) -> Result<()> {
        let target = self.selected.context("create or select a voyage first")?;
        let draft = self.views[&target].draft.text.clone();
        let command_text = draft.trim();
        if command_text == "/quit" {
            self.quit = true;
            return Ok(());
        }
        if command_text == "/help" {
            self.status = "Tab switches · Ctrl+N creates · /new [absolute-workspace] · /use UUID · /rename NAME · /model NAME · /cancel · /approve UUID · /deny UUID · /answer UUID text · /receipt · /quit".into();
            return Ok(());
        }
        if command_text == "/new" || command_text.starts_with("/new ") {
            return self.create(command_text.strip_prefix("/new "));
        }
        if let Some(id) = command_text.strip_prefix("/use ") {
            let session: Uuid = id.parse()?;
            let key = self
                .views
                .keys()
                .find(|key| key.session == session)
                .copied()
                .context("voyage not in permitted catalogue")?;
            self.selected = Some(key);
            return Ok(());
        }
        let view = self.views.get_mut(&target).expect("selected view");
        if command_text == "/receipt" {
            let pending = view
                .pending
                .as_ref()
                .context("no unresolved command")?
                .clone();
            self.dispatch(
                target,
                pending.command_id,
                RuntimeCommand::Receipt {
                    command_id: pending.command_id,
                },
            );
            return Ok(());
        }
        ensure!(
            view.pending.is_none(),
            "resolve the pending command with /receipt before sending again; draft preserved"
        );
        ensure!(!command_text.is_empty(), "input is empty");
        ensure!(
            draft.len() <= 64 * 1024,
            "input limit is 64 KiB; draft preserved"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("waiting for an authenticated snapshot")?;
        let expected_revision = snapshot.revision;
        let command_id = Uuid::new_v4();
        let expires_at_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        )?
        .saturating_add(60_000);
        let command = if command_text.starts_with("/approve ")
            || command_text.starts_with("/deny ")
            || command_text.starts_with("/answer ")
        {
            let (operation, arguments) = command_text.split_once(' ').expect("decision arguments");
            let (id, answer) = arguments.split_once(' ').unwrap_or((arguments, ""));
            let decision_id: Uuid = id.parse()?;
            let decision = snapshot
                .decisions
                .iter()
                .find(|decision| decision.decision_id == decision_id)
                .context("decision not pending in this voyage")?;
            ensure!(
                decision.incarnation == view.process.incarnation,
                "decision belongs to an earlier runtime"
            );
            let kind = decision.request.get("kind").and_then(|kind| kind.as_str());
            let response = match operation {
                "/approve" | "/deny" => {
                    ensure!(kind == Some("approval"), "this decision is a question");
                    serde_json::Value::String(
                        if operation == "/approve" {
                            "approved"
                        } else {
                            "denied"
                        }
                        .into(),
                    )
                }
                "/answer" => {
                    ensure!(
                        kind == Some("question") && !answer.is_empty(),
                        "provide an answer to a pending question"
                    );
                    serde_json::json!({"status":"custom", "answer":answer})
                }
                _ => unreachable!(),
            };
            RuntimeCommand::Respond {
                command_id,
                expected_revision,
                expires_at_ms: expires_at_ms.min(decision.expires_at_ms),
                run_id: decision.run_id,
                decision_id,
                response,
            }
        } else if let Some(name) = command_text.strip_prefix("/rename ") {
            RuntimeCommand::Rename {
                command_id,
                expected_revision,
                expires_at_ms,
                name: name.into(),
            }
        } else if let Some(model) = command_text.strip_prefix("/model ") {
            RuntimeCommand::SetModel {
                command_id,
                expected_revision,
                expires_at_ms,
                model: model.into(),
            }
        } else if command_text == "/cancel" {
            let run = snapshot
                .run
                .as_ref()
                .filter(|run| run.active())
                .context("no observed active run to cancel")?;
            RuntimeCommand::Cancel {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: run.run_id,
            }
        } else if command_text.starts_with('/') {
            anyhow::bail!("unknown command; /help lists connected controls");
        } else if let Some(run) = snapshot.run.as_ref().filter(|run| run.active()) {
            RuntimeCommand::Steer {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: run.run_id,
                prompt: draft.clone(),
            }
        } else {
            RuntimeCommand::Submit {
                command_id,
                expected_revision,
                expires_at_ms,
                prompt: draft.clone(),
            }
        };
        view.pending = Some(Pending {
            command_id,
            incarnation: view.process.incarnation,
            draft,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist command identity; nothing sent"));
        }
        self.dispatch(target, command_id, command);
        Ok(())
    }

    fn dispatch(&mut self, target: Target, command_id: Uuid, command: RuntimeCommand) {
        let incarnation = self.views[&target].process.incarnation;
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        self.status = format!("Sending command {command_id}");
        tokio::spawn(async move {
            let result = client.forward(target.session, incarnation, command).await;
            let _ = sender
                .send(Update::Command {
                    target,
                    command_id,
                    result: result.map_err(|error| error.to_string()),
                })
                .await;
        });
    }
}
