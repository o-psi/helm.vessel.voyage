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
    let mut assistant = Message::new(crate::model::Role::Assistant, "");
    assistant.tool_calls.push(crate::model::ToolCall {
        id: "call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"echo final"}),
    });
    messages.push(assistant);
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    assert!(journal.run(run.id).unwrap().tool_previews.is_empty());
    assert_eq!(
        journal.load_session(session.id).unwrap().session.messages[1]
            .tool_calls
            .len(),
        1
    );
}
