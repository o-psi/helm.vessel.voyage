//! Goal control is reviewed human intent. Only Voyage schedules continuation.
#[cfg(all(test, unix))]
#[path = "goal_journey_tests.rs"]
mod journey_tests;

use super::{
    App,
    observe::Update,
    receipts, safe,
    state::{Pending, Snapshot, Target},
};
use anyhow::{Context, Result, bail, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph},
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use uuid::Uuid;
use voyage_protocol::{
    goals::{
        Goal, GoalAction, GoalLimits, GoalSnapshot, GoalStatus, GoalStopReason, valid_objective,
    },
    vessel::{VesselCommand, VoyageCommand},
};

pub(super) struct Review {
    id: Uuid,
    target: Target,
    incarnation: Uuid,
    state: GoalSnapshot,
    action: Option<GoalAction>,
    owner: Option<bool>,
    owner_at: Option<Instant>,
    draft: String,
    preserve_draft: bool,
    error: Option<String>,
    scroll: Cell<u16>,
}

pub(super) fn is_command(text: &str) -> bool {
    text == "/goal" || text.starts_with("/goal ")
}

fn owner_capability(local: bool, caps: &serde_json::Value) -> bool {
    // Local transport authenticates with the private owner credential. Its
    // capabilities predate the scope field used by remote grants. Never infer
    // owner access from a missing scope on an access-file/remote connection.
    caps["scope"] == "owner" || (local && caps.get("scope").is_none())
}

fn parse(text: &str, state: &GoalSnapshot) -> Result<Option<GoalAction>> {
    let args = text
        .strip_prefix("/goal")
        .context("Invalid Goal command")?
        .trim();
    let (op, value) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
    let value = value.trim();
    let current = || {
        state
            .goal
            .as_ref()
            .context("No current goal. Use /goal set OBJECTIVE first")
    };
    Ok(Some(match op {
        "" | "status" if value.is_empty() => return Ok(None),
        "set" => {
            ensure!(
                valid_objective(value),
                "Objective requires 1–8192 UTF-8 bytes without control characters"
            );
            GoalAction::Set {
                objective: value.into(),
                limits: GoalLimits::default(),
                replace_goal_id: state.goal.as_ref().map(|g| g.id),
                continue_automatically: false,
            }
        }
        "edit" => {
            ensure!(
                valid_objective(value),
                "Objective requires 1–8192 UTF-8 bytes without control characters"
            );
            let goal = current()?;
            ensure!(
                goal.status != GoalStatus::Complete,
                "Replace a completed goal with /goal set OBJECTIVE"
            );
            GoalAction::Edit {
                goal_id: goal.id,
                objective: value.into(),
                limits: goal.limits.clone(),
            }
        }
        "limits" => {
            let goal = current()?;
            ensure!(
                goal.status != GoalStatus::Complete,
                "Replace a completed goal before changing limits"
            );
            let values = value
                .split_whitespace()
                .map(str::parse::<u64>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("Limits must be whole positive numbers")?;
            ensure!(
                values.len() == 4,
                "Use /goal limits RUNS TOKENS SECONDS NO_PROGRESS_TURNS"
            );
            let limits = GoalLimits {
                runs: values[0].try_into()?,
                tokens: values[1],
                elapsed_ms: values[2].checked_mul(1000).context("Time limit overflow")?,
                no_progress_runs: values[3].try_into()?,
            };
            ensure!(
                limits.valid(),
                "Limits: runs 1–1000, tokens 1–10000000, seconds 1–86400, no-progress turns 1–10"
            );
            GoalAction::Edit {
                goal_id: goal.id,
                objective: goal.objective.clone(),
                limits,
            }
        }
        "pause" if value.is_empty() => GoalAction::Pause {
            goal_id: current()?.id,
        },
        "resume" if value.is_empty() => {
            let goal = current()?;
            ensure!(
                goal.status != GoalStatus::Complete
                    && goal.limit_reached().is_none()
                    && goal.usage.unmeasured_runs == 0,
                "Cannot resume: review exhausted limits or explicitly replace a goal with unknown usage"
            );
            GoalAction::Resume { goal_id: goal.id }
        }
        "clear" if value.is_empty() => GoalAction::Clear {
            goal_id: current()?.id,
        },
        _ => bail!(
            "Use /goal [set OBJECTIVE | edit OBJECTIVE | limits RUNS TOKENS SECONDS NO_PROGRESS_TURNS | pause | resume | clear]"
        ),
    }))
}

impl Review {
    fn command(&self, incarnation: Uuid, snapshot: &Snapshot) -> Result<VoyageCommand> {
        ensure!(
            self.owner == Some(true)
                && self
                    .owner_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(30)),
            "Fresh owner access is required; reopen Goal review"
        );
        ensure!(
            incarnation == self.incarnation && snapshot.session_id == self.target.session,
            "Voyage owner changed; reopen Goal review"
        );
        let current = snapshot.goal.as_ref().context("Goal state unavailable")?;
        ensure!(
            current.revision == self.state.revision
                && current.goal.as_ref().map(|g| g.id) == self.state.goal.as_ref().map(|g| g.id),
            "Goal changed since review; reopen it before applying"
        );
        ensure!(
            !snapshot.catalogue_only && !snapshot.recovery_pending,
            "Wait for canonical voyage recovery"
        );
        let action = self.action.clone().context("Choose a Goal action first")?;
        ensure!(
            matches!(action, GoalAction::Pause { .. })
                || (snapshot.run.as_ref().is_none_or(|r| !r.active())
                    && snapshot.pending_cleanup_run.is_none()),
            "Wait for this run and its cleanup; Pause only stops future continuation"
        );
        Ok(VoyageCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: snapshot.revision,
            expires_at_ms: super::super::frontend::deadline()?,
            action,
        })
    }
}

pub(super) fn label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "Active",
        GoalStatus::Paused => "Paused",
        GoalStatus::Complete => "Complete",
        GoalStatus::Blocked => "Blocked",
        GoalStatus::Limited => "Limit reached",
        GoalStatus::NeedsAttention => "Needs attention",
    }
}
fn reason(goal: &Goal) -> &'static str {
    match goal.stop_reason {
        Some(GoalStopReason::UserPaused) => "Paused by you; an accepted run can still finish.",
        Some(GoalStopReason::UserInput) => {
            "New input stopped continuation; review before resuming."
        }
        Some(GoalStopReason::RunLimit) => "Run limit reached.",
        Some(GoalStopReason::TokenLimit) => "Token limit reached.",
        Some(GoalStopReason::TimeLimit) => "Execution time limit reached.",
        Some(GoalStopReason::NoProgress) => {
            "Repeated turns without progress; review the objective."
        }
        Some(GoalStopReason::Interrupted) => {
            "Interrupted; review the previous outcome before resuming."
        }
        Some(GoalStopReason::Cancelled) => "Run stopped.",
        Some(GoalStopReason::AuthorityRevoked) => "Continuation authority is unavailable.",
        Some(GoalStopReason::ApprovalRequired) => "A decision requires your attention.",
        Some(GoalStopReason::ProviderFailure) => "Provider did not finish successfully.",
        Some(GoalStopReason::UsageUnknown) => {
            "Usage is incomplete; a new allowance requires explicit replacement."
        }
        Some(GoalStopReason::UnresolvedEffects) => "Previous work or cleanup is not confirmed.",
        None => match goal.status {
            GoalStatus::Active => {
                "Continues on the Vessel within limits while Helm is disconnected."
            }
            GoalStatus::Complete => "Complete with a recorded model assessment.",
            GoalStatus::Blocked => "Blocked; review the recorded assessment.",
            _ => "Review before resuming.",
        },
    }
}

fn describe(state: &GoalSnapshot) -> String {
    let Some(goal) = &state.goal else {
        return "No current goal.\n".into();
    };
    let mut text = format!(
        "Goal · {}\n{}\n{}\nRuns: {} / {} · Tokens: {}{} / {}\nExecution: {} / {} ms · No progress: {} / {} turns\n",
        label(goal.status),
        safe(&goal.objective),
        reason(goal),
        goal.usage.runs,
        goal.limits.runs,
        if goal.usage.unmeasured_runs > 0 {
            "at least "
        } else {
            ""
        },
        goal.usage
            .input_tokens
            .saturating_add(goal.usage.output_tokens),
        goal.limits.tokens,
        goal.usage.elapsed_ms,
        goal.limits.elapsed_ms,
        goal.usage.no_progress_runs,
        goal.limits.no_progress_runs
    );
    if goal.usage.unmeasured_runs > 0 {
        text.push_str(&format!(
            "{} run(s) have incomplete usage. Resume cannot create a fresh allowance.\n",
            goal.usage.unmeasured_runs
        ));
    }
    if let Some(assessment) = &goal.assessment {
        text.push_str(&format!(
            "\nRecorded model assessment: {}\n",
            safe(&assessment.report.summary)
        ));
        for evidence in &assessment.report.evidence {
            text.push_str(&format!("- {}\n", safe(&evidence.conclusion)));
        }
        text.push_str("Evidence links observed tool results; interpretation of the objective is the model's.\n");
    }
    text
}

/// The command ID alone does not make an arbitrary positive response a Goal receipt.
pub(super) fn receipt_valid(command: &VoyageCommand, value: &serde_json::Value) -> bool {
    let VoyageCommand::GoalUpdate {
        command_id, action, ..
    } = command
    else {
        return true;
    };
    if matches!(
        value["status"].as_str(),
        Some("rejected" | "not_admitted" | "not_applied")
    ) {
        return true;
    }
    if value["status"] != "applied" || value["goal_revision"].as_u64().is_none_or(|r| r == 0) {
        return false;
    }
    let id = match action {
        GoalAction::Set { .. } => Some(*command_id),
        GoalAction::Edit { goal_id, .. }
        | GoalAction::Pause { goal_id }
        | GoalAction::Resume { goal_id } => Some(*goal_id),
        GoalAction::Clear { .. } => None,
    };
    value["goal_id"] == serde_json::to_value(id).unwrap_or_default()
}

impl App {
    pub(super) fn review_goal(
        &mut self,
        target: Target,
        text: &str,
        preserve_draft: bool,
    ) -> Result<()> {
        let view = self.views.get(&target).context("Select a voyage first")?;
        let snapshot = view
            .snapshot
            .as_ref()
            .context("Waiting for an authenticated snapshot")?;
        let state = snapshot
            .goal
            .clone()
            .context("This Voyage does not expose Goal state; update its runtime")?;
        ensure!(
            state
                .goal
                .as_ref()
                .is_none_or(|g| g.session_id == target.session),
            "Goal session mismatch"
        );
        let action = parse(text, &state)?;
        let id = Uuid::new_v4();
        self.goal_review = Some(Review {
            id,
            target,
            incarnation: view.process.incarnation,
            state,
            action,
            owner: None,
            owner_at: None,
            draft: view.draft.text.clone(),
            preserve_draft,
            error: None,
            scroll: Cell::new(0),
        });
        let client = self.clients[target.route].clone();
        let local = client.is_local();
        let sender = self.sender.clone();
        let task = tokio::spawn(async move {
            let owner = tokio::time::timeout(
                Duration::from_secs(10),
                client.request(VesselCommand::Capabilities),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .is_some_and(|caps| owner_capability(local, &caps));
            let _ = sender.send(Update::GoalOwner { target, id, owner }).await;
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(task);
        Ok(())
    }

    pub(super) fn goal_owner(&mut self, target: Target, id: Uuid, owner: bool) {
        if let Some(review) = self
            .goal_review
            .as_mut()
            .filter(|r| r.id == id && r.target == target)
        {
            review.owner = Some(owner);
            review.owner_at = Some(Instant::now());
        }
    }

    fn confirm_goal(&mut self, review: &Review) -> Result<()> {
        ensure!(
            self.clients.available(review.target.route),
            "Vessel disconnected; nothing sent"
        );
        let view = self
            .views
            .get_mut(&review.target)
            .context("Voyage unavailable")?;
        ensure!(
            !view.deleted()
                && !view.archived()
                && !view.connection_unavailable
                && view.error.is_none()
                && view
                    .observed
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(35)),
            "Wait for a fresh active voyage snapshot"
        );
        ensure!(
            view.pending.is_none(),
            "Another command is pending; its outcome must be resolved first"
        );
        let command = review.command(
            view.process.incarnation,
            view.snapshot.as_ref().context("Snapshot unavailable")?,
        )?;
        let command_id = command
            .mutation_id()
            .context("Goal mutation identity missing")?;
        view.pending = Some(Pending {
            account_host: None,
            command_id,
            incarnation: review.incarnation,
            draft: review.draft.clone(),
            preserve_draft: review.preserve_draft,
            original: Some(Box::new(command.clone())),
            receipt_only: false,
        });
        if let Err(error) = receipts::save(&self.clients[review.target.route], view) {
            view.pending = None;
            return Err(error.context("Cannot retain Goal identity; nothing sent"));
        }
        self.dispatch(review.target, command_id, command);
        self.status = "Goal change pending; waiting for its exact receipt".into();
        Ok(())
    }

    pub(super) fn goal_input(&mut self, event: &Event) -> bool {
        let Some(mut review) = self.goal_review.take() else {
            return false;
        };
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
        {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                self.quit = true;
                return true;
            }
            match key.code {
                KeyCode::Esc => return true,
                KeyCode::PageUp | KeyCode::Up => {
                    review.scroll.set(review.scroll.get().saturating_sub(3))
                }
                KeyCode::PageDown | KeyCode::Down => {
                    review.scroll.set(review.scroll.get().saturating_add(3))
                }
                KeyCode::Enter if review.action.is_some() => match self.confirm_goal(&review) {
                    Ok(()) => return true,
                    Err(error) => review.error = Some(safe(&error.to_string())),
                },
                _ => {}
            }
        }
        self.goal_review = Some(review);
        true
    }

    pub(super) fn draw_goal(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(review) = &self.goal_review else {
            return;
        };
        let width = area.width.min(90);
        let height = area.height.min(32);
        let area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let mut text = format!(
            "Vessel: {} · Voyage: {}\n{}\n",
            self.route_label(review.target.route),
            review.target.session,
            describe(&review.state)
        );
        if self
            .views
            .get(&review.target)
            .and_then(|v| v.snapshot.as_ref())
            .and_then(|s| s.goal.as_ref())
            .is_some_and(|s| s != &review.state)
        {
            text.push_str(
                "Goal changed since this review. Esc and reopen to see its current state.\n\n",
            );
        }
        match &review.action {
            Some(GoalAction::Set { objective, limits, replace_goal_id, .. }) => text.push_str(&format!("{}\nNew objective: {}\nLimits: {} runs · {} tokens · {} seconds · {} turns without progress\nSaves paused. Use /goal resume to authorize continuation.\n", if replace_goal_id.is_some() { "CONFIRM REPLACEMENT: the current goal above will be replaced with a new budget." } else { "Create goal" }, safe(objective), limits.runs, limits.tokens, limits.elapsed_ms/1000, limits.no_progress_runs)),
            Some(GoalAction::Edit { objective, limits, .. }) => text.push_str(&format!("Save objective: {}\nLimits: {} runs · {} tokens · {} seconds · {} turns without progress\nUsage is retained; this pauses continuation.\n", safe(objective), limits.runs, limits.tokens, limits.elapsed_ms/1000, limits.no_progress_runs)),
            Some(GoalAction::Pause { .. }) => text.push_str("Pause future continuation. The current run may finish; /stop cancels it.\n"),
            Some(GoalAction::Resume { .. }) => text.push_str("AUTHORIZE automatic continuation within the reviewed limits, including while Helm is disconnected.\n"),
            Some(GoalAction::Clear { .. }) => text.push_str("CONFIRM CLEAR: remove the current goal above. Existing run receipts remain.\n"),
            None => text.push_str("Esc returns to the composer.\n/goal set OBJECTIVE · /goal edit OBJECTIVE\n/goal limits RUNS TOKENS SECONDS NO_PROGRESS_TURNS\n/goal pause · /goal resume · /goal clear\n"),
        }
        text.push_str("\nTokens may exceed the limit during an in-flight request; this is not a billing cap.\n");
        text.push_str(match review.owner {
            None => "Checking owner access…\n",
            Some(false) => "Goal changes require owner access.\n",
            Some(true) => "Owner access observed; revalidated by the runtime.\n",
        });
        if review.action.is_some() {
            text.push_str("Enter confirms this exact change · Esc cancels · PgUp/PgDn scroll\n");
        }
        if let Some(error) = &review.error {
            text.push_str(error);
        }
        let block = super::right_panel::block("Voyage Goal · Esc Back · PgUp/PgDn");
        let inner = block.inner(area);
        let text = super::presentation::wrap(ratatui::text::Text::raw(text), inner.width);
        let max = text
            .lines
            .len()
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16;
        review.scroll.set(review.scroll.get().min(max));
        frame.render_widget(
            Paragraph::new(text)
                .block(block)
                .scroll((review.scroll.get(), 0)),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use voyage_protocol::goals::GoalUsage;

    fn state(session: Uuid) -> GoalSnapshot {
        GoalSnapshot {
            revision: 3,
            goal: Some(Goal {
                id: Uuid::new_v4(),
                session_id: session,
                objective: "Verify the output".into(),
                status: GoalStatus::Paused,
                continuation_authorized: false,
                limits: GoalLimits::default(),
                usage: GoalUsage::default(),
                created_at_ms: 1,
                updated_at_ms: 1,
                stop_reason: Some(GoalStopReason::UserPaused),
                assessment: None,
            }),
        }
    }
    fn review(
        target: Target,
        incarnation: Uuid,
        state: GoalSnapshot,
        action: Option<GoalAction>,
    ) -> Review {
        Review {
            id: Uuid::new_v4(),
            target,
            incarnation,
            state,
            action,
            owner: Some(true),
            owner_at: Some(Instant::now()),
            draft: "/goal resume".into(),
            preserve_draft: true,
            error: None,
            scroll: Cell::new(0),
        }
    }
    #[test]
    fn commands_require_explicit_resume_and_preserve_usage_on_edit() {
        assert!(owner_capability(true, &json!({"protocol":1})));
        assert!(!owner_capability(false, &json!({"protocol":1})));
        assert!(!owner_capability(true, &json!({"scope":"session"})));
        assert!(owner_capability(false, &json!({"scope":"owner"})));
        let mut state = state(Uuid::new_v4());
        let id = state.goal.as_ref().unwrap().id;
        assert!(parse("/goal", &state).unwrap().is_none());
        assert!(
            matches!(parse("/goal set New objective", &state).unwrap(), Some(GoalAction::Set { replace_goal_id: Some(old), continue_automatically: false, limits, .. }) if old == id && limits == GoalLimits::default())
        );
        assert!(
            matches!(parse("/goal limits 4 5000 60 2", &state).unwrap(), Some(GoalAction::Edit { goal_id, limits, objective }) if goal_id == id && limits.elapsed_ms == 60000 && objective == "Verify the output")
        );
        for bad in [
            "/goal limits 0 100 30 1",
            "/goal limits 1 100 86401 1",
            "/goal limits 1 100 1",
            "/goal limits 1 100 18446744073709551615 1",
            "/goal pause ignored",
            "/goal set \u{1b}[31m",
        ] {
            assert!(parse(bad, &state).is_err(), "{bad}");
        }
        state.goal.as_mut().unwrap().usage.unmeasured_runs = 1;
        assert!(parse("/goal resume", &state).is_err());
        state.goal.as_mut().unwrap().usage.unmeasured_runs = 0;
        state.goal.as_mut().unwrap().usage.runs = 20;
        assert!(parse("/goal resume", &state).is_err());
        state.goal.as_mut().unwrap().status = GoalStatus::Complete;
        assert!(parse("/goal edit changed", &state).is_err());
        assert!(parse("/goal clear", &GoalSnapshot::default()).is_err());
    }

    #[test]
    fn reviewed_goal_fences_identity_revision_authority_and_cleanup() {
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let target = Target {
            route: super::super::state::Route {
                id: Uuid::new_v4(),
                generation: 1,
            },
            session,
        };
        let state = state(session);
        let action = parse("/goal resume", &state).unwrap();
        let mut review = review(target, incarnation, state.clone(), action);
        let mut snapshot: Snapshot = serde_json::from_value(
            json!({"session_id":session,"revision":7,"model":"fixture","messages":[],"goal":state}),
        )
        .unwrap();
        assert!(review.command(incarnation, &snapshot).is_ok());
        snapshot.revision += 1;
        assert!(matches!(
            review.command(incarnation, &snapshot).unwrap(),
            VoyageCommand::GoalUpdate {
                expected_revision: 8,
                ..
            }
        ));
        assert!(review.command(Uuid::new_v4(), &snapshot).is_err());
        snapshot.goal.as_mut().unwrap().revision += 1;
        assert!(review.command(incarnation, &snapshot).is_err());
        snapshot.goal.as_mut().unwrap().revision -= 1;
        review.owner = Some(false);
        assert!(review.command(incarnation, &snapshot).is_err());
        review.owner = Some(true);
        review.owner_at = Some(Instant::now() - Duration::from_secs(31));
        assert!(review.command(incarnation, &snapshot).is_err());
        review.owner_at = Some(Instant::now());
        snapshot.pending_cleanup_run = Some(Uuid::new_v4());
        assert!(review.command(incarnation, &snapshot).is_err());
        review.action = parse("/goal pause", &review.state).unwrap();
        assert!(review.command(incarnation, &snapshot).is_ok());
        snapshot.recovery_pending = true;
        assert!(review.command(incarnation, &snapshot).is_err());
    }

    #[tokio::test]
    async fn modal_is_read_only_until_confirmed_and_unknown_receipts_remain_pending() {
        let (_fixture, mut app, target) = super::super::coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let state = state(target.session);
        view.snapshot.as_mut().unwrap().goal = Some(state.clone());
        view.observed = Some(Instant::now());
        let mut review = review(
            target,
            view.process.incarnation,
            state.clone(),
            parse("/goal resume", &state).unwrap(),
        );
        review.owner = Some(false);
        app.goal_review = Some(review);
        let key = |code| Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE));
        app.goal_input(&Event::Paste("Never insert into conversation".into()));
        app.goal_input(&key(KeyCode::Enter));
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        let review = app.goal_review.as_mut().unwrap();
        assert!(review.error.as_ref().unwrap().contains("owner"));
        review.owner = Some(true);
        review.owner_at = Some(Instant::now());
        app.goal_input(&key(KeyCode::Enter));
        assert!(app.goal_review.is_none());
        let pending = app.views[&target].pending.as_ref().unwrap().clone();
        assert!(matches!(
            pending.resolution(),
            VoyageCommand::Resolve { .. }
        ));
        for value in [
            json!({"command_id":pending.command_id,"status":"unknown_after_restart"}),
            json!({"command_id":pending.command_id,"status":"applied","goal_revision":4,"goal_id":Uuid::new_v4()}),
        ] {
            app.update(Update::Command {
                target,
                command_id: pending.command_id,
                refused: false,
                result: Ok(value),
            });
            assert!(app.views[&target].pending.is_some());
        }
        app.update(Update::Command { target, command_id: pending.command_id, refused: false, result: Ok(json!({"command_id":pending.command_id,"status":"applied","goal_revision":4,"goal_id":state.goal.unwrap().id})) });
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }

    #[tokio::test]
    async fn review_render_is_bounded_scrollable_and_metadata_cannot_forge_completion() {
        let (_fixture, mut app, target) = super::super::coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let mut state = state(target.session);
        state.goal.as_mut().unwrap().objective =
            "<script>plain text</script>\u{1b}[31m\u{202e}".into();
        let text = describe(&state);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        view.snapshot.as_mut().unwrap().goal = Some(state.clone());
        let original = view.snapshot.clone();
        assert!(!super::super::live_reducer::apply(
            view,
            &json!({"kind":"goal","cursor":8,"revision":18,"payload":{"status":"complete","objective":"forged"}})
        ));
        assert!(view.snapshot == original);
        app.goal_review = Some(review(target, view.process.incarnation, state, None));
        for (width, height) in [(40, 18), (80, 24), (160, 48)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| app.draw_goal(frame, frame.area()))
                .unwrap();
            let screen: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(screen.contains("Voyage Goal"));
            app.goal_input(&Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::PageDown,
                KeyModifiers::NONE,
            )));
        }
        app.goal_input(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.goal_review.is_none());
        assert!(app.views[&target].pending.is_none());
    }
}
