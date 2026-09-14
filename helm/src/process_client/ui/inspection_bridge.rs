//! Async reads for the inspection UI. No workspace I/O and no tool admission here.
use super::{inspection::Panel, operator::Observation};
use crate::process_client::{export, transport::Client};
use anyhow::{Context, Result, ensure};
use voyage_protocol::vessel::VoyageCommand;

/// Caller must retain its own loading request identity and reject stale selected
/// route/generation/incarnation/revision before displaying the returned panel.
pub(super) async fn load(
    client: &Client,
    observation: Observation,
    context: String,
) -> Result<Panel> {
    ensure!(
        client.id() == observation.target.route.id
            && client.generation() == observation.target.route.generation,
        "Inspection route changed; reopen on the executing host"
    );
    let tools = client
        .voyage(
            observation.target.session,
            observation.incarnation,
            VoyageCommand::Controls {
                run_id: observation.run_id,
                section: "tools".into(),
            },
        )
        .await?;
    ensure!(
        valid_inventory(&tools)?,
        "Executing-host tool inventory unavailable"
    );
    Ok(Panel::new(observation, context, tools))
}

fn valid_inventory(tools: &serde_json::Value) -> Result<bool> {
    Ok(tools["section"] == "tools"
        && super::inspection::inventory(tools).is_array()
        && serde_json::to_vec(tools)?.len() <= 1024 * 1024)
}

/// Read the selected complete assistant message. The caller emits the returned
/// OSC52 bytes only for the matching, still-open explicit human copy request.
/// Do not log the sequence or clipboard payload. Clipboard support is terminal
/// dependent; lack of acknowledgment is not a verified copy.
pub(super) async fn copy_response(
    client: &Client,
    observation: Observation,
    index: u64,
) -> Result<String> {
    ensure!(
        client.id() == observation.target.route.id
            && client.generation() == observation.target.route.generation,
        "Copy route changed; select the response again"
    );
    let text = export::response_text(
        client,
        observation.target.session,
        observation.incarnation,
        observation.revision,
        index,
    )
    .await?;
    export::response_clipboard_sequence(&text)
}

/// Default to the newest loaded completed assistant response, never live deltas,
/// tool text, interrupted provider output, or a visually selected terminal row.
/// `MessageChunk` fetches the full canonical text even for a bounded projection.
pub(super) fn response_index(snapshot: &super::state::Snapshot) -> Result<u64> {
    snapshot.messages.iter().rev()
        .find(|m| m.role == "assistant" && !m.interrupted() && !m.content.is_empty() && m.tool_calls.is_empty())
        .map(|m| m.message_index as u64)
        .context("No completed response in loaded history; load older history or select an assistant response")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_and_active_inventory_envelopes_are_validated_at_load_boundary() {
        let active = serde_json::json!({"section":"tools","value":[]});
        let idle = serde_json::json!({"section":"tools","value":{"inventory":[],"source":"builtin_preflight"}});
        assert!(valid_inventory(&active).unwrap());
        assert!(valid_inventory(&idle).unwrap());
        assert!(!valid_inventory(&serde_json::json!({"section":"wrong","value":[]})).unwrap());
        assert!(!valid_inventory(&serde_json::json!({"section":"tools","value":{}})).unwrap());
    }
    #[test]
    fn default_copy_ignores_tool_and_interrupted_text() {
        let snapshot: super::super::state::Snapshot = serde_json::from_value(serde_json::json!({
            "session_id":uuid::Uuid::new_v4(),"revision":3,"total_messages":4,"name":null,"model":"fixture","run":null,"decisions":[],
            "messages":[
                {"message_index":0,"role":"assistant","content":"**canonical**"},
                {"message_index":1,"role":"tool","content":"tool"},
                {"message_index":2,"role":"assistant","content":"interrupted","interrupted_attempt":uuid::Uuid::new_v4()},
                {"message_index":3,"role":"assistant","content":"","tool_calls":[]}
            ]
        })).unwrap();
        assert_eq!(response_index(&snapshot).unwrap(), 0);
    }
}

use super::{App, observe::Update, state::Target};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    pub panel: Option<Panel>,
    pub confirmation: Option<(Observation, u64)>,
    job: Option<Job>,
    admitted: Option<Admission>,
}
struct Job {
    id: Uuid,
    observation: Observation,
    deadline: Instant,
    task: tokio::task::JoinHandle<()>,
}
struct Admission {
    observation: Observation,
    command: Uuid,
    run: Option<Uuid>,
    deadline: Instant,
}
pub(super) struct Loaded {
    pub id: Uuid,
    pub observation: Observation,
    pub result: Result<Content, String>,
}
pub(super) enum Content {
    Panel(Panel),
    Copy(String),
    Result(String),
}
impl App {
    fn inspection_matches(&self, o: Observation) -> bool {
        self.selected == Some(o.target)
            && self.active_draft.is_none()
            && self
                .operator_observation(o.target)
                .is_ok_and(|current| current == o)
    }
    pub(super) fn close_inspection(&mut self) {
        if let Some(job) = self.inspection.job.take() {
            job.task.abort();
            self.retired_observers.push(job.task);
        }
        self.inspection.panel = None;
        self.inspection.confirmation = None;
        self.inspection.admitted = None;
    }
    fn inspection_job<F>(&mut self, o: Observation, work: F)
    where
        F: std::future::Future<Output = Result<Content>> + Send + 'static,
    {
        let id = Uuid::new_v4();
        let sender = self.sender.clone();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(60), work)
                .await
                .map_err(|_| "Inspection read timed out; nothing replayed".to_owned())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = sender
                .send(Update::Inspection(Box::new(Loaded {
                    id,
                    observation: o,
                    result,
                })))
                .await;
        });
        self.inspection.job = Some(Job {
            id,
            observation: o,
            task,
            deadline: Instant::now() + Duration::from_secs(65),
        });
    }
    pub(super) fn open_inspection(&mut self, target: Target) -> Result<()> {
        let o = self.operator_observation(target)?;
        ensure!(
            self.selected == Some(target) && self.active_draft.is_none(),
            "Select the voyage first"
        );
        self.close_inspection();
        let context = format!(
            "{} · {} · workspace {}",
            self.route_label(target.route),
            crate::process_client::safe(&self.views[&target].title()),
            self.views[&target].process.workspace.display()
        );
        let client = self.clients[target.route].clone();
        self.explore = None;
        self.help = false;
        self.inspection_job(o, async move {
            load(&client, o, context).await.map(Content::Panel)
        });
        self.status = "Loading executing-host inspection tools · Esc cancels this read".into();
        Ok(())
    }
    pub(super) fn request_answer_copy(
        &mut self,
        observation: Observation,
        index: u64,
    ) -> Result<()> {
        ensure!(
            self.inspection_matches(observation),
            "Answer changed; select it again"
        );
        let view = self
            .views
            .get(&observation.target)
            .context("Conversation unavailable")?;
        let snapshot = view.snapshot.as_ref().context("Snapshot unavailable")?;
        let state = view.transcript.borrow();
        let message = state
            .messages
            .iter()
            .chain(snapshot.messages.iter())
            .find(|m| m.message_index as u64 == index)
            .context("Answer is no longer loaded; select it again")?;
        ensure!(
            message.role == "assistant"
                && message.interrupted_attempt.is_none()
                && message.operator_name.is_none()
                && message.tool_calls.is_empty()
                && !message.content.is_empty(),
            "Select a saved completed answer"
        );
        drop(state);
        self.inspection.confirmation = Some((observation, index));
        Ok(())
    }
    pub(super) fn request_response_copy(&mut self, target: Target) -> Result<()> {
        let o = self.operator_observation(target)?;
        ensure!(
            self.inspection.job.is_none() && self.inspection.admitted.is_none(),
            "Wait for inspection work first"
        );
        let index = response_index(
            self.views[&target]
                .snapshot
                .as_ref()
                .context("No history")?,
        )?;
        self.inspection.confirmation = Some((o, index));
        self.status = "Copy canonical assistant response to your terminal/system clipboard? Enter confirms; Esc cancels. Terminal clipboard support required; /export PATH is the file fallback.".into();
        Ok(())
    }
    pub(super) fn inspection_loaded(&mut self, loaded: Loaded) {
        if !self
            .inspection
            .job
            .as_ref()
            .is_some_and(|j| j.id == loaded.id && j.observation == loaded.observation)
        {
            return;
        }
        let job = self.inspection.job.take().unwrap();
        self.retired_observers.push(job.task);
        if !self.inspection_matches(loaded.observation) {
            self.close_inspection();
            self.status =
                "Inspection/copy discarded: voyage observation changed; reopen explicitly".into();
            return;
        }
        match loaded.result {
            Ok(Content::Panel(panel)) => self.inspection.panel = Some(panel),
            Ok(Content::Result(text)) => {
                self.inspection.admitted = None;
                if let Some(panel) = &mut self.inspection.panel {
                    panel.result(&text);
                }
            }
            Ok(Content::Copy(sequence)) => {
                use std::io::Write;
                let result = std::io::stdout()
                    .write_all(sequence.as_bytes())
                    .and_then(|_| std::io::stdout().flush());
                self.status = if result.is_ok() {
                    "Copy requested; terminal clipboard support required"
                } else {
                    "Clipboard write failed; use /export PATH"
                }
                .into();
            }
            Err(error) => {
                self.inspection.admitted = None;
                self.status = crate::process_client::safe(&error);
                if let Some(panel) = &mut self.inspection.panel {
                    panel.set_error(error);
                }
            }
        }
    }
    // Observe the exact durable command receipt, never the latest tool/message.
    pub(super) fn inspection_receipt(
        &mut self,
        target: Target,
        command: Uuid,
        refused: bool,
        result: &Result<serde_json::Value, String>,
    ) {
        let Some(a) = self
            .inspection
            .admitted
            .as_mut()
            .filter(|a| a.observation.target == target && a.command == command)
        else {
            return;
        };
        match result {
            Ok(v) if v["command_id"] == serde_json::json!(command) => {
                if matches!(v["status"].as_str(), Some("rejected" | "not_admitted")) {
                    self.inspection.admitted = None;
                    if let Some(p) = &mut self.inspection.panel {
                        p.set_error("Inspection command refused; nothing replayed".into());
                    }
                } else if let Some(run) = v["run_id"].as_str().and_then(|s| Uuid::parse_str(s).ok())
                {
                    a.run = Some(run);
                }
            }
            Err(_) if refused => {
                self.inspection.admitted = None;
                if let Some(p) = &mut self.inspection.panel {
                    p.set_error("Inspection command refused; nothing replayed".into());
                }
            }
            _ => {} // Unknown admission remains owned by ordinary receipt reconciliation.
        }
    }
    pub(super) fn poll_inspection(&mut self) {
        let target = self
            .inspection
            .job
            .as_ref()
            .map(|j| j.observation.target)
            .or_else(|| {
                self.inspection
                    .admitted
                    .as_ref()
                    .map(|a| a.observation.target)
            })
            .or_else(|| self.inspection.confirmation.map(|c| c.0.target))
            .or_else(|| self.inspection.panel.as_ref().map(|p| p.observation.target));
        if target.is_some_and(|t| {
            self.selected != Some(t)
                || self.active_draft.is_some()
                || !self.clients.available(t.route)
        }) {
            self.close_inspection();
            return;
        }
        if self
            .inspection
            .job
            .as_ref()
            .is_some_and(|j| Instant::now() > j.deadline || !self.inspection_matches(j.observation))
        {
            self.close_inspection();
            self.status =
                "Stale inspection read cancelled; no clipboard write or command replay".into();
            return;
        }
        let Some(a) = self.inspection.admitted.as_ref() else {
            return;
        };
        if Instant::now() > a.deadline {
            self.inspection.admitted = None;
            if let Some(p) = &mut self.inspection.panel {
                p.set_error("Result observation timed out; command may continue. Canonical history and durable receipt remain available; not replayed.".into());
            }
            return;
        }
        if self.inspection.job.is_some() {
            return;
        }
        let Some(run) = a.run else { return };
        let target = a.observation.target;
        let Some(snapshot) = self.views.get(&target).and_then(|v| v.snapshot.as_ref()) else {
            return;
        };
        // Idle operator admission may start a fresh owner incarnation. The
        // accepted receipt's exact run ID below, not the preflight incarnation,
        // identifies the result. Read it using a fresh observation; never replay.
        let Some(observed) = snapshot
            .run
            .as_ref()
            .filter(|r| r.run_id == run && !r.active())
        else {
            return;
        };
        if snapshot.pending_cleanup_run.is_some()
            || snapshot
                .cleanup
                .as_ref()
                .is_some_and(|c| !c.pending.is_empty())
        {
            return;
        }
        let state = observed.state.clone();
        // Operator completion checkpoints its canonical assistant result inside
        // this run's Turn range; RunOutput partial_text is not that result.
        let index = snapshot
            .turns
            .iter()
            .find(|t| t.run_id == run)
            .and_then(|t| t.message_start.zip(t.message_end))
            .filter(|(start, end)| end > start)
            .map(|(_, end)| (end - 1) as u64);
        if state != "completed" && index.is_none() {
            self.inspection.admitted = None;
            if let Some(p) = &mut self.inspection.panel {
                p.set_error(format!(
                    "Exact admitted run {run}: {state}; no canonical result"
                ));
            }
            return;
        }
        let Some(index) = index else { return };
        let Ok(o) = self.operator_observation(target) else {
            return;
        };
        if let Some(panel) = &mut self.inspection.panel {
            panel.observation = o;
        }
        let client = self.clients[target.route].clone();
        self.inspection_job(o, async move {
            let text =
                export::response_text(&client, target.session, o.incarnation, o.revision, index)
                    .await?;
            Ok(Content::Result(format!("Inspection {state}\n{text}")))
        });
    }

    pub(super) fn inspection_input(&mut self, event: &Event) -> bool {
        let active = self.inspection.panel.is_some()
            || self.inspection.job.is_some()
            || self.inspection.confirmation.is_some();
        if !active {
            return false;
        }
        if let Event::Key(k) = event {
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(k.code, KeyCode::Char('c' | 'q'))
            {
                self.close_inspection();
                return false;
            }
            if k.kind != KeyEventKind::Press {
                return true;
            }
            if k.code == KeyCode::Esc {
                self.close_inspection();
                return true;
            }
            if let Some((o, index)) = self.inspection.confirmation {
                if k.code == KeyCode::Enter {
                    self.inspection.confirmation = None;
                    if self.inspection_matches(o) {
                        let client = self.clients[o.target.route].clone();
                        self.inspection_job(o, async move {
                            copy_response(&client, o, index).await.map(Content::Copy)
                        });
                    } else {
                        self.status = "Copy cancelled: stale response".into();
                    }
                }
                return true;
            }
        }
        if self.inspection.job.is_some() || self.inspection.admitted.is_some() {
            return true;
        }
        let Some(mut panel) = self.inspection.panel.take() else {
            return true;
        };
        match panel.input(event) {
            super::inspection::Outcome::Close => {}
            super::inspection::Outcome::Stay => self.inspection.panel = Some(panel),
            super::inspection::Outcome::CopyResponse => {
                let target = panel.observation.target;
                self.inspection.panel = Some(panel);
                if let Err(e) = self.request_response_copy(target) {
                    self.status = e.to_string();
                }
            }
            super::inspection::Outcome::Submit(request) => {
                let result = (|| -> Result<()> {
                    ensure!(
                        self.inspection_matches(request.observation),
                        "Voyage changed; reopen inspection"
                    );
                    self.command_for(request.observation.target, request.command_text()?, true)?;
                    let command = self.views[&request.observation.target]
                        .pending
                        .as_ref()
                        .context("Inspection was not admitted to durable command tracking")?
                        .command_id;
                    self.inspection.admitted = Some(Admission {
                        observation: request.observation,
                        command,
                        run: None,
                        deadline: Instant::now() + Duration::from_secs(120),
                    });
                    Ok(())
                })();
                match result {Ok(())=>panel.result("Waiting for exact admitted command receipt, run result and observed cleanup; Esc only closes inspection."),Err(e)=>panel.set_error(e.to_string())}
                self.inspection.panel = Some(panel);
            }
        }
        true
    }
}

#[cfg(test)]
mod app_tests {
    use super::super::{account_test_support::Fixture, accounts::app_tests::app, state::View};
    use super::*;
    use crossterm::event::KeyEvent;
    fn live(app: &mut App) -> Observation {
        let target = Target {
            route: app.clients.first_route().unwrap(),
            session: Uuid::new_v4(),
        };
        let mut view=View::new(serde_json::from_value(serde_json::json!({"session_id":target.session,"incarnation":Uuid::new_v4(),"workspace":"/fixture","state":"live","name":"fixture"})).unwrap());
        view.snapshot=Some(serde_json::from_value(serde_json::json!({"session_id":target.session,"revision":17,"model":"fixture","messages":[{"role":"assistant","content":"**canonical**","message_index":0}],"run":null})).unwrap());
        view.draft.text = "preserve composer".into();
        app.views.insert(target, view);
        app.selected = Some(target);
        app.operator_observation(target).unwrap()
    }
    fn panel(app: &mut App, o: Observation) {
        app.inspection.panel = Some(Panel::new(
            o,
            "fixture".into(),
            serde_json::json!({"section":"tools","value":[]}),
        ));
    }
    #[tokio::test]
    async fn inspection_app_refusal_and_modal_preserve_composer() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let o = live(&mut app);
        panel(&mut app, o);
        assert!(app.inspection_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))));
        assert!(app.views[&o.target].pending.is_none());
        assert_eq!(app.views[&o.target].draft.text, "preserve composer");
        assert!(app.inspection.panel.is_some());
        app.inspection_input(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.inspection.panel.is_none());
    }
    #[tokio::test]
    async fn inspection_app_submit_uses_durable_operator_command_keeps_draft() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let o = live(&mut app);
        app.inspection.panel = Some(Panel::new(
            o,
            "fixture".into(),
            serde_json::json!({"section":"tools","value":[{"name":"shell","input_schema":{}}]}),
        ));
        app.inspection_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let pending = app.views[&o.target]
            .pending
            .as_ref()
            .expect("durable pending command");
        assert!(pending.preserve_draft);
        assert!(
            matches!(pending.original.as_deref(),Some(VoyageCommand::OperatorTool {name,..}) if name=="shell")
        );
        assert_eq!(
            app.inspection.admitted.as_ref().unwrap().command,
            pending.command_id
        );
        assert_eq!(app.views[&o.target].draft.text, "preserve composer");
        app.close_inspection();
        for (_, jobs) in app.route_tasks {
            for job in jobs {
                job.abort();
                let _ = job.await;
            }
        }
    }
    #[tokio::test]
    async fn inspection_app_exact_receipt_and_result_not_unrelated() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let o = live(&mut app);
        panel(&mut app, o);
        let command = Uuid::new_v4();
        let run = Uuid::new_v4();
        app.inspection.admitted = Some(Admission {
            observation: o,
            command,
            run: None,
            deadline: Instant::now() + Duration::from_secs(120),
        });
        let receipt =
            Ok(serde_json::json!({"command_id":command,"run_id":run,"status":"accepted"}));
        app.inspection_receipt(o.target, Uuid::new_v4(), false, &receipt);
        assert!(app.inspection.admitted.as_ref().unwrap().run.is_none());
        app.inspection_receipt(o.target, command, false, &receipt);
        assert_eq!(app.inspection.admitted.as_ref().unwrap().run, Some(run));
        let id = Uuid::new_v4();
        app.inspection.job = Some(Job {
            id,
            observation: o,
            deadline: Instant::now() + Duration::from_secs(65),
            task: tokio::spawn(async {}),
        });
        app.inspection_loaded(Loaded {
            id,
            observation: o,
            result: Ok(Content::Result("exact result".into())),
        });
        assert!(app.inspection.admitted.is_none());
        assert!(app.inspection.panel.is_some());
        assert_eq!(app.views[&o.target].draft.text, "preserve composer");
        app.close_inspection();
    }
    #[tokio::test]
    async fn accepted_idle_inspection_follows_exact_run_across_new_owner() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let old = live(&mut app);
        panel(&mut app, old);
        let run = Uuid::new_v4();
        let new_owner = Uuid::new_v4();
        app.inspection.admitted = Some(Admission {
            observation: old,
            command: Uuid::new_v4(),
            run: Some(run),
            deadline: Instant::now() + Duration::from_secs(120),
        });
        let view = app.views.get_mut(&old.target).unwrap();
        view.process.incarnation = new_owner;
        view.snapshot=Some(serde_json::from_value(serde_json::json!({"session_id":old.target.session,"revision":20,"model":"fixture","messages":[],"run":{"run_id":run,"state":"completed"},"turns":[{"run_id":run,"phase":"completed","message_start":0,"message_end":2}]})).unwrap());
        app.poll_inspection();
        assert!(app.inspection.panel.is_some());
        let job = app
            .inspection
            .job
            .as_ref()
            .expect("exact completed run read");
        assert_eq!(job.observation.incarnation, new_owner);
        assert_eq!(job.observation.revision, 20);
        app.close_inspection();
        for job in app.retired_observers {
            let _ = job.await;
        }
    }
    #[tokio::test]
    async fn inspection_app_copy_disclosure_stale_no_external_write() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let o = live(&mut app);
        app.request_response_copy(o.target).unwrap();
        assert!(app.inspection.confirmation.is_some());
        assert!(app.inspection.job.is_none());
        assert!(app.status.contains("clipboard"));
        app.views
            .get_mut(&o.target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .revision += 1;
        app.inspection_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.inspection.job.is_none());
        assert!(app.status.contains("stale"));
        // A previously confirmed asynchronous copy arriving after revision change
        // is discarded before entering the terminal write branch (invalid payload).
        let id = Uuid::new_v4();
        app.inspection.job = Some(Job {
            id,
            observation: o,
            deadline: Instant::now() + Duration::from_secs(65),
            task: tokio::spawn(async {}),
        });
        app.inspection_loaded(Loaded {
            id,
            observation: o,
            result: Ok(Content::Copy("MUST NOT WRITE".into())),
        });
        assert!(app.status.contains("discarded"));
        assert_eq!(app.views[&o.target].draft.text, "preserve composer");
        assert_eq!(
            export::response_clipboard_sequence("hello").unwrap(),
            "\x1b]52;c;aGVsbG8=\x07"
        );
        for job in app.retired_observers {
            let _ = job.await;
        }
    }
    #[tokio::test]
    async fn inspection_app_close_aborts_owned_read_observes_cleanup() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let o = live(&mut app);
        app.inspection_job(o, std::future::pending());
        app.close_inspection();
        assert!(app.inspection.job.is_none());
        assert_eq!(app.retired_observers.len(), 1);
        for job in app.retired_observers {
            assert!(job.await.unwrap_err().is_cancelled());
        }
    }
}
