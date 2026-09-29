use super::*;
use crate::model::{Message, Role};

#[test]
fn public_text_and_final_message_are_atomic_bounded_and_private_state_is_excluded() {
    let (_root, mut journal, session, guard) = fixture();
    let run = admit(&mut journal, &guard);
    let delta = "é".repeat(3000);
    journal.mark_running(&guard, run.id).unwrap();
    journal.append_text(&guard, run.id, &delta).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let mut assistant = Message::new(Role::Assistant, "final output");
    assistant.provider_state = Some(json!({"private_marker":"do not reveal"}));
    messages.push(assistant);
    // A canonical checkpoint requires the current provider attempt only for tool
    // calls, which are absent here.
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    let page = journal.live_observations(session.id, 0, 128).unwrap();
    assert_eq!(page["projection"], "public-v2");
    let typed: voyage_protocol::live_events::LiveEventPage =
        serde_json::from_value(page.clone()).unwrap();
    assert_eq!(typed.events.len(), page["events"].as_array().unwrap().len());
    let events = page["events"].as_array().unwrap();
    let created = events
        .iter()
        .find(|e| e["kind"] == "message_created")
        .unwrap();
    assert_eq!(created["payload"]["message"]["content"], "retained input");
    assert_eq!(created["payload"]["message_index"], 0);
    let text: Vec<_> = events
        .iter()
        .filter(|e| e["kind"] == "text_delta")
        .collect();
    assert_eq!(text.len(), 2);
    assert_eq!(text[0]["payload"]["offset"], 0);
    assert_eq!(text[1]["payload"]["offset"], 4096);
    assert_eq!(
        text.iter()
            .map(|e| e["payload"]["text"].as_str().unwrap())
            .collect::<String>(),
        delta
    );
    let final_message = events
        .iter()
        .find(|e| e["kind"] == "message_finalized")
        .unwrap();
    assert_eq!(
        final_message["payload"]["message"]["content"],
        "final output"
    );
    assert_eq!(final_message["payload"]["message_index"], 1);
    assert!(!page.to_string().contains("private_marker"));
    let replay = journal.live_observations(session.id, 0, 128).unwrap();
    assert_eq!(page, replay);
}

#[test]
fn sparse_session_cursors_do_not_create_false_replay_gaps_or_cross_session_receipts() {
    let (_root, mut journal, session, guard) = fixture();
    let other = Session::new(session.workspace.clone(), "other".into());
    journal.create_session(&other).unwrap();
    let command = rename(revision(&journal, session.id), "new name");
    journal
        .process_metadata(&guard, actor(), &command, 1000)
        .unwrap();
    let other_events = journal.live_observations(other.id, 0, 128).unwrap();
    assert!(!other_events["replay_gap"].as_bool().unwrap());
    assert!(
        other_events["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["kind"] != "command_outcome")
    );
    let events = journal.live_observations(session.id, 0, 128).unwrap();
    assert!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["session_id"] == session.id.to_string())
    );
}

#[test]
fn command_and_decision_observations_never_publish_private_request_or_response() {
    let (_root, mut journal, session, guard) = fixture();
    let run = admit(&mut journal, &guard);
    journal.mark_running(&guard, run.id).unwrap();
    let decision_id = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    journal
        .create_decision(
            &guard,
            run.id,
            incarnation,
            decision_id,
            60000,
            json!({"private_marker":"secret decision"}),
        )
        .unwrap();
    let page = journal.live_observations(session.id, 0, 128).unwrap();
    assert!(!page.to_string().contains("private_marker"));
    assert!(
        page["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["kind"] == "decision" && event["payload"]["status"] == "pending")
    );
}
