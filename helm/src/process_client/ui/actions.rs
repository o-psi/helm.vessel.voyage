//! Capture identity before dispatch and retain uncertain drafts across reconnects.
use super::{
    App, drafts,
    observe::Update,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

impl App {
    pub fn send(&mut self) -> Result<()> {
        let original = self.selected.and_then(|target| {
            self.views
                .get(&target)
                .map(|view| (target, view.draft.text.clone()))
        });
        self.send_command()?;
        if let Some((target, draft)) = original
            && super::inference::parse(draft.trim()).is_none()
            && draft.trim_start().starts_with('/')
            && let Some(view) = self.views.get_mut(&target)
            && view.pending.is_none()
            && view.draft.text == draft
        {
            view.history.record(&draft);
            view.draft.take();
            drafts::save(&self.clients[target.route], view)
                .context("The command ran, but the cleared draft could not be saved")?;
        }
        Ok(())
    }

    fn send_command(&mut self) -> Result<()> {
        let target = self.selected.context("create or select a voyage first")?;
        let draft = self
            .views
            .get(&target)
            .context("waiting for selected voyage catalogue")?
            .draft
            .text
            .clone();
        self.command_for(target, draft, false)
    }

    pub(super) fn command_for(
        &mut self,
        target: Target,
        draft: String,
        preserve_draft: bool,
    ) -> Result<()> {
        let command_text = draft.trim();
        if super::inference::parse(command_text).is_some() {
            return self.inference_command(
                super::inference::Destination::Live(target),
                command_text,
                preserve_draft,
            );
        }
        if command_text == "/access" || command_text.starts_with("/access ") {
            return self.open_access(target, command_text.strip_prefix("/access "));
        }
        if command_text == "/archived" || command_text == "/voyages" {
            self.show_archives(command_text == "/archived");
            return Ok(());
        }
        if command_text == "/restore" && self.views[&target].process.archive.is_some() {
            return self.restore_archive(target, preserve_draft);
        }
        if command_text == "/quit" {
            self.quit = true;
            return Ok(());
        }
        if command_text == "/help" {
            self.help = true;
            self.help_scroll = 0;
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
                .filter(|key| key.session == session)
                .copied()
                .collect::<Vec<_>>();
            ensure!(
                key.len() == 1,
                "voyage UUID is absent or ambiguous across routes; use Tab to select"
            );
            let key = key[0];
            self.selected = Some(key);
            return Ok(());
        }
        if let Some(path) = command_text.strip_prefix("/export ") {
            return self.export(target, path);
        }
        if let Some(id) = command_text.strip_prefix("/terminal ") {
            let terminal_id = id.parse()?;
            self.request_terminal(target, terminal_id)?;
            return Ok(());
        }
        if command_text == "/branch" || command_text.starts_with("/branch ") {
            return self.branch(
                target,
                command_text.strip_prefix("/branch ").map(str::to_owned),
                preserve_draft,
            );
        }
        if command_text == "/terminals" || command_text == "/terminal" {
            self.views
                .get_mut(&target)
                .expect("selected view")
                .terminals
                .open = true;
            return Ok(());
        }
        if matches!(
            command_text,
            "/tools"
                | "/policy"
                | "/todos"
                | "/subagents"
                | "/workflows"
                | "/models"
                | "/host_resources"
        ) {
            return self.inspect_control(target, &command_text[1..]);
        }
        if command_text == "/conversation" {
            self.views.get_mut(&target).expect("selected view").panel = None;
            return Ok(());
        }
        let view = self.views.get_mut(&target).expect("selected view");
        if command_text == "/receipt" {
            let pending = view
                .pending
                .as_ref()
                .context("no unresolved command")?
                .clone();
            self.dispatch(target, pending.command_id, pending.resolution());
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
        let command = if let Some(command) = super::controls::mutation(
            command_text,
            view,
            command_id,
            expected_revision,
            expires_at_ms,
        )? {
            command
        } else if command_text.starts_with("/approve ")
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
            VoyageCommand::Respond {
                command_id,
                expected_revision,
                expires_at_ms: expires_at_ms.min(decision.expires_at_ms),
                run_id: decision.run_id,
                decision_id,
                response,
            }
        } else if let Some(name) = command_text.strip_prefix("/rename ") {
            VoyageCommand::Rename {
                command_id,
                expected_revision,
                expires_at_ms,
                name: name.into(),
            }
        } else if command_text == "/cancel" {
            let run = snapshot
                .run
                .as_ref()
                .filter(|run| run.active())
                .context("no observed active run to cancel")?;
            VoyageCommand::Cancel {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: run.run_id,
            }
        } else if command_text.starts_with('/') {
            anyhow::bail!("unknown command; /help lists connected controls");
        } else if let Some(run) = snapshot.run.as_ref().filter(|run| run.active()) {
            VoyageCommand::Steer {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: run.run_id,
                prompt: draft.clone(),
            }
        } else {
            VoyageCommand::Submit {
                command_id,
                expected_revision,
                expires_at_ms,
                prompt: draft.clone(),
            }
        };
        view.pending = Some(Pending {
            command_id,
            original: Some(Box::new(command.clone())),
            receipt_only: false,
            incarnation: view.process.incarnation,
            draft,
            preserve_draft,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist command identity; nothing sent"));
        }
        if !view.draft.text.trim_start().starts_with('/') {
            let mut transcript = view.transcript.borrow_mut();
            transcript.delivery = Some(super::transcript::Delivery {
                text: view.draft.text.clone(),
                before: view.snapshot.as_ref().map_or(0, |s| s.total_messages),
                label: "Sending…".into(),
            });
            transcript.dirty = true;
        }
        self.dispatch(target, command_id, command);
        Ok(())
    }

    pub(super) fn dispatch(&mut self, target: Target, command_id: Uuid, command: VoyageCommand) {
        let incarnation = self.views[&target].process.incarnation;
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        self.status = "Sending...".into();
        let resolving = matches!(
            command,
            VoyageCommand::Resolve { .. } | VoyageCommand::Receipt { .. }
        );
        tokio::spawn(async move {
            let mut result = client
                .voyage(target.session, incarnation, command.clone())
                .await;
            // Bounded status recovery only. Never replay the submitted mutation.
            if resolving {
                for _ in 1..3 {
                    if result
                        .as_ref()
                        .is_ok_and(|value| value["status"] != "unknown")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                    result = client
                        .voyage(target.session, incarnation, command.clone())
                        .await;
                }
            }
            let _ = sender
                .send(Update::Command {
                    target,
                    command_id,
                    refused: !resolving
                        && result.as_ref().err().is_some_and(|error| {
                            error
                                .downcast_ref::<crate::process_client::transport::Refusal>()
                                .is_some()
                        }),
                    result: result.map_err(|error| {
                        if resolving {
                            format!("Status check unavailable after bounded attempts: {error}. Original command remains unresolved; F4 checks again")
                        } else {
                            error.to_string()
                        }
                    }),
                })
                .await;
        });
    }
}
