use super::*;

fn tool(rev: u64, run: Uuid) -> RuntimeCommand {
    RuntimeCommand::ExecuteTool {
        command_id: Uuid::new_v4(),
        expected_revision: rev,
        expires_at_ms: 61000,
        run_id: run,
        name: "read_file".into(),
        arguments: json!({"path":"local"}),
    }
}
fn id(command: &RuntimeCommand) -> Uuid {
    match command {
        RuntimeCommand::ExecuteTool { command_id, .. } => *command_id,
        _ => unreachable!(),
    }
}

#[test]
fn accepted_tool_is_not_redispatched_and_completion_is_write_once() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let command = tool(revision(&j, s.id), run.id);
    assert!(j.control_receipt(&command).unwrap().is_none());
    let (receipt, dispatch) = j.admit_control(&g, command.clone(), 1000).unwrap();
    assert!(dispatch);
    assert_eq!(receipt["outcome"], "pending_or_unknown");
    assert_eq!(
        j.admit_control(&g, command.clone(), 999999).unwrap(),
        (receipt, false)
    );
    j.complete_control(&g, id(&command), json!({"ok":true}))
        .unwrap();
    assert!(
        j.complete_control(&g, id(&command), json!("overwrite"))
            .is_err()
    );
    let durable = j.control_receipt(&command).unwrap().unwrap();
    assert_eq!(durable["outcome"], json!({"ok":true}));
    assert_eq!(
        j.process_receipt(id(&command)).unwrap(),
        Some(durable.clone())
    );
    assert_eq!(
        j.admit_control(&g, command, 999999).unwrap(),
        (durable, false)
    );
}

#[test]
fn tool_payload_conflict_and_oversized_result_preserve_unknown_outcome() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let command = tool(revision(&j, s.id), run.id);
    j.admit_control(&g, command.clone(), 1000).unwrap();
    let mut other = command.clone();
    if let RuntimeCommand::ExecuteTool { arguments, .. } = &mut other {
        *arguments = json!({"path":"elsewhere"});
    }
    assert!(j.control_receipt(&other).is_err());
    assert!(j.admit_control(&g, other, 1000).is_err());
    assert!(
        j.complete_control(&g, id(&command), json!("x".repeat(2 * 1024 * 1024)))
            .is_err()
    );
    assert_eq!(
        j.control_receipt(&command).unwrap().unwrap()["outcome"],
        "pending_or_unknown"
    );
    j.complete_control(&g, id(&command), json!("bounded"))
        .unwrap();
}

#[test]
fn tool_admission_rejects_invalid_identity_deadline_revision_and_run() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let rev = revision(&j, s.id);
    let mut nil = tool(rev, run.id);
    if let RuntimeCommand::ExecuteTool { command_id, .. } = &mut nil {
        *command_id = Uuid::nil();
    }
    for (command, now) in [
        (nil, 1000),
        (tool(rev + 1, run.id), 1000),
        (tool(rev, Uuid::new_v4()), 1000),
        (tool(rev, run.id), 61000),
        (tool(rev, run.id), -300000),
        (RuntimeCommand::Health, 1000),
    ] {
        assert!(j.admit_control(&g, command, now).is_err());
    }
    let count: i64 = j
        .connection
        .query_row("SELECT count(*) FROM process_commands", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert!(j.control_receipt(&RuntimeCommand::Health).is_err());
    assert!(j.complete_control(&g, Uuid::new_v4(), Value::Null).is_err());
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    assert!(
        j.admit_control(&g, tool(revision(&j, s.id), run.id), 1000)
            .is_err()
    );
}

#[test]
fn metadata_receipts_are_idempotent_and_bind_payload() {
    let (_root, mut j, s, g) = fixture();
    assert!(j.process_latest_run(s.id).unwrap().is_none());
    let command = rename(0, "renamed");
    let receipt = j.process_metadata(&g, actor(), &command, 1000).unwrap();
    assert_eq!(receipt["revision"], 1);
    assert_eq!(
        j.process_metadata(&g, actor(), &command, 999999).unwrap(),
        receipt
    );
    assert_eq!(
        j.load_session(s.id).unwrap().session.name.as_deref(),
        Some("renamed")
    );
    let mut conflict = command;
    if let RuntimeCommand::Rename { name, .. } = &mut conflict {
        *name = "conflict".into();
    }
    assert!(j.process_metadata(&g, actor(), &conflict, 1000).is_err());
    for command in [
        rename(0, "stale"),
        rename(1, " "),
        rename(1, &"x".repeat(1025)),
        RuntimeCommand::Health,
    ] {
        assert!(j.process_metadata(&g, actor(), &command, 1000).is_err());
    }
    assert_eq!(revision(&j, s.id), 1);
    assert!(j.process_receipt(Uuid::new_v4()).unwrap().is_none());
}

#[test]
fn process_receipts_project_turns_and_metadata_cancel_does_not_finish_execution() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    assert_eq!(j.process_latest_run(s.id).unwrap().unwrap().id, run.id);
    assert!(j.process_receipt(run.command_id).unwrap().is_some());
    let command = RuntimeCommand::Cancel {
        command_id: Uuid::new_v4(),
        expected_revision: revision(&j, s.id),
        expires_at_ms: 61000,
        run_id: run.id,
    };
    let receipt = j.process_metadata(&g, actor(), &command, 1000).unwrap();
    assert_eq!(j.run(run.id).unwrap().state, RunState::Accepted);
    assert_eq!(
        j.process_metadata(&g, actor(), &command, 999999).unwrap(),
        receipt
    );
}

#[test]
fn workflow_binding_is_persisted_only_before_dispatch() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let invocation = crate::workflow::Invocation {
        id: "review".into(),
        version: "1".into(),
        digest: "a".repeat(64),
        scope: crate::workflow::Scope::Repository,
        inputs: [("path".into(), json!("src"))].into_iter().collect(),
    };
    let before = revision(&j, s.id);
    j.record_workflow(&g, run.id, invocation.clone()).unwrap();
    let saved = j.load_session(s.id).unwrap();
    assert_eq!(saved.revision, before + 1);
    assert_eq!(saved.session.workflow_runs[0].id, "review");
    assert_eq!(saved.session.workflow_runs[0].inputs["path"], "src");
    j.mark_running(&g, run.id).unwrap();
    assert!(j.record_workflow(&g, run.id, invocation).is_err());
    assert_eq!(j.load_session(s.id).unwrap().session.workflow_runs.len(), 1);
}

#[test]
fn metadata_model_change_rejects_active_run_and_clears_pending_model_when_idle() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let command = |revision| RuntimeCommand::SetModel {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: 61000,
        model: "replacement".into(),
    };
    assert!(
        j.process_metadata(&g, actor(), &command(revision(&j, s.id)), 1000)
            .is_err()
    );
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    j.process_metadata(&g, actor(), &command(revision(&j, s.id)), 1000)
        .unwrap();
    let saved = j.load_session(s.id).unwrap();
    assert_eq!(saved.session.model, "replacement");
    assert!(saved.session.pending_model.is_none());
}

#[test]
fn root_decision_requires_typed_consent_and_deduplicates_without_replay() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    j.mark_running(&g, run.id).unwrap();
    let incarnation = Uuid::new_v4();
    let decision = Uuid::new_v4();
    j.create_decision(
        &g,
        run.id,
        incarnation,
        decision,
        61000,
        json!({"kind":"root_grant"}),
    )
    .unwrap();
    let mut command = RuntimeCommand::Respond {
        command_id: Uuid::new_v4(),
        expected_revision: revision(&j, s.id),
        expires_at_ms: 61000,
        run_id: run.id,
        decision_id: decision,
        response: json!("approved"),
    };
    assert!(
        j.respond_decision(&g, incarnation, &command, || Ok(1000))
            .is_err()
    );
    if let RuntimeCommand::Respond { response, .. } = &mut command {
        *response = json!({"root_grant":"approved"});
    }
    let receipt = j
        .respond_decision(&g, incarnation, &command, || Ok(1000))
        .unwrap();
    assert_eq!(
        j.respond_decision(&g, incarnation, &command, || Ok(999999))
            .unwrap(),
        receipt
    );
    if let RuntimeCommand::Respond { response, .. } = &mut command {
        *response = json!({"root_grant":"denied"});
    }
    assert!(
        j.respond_decision(&g, incarnation, &command, || Ok(1000))
            .is_err()
    );
    assert_eq!(
        j.decision_response(decision, 999999).unwrap(),
        Some(json!({"root_grant":"approved"}))
    );
}
