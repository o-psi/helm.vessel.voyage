use super::*;
use voyage_protocol::tool_preview::ToolPreview;

#[test]
fn previews_reopen_and_atomically_yield_to_canonical_calls() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut journal = Journal::open(path.clone()).unwrap();
    let session = Session::new(root.path().into(), "fixture".into());
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let request = TurnAdmission {
        coordination: None,
        operator_name: None,
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: now + 60000,
        prompt: "fixture".into(),
        parts: vec![],
    };
    let run = journal.admit_turn(&guard, &request, now).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let preview = ToolPreview {
        attempt_id: Uuid::new_v4(),
        index: 3,
        call_id: Some("call".into()),
        name: "shell".into(),
        arguments: "{command:partial".into(),
        truncated: false,
    };
    journal
        .tool_previews(&guard, run.id, vec![preview.clone()])
        .unwrap();
    let reasoning = voyage_protocol::reasoning_preview::ReasoningPreview {
        attempt_id: preview.attempt_id,
        index: 0,
        kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
        text: "public summary".into(),
        truncated: false,
        finalized: false,
    };
    journal
        .reasoning_previews(&guard, run.id, vec![reasoning.clone()])
        .unwrap();
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    // An error-only checkpoint cannot silently discard unfinished generation.
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    assert_eq!(
        journal.run(run.id).unwrap().tool_previews,
        vec![preview.clone()]
    );
    drop(journal);
    let mut journal = Journal::open(path).unwrap();
    assert_eq!(journal.run(run.id).unwrap().tool_previews, vec![preview]);
    assert_eq!(
        journal.run(run.id).unwrap().reasoning_previews,
        vec![reasoning.clone()]
    );
    let mut assistant = Message::new(crate::model::Role::Assistant, "");
    assistant.tool_calls.push(crate::model::ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"echo final"}),
    });
    // A final checkpoint must correlate to a completed provider attempt, not
    // accidentally finalize a disclosure from an older failed attempt.
    let attempt = voyage_protocol::provider_attempt::ProviderAttempt {
        retry: Default::default(),
        request_id: Uuid::new_v4(),
        attempt_id: reasoning.attempt_id,
        provider: "fixture".into(),
        model: "fixture".into(),
        attempt: 1,
        limit: 1,
        started_at_ms: 1,
        duration_ms: 1,
        phase: voyage_protocol::provider_attempt::AttemptPhase::Stream,
        category: None,
        http_status: None,
        text_observed: false,
        tool_fragment_observed: true,
        retry_delay_ms: None,
        decision: voyage_protocol::provider_attempt::RetryDecision::Completed,
    };
    journal.provider_attempt(&guard, run.id, &attempt).unwrap();
    messages.push(assistant);
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    assert!(journal.run(run.id).unwrap().tool_previews.is_empty());
    let retained = journal.run(run.id).unwrap().reasoning_previews;
    assert_eq!(retained.len(), 1);
    assert!(retained[0].finalized);
    assert_eq!(retained[0].text, reasoning.text);
    assert!(
        journal
            .reasoning_previews(&guard, run.id, vec![reasoning])
            .is_err()
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().session.messages[1]
            .tool_calls
            .len(),
        1
    );
}
