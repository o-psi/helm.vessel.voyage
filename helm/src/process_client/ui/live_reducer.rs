//! Apply bounded public-v2 observations to a hydrated view. Unknown projections
//! are not guessed: the observer requests a fresh snapshot instead.
use super::state::{Message, Run, View};
use serde_json::Value;
use std::time::Instant;

/// Returns true only when the event is fully applied or is an old duplicate.
pub(super) fn apply(view: &mut View, event: &Value) -> bool {
    let Some(snapshot) = view.snapshot.as_mut() else {
        return false;
    };
    let Some(cursor) = event.get("cursor").and_then(Value::as_u64) else {
        return false;
    };
    if snapshot.observation_cursor.is_some_and(|old| cursor <= old) {
        return true;
    }
    let Some(revision) = event.get("revision").and_then(Value::as_u64) else {
        return false;
    };
    if revision < snapshot.revision {
        return false;
    }
    let Some(kind) = event.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let Some(payload) = event.get("payload") else {
        return false;
    };
    let mut grew = false;
    match kind {
        "message_created" | "message_finalized" => {
            let Some(index) = payload
                .get("message_index")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
            else {
                return false;
            };
            let Ok(mut message) = serde_json::from_value::<Message>(
                payload.get("message").cloned().unwrap_or(Value::Null),
            ) else {
                // A creation event may reserve an index before its bounded
                // projection is available. Do not fabricate its contents.
                if kind == "message_created" && index == snapshot.total_messages {
                    snapshot.total_messages += 1;
                    snapshot.revision = revision;
                    snapshot.observation_cursor = Some(cursor);
                    return true;
                }
                return false;
            };
            if index > snapshot.total_messages {
                return false;
            }
            message.message_index = index;
            if let Some(existing) = snapshot
                .messages
                .iter_mut()
                .find(|m| m.message_index == index)
            {
                *existing = message.clone();
            } else {
                snapshot.messages.push(message.clone());
                snapshot.messages.sort_by_key(|m| m.message_index);
                grew = true;
            }
            snapshot.total_messages = snapshot.total_messages.max(index.saturating_add(1));
            let mut transcript = view.transcript.borrow_mut();
            if message.projection_truncated {
                transcript.requested_from = Some(index);
            }
            if transcript
                .delivery
                .as_ref()
                .is_some_and(|d| d.matches(&message))
            {
                transcript.delivery = None;
            }
            if let Some(existing) = transcript
                .messages
                .iter_mut()
                .find(|m| m.message_index == index)
            {
                *existing = message;
            } else if transcript.loaded_revision.is_some() {
                transcript.messages.push(message);
                transcript.messages.sort_by_key(|m| m.message_index);
            }
            if transcript.loaded_revision.is_some() {
                transcript.loaded_revision = Some(revision);
            }
        }
        "text_delta" => {
            let Some(run) = snapshot.run.as_mut() else {
                return false;
            };
            if event.get("run_id").and_then(Value::as_str) != Some(run.run_id.to_string().as_str())
            {
                return false;
            }
            let (Some(offset), Some(text)) = (
                payload.get("offset").and_then(Value::as_u64),
                payload.get("text").and_then(Value::as_str),
            ) else {
                return false;
            };
            if text.len() > 4096 || run.live_text_truncated {
                return false;
            }
            let current = run.live_text.get_or_insert_with(String::new);
            if offset != current.len() as u64 {
                return false;
            }
            current.push_str(text);
            run.partial_text_bytes = current.len() as u64;
            run.live_text_offset = Some(0);
            grew = !text.is_empty();
        }
        "run_state" => {
            let Some(object) = payload.as_object() else {
                return false;
            };
            let Some(id) = object
                .get("run_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<uuid::Uuid>().ok())
            else {
                return false;
            };
            if event.get("run_id").and_then(Value::as_str) != Some(id.to_string().as_str()) {
                return false;
            }
            let Some(state) = object.get("state").and_then(Value::as_str) else {
                return false;
            };
            if !matches!(
                state,
                "accepted"
                    | "running"
                    | "awaiting_decision"
                    | "cancel_requested"
                    | "completed"
                    | "failed"
                    | "cancelled"
                    | "interrupted"
                    | "incomplete"
            ) {
                return false;
            }
            let run = if let Some(existing) = snapshot.run.as_ref().filter(|r| r.run_id == id) {
                let mut run = existing.clone();
                run.state = state.to_owned();
                run
            } else {
                serde_json::from_value::<Run>(payload.clone()).unwrap_or_else(|_| Run {
                    run_id: id,
                    state: state.to_owned(),
                    provider_attempts: vec![],
                    provider_attempt_count: 0,
                    message_start: None,
                    live_text: None,
                    live_text_truncated: false,
                    live_text_offset: None,
                    partial_text_bytes: 0,
                    partial_text: String::new(),
                    tool_previews: vec![],
                    reasoning_previews: vec![],
                    failure_summary: None,
                    partial_text_truncated: false,
                })
            };
            snapshot.run = Some(run);
        }
        "session" => {
            let Some(object) = payload.as_object() else {
                return false;
            };
            if let Some(total) = object.get("total_messages").and_then(Value::as_u64) {
                let Ok(total) = usize::try_from(total) else {
                    return false;
                };
                if total < snapshot.total_messages {
                    return false;
                }
                snapshot.total_messages = total;
            }
            if let Some(name) = object.get("name") {
                if !name.is_null() && !name.is_string() {
                    return false;
                }
                snapshot.name = name.as_str().map(str::to_owned);
            }
            if let Some(model) = object.get("model") {
                let Some(model) = model.as_str() else {
                    return false;
                };
                snapshot.model = model.to_owned();
            }
        }
        "lifecycle" => {
            if !payload.is_object() {
                return false;
            }
            snapshot.lifecycle = payload.clone();
        }
        // These projections require an exact schema before mutating state.
        _ => return false,
    }
    snapshot.revision = revision;
    snapshot.observation_cursor = Some(cursor);
    view.transcript.borrow_mut().observe_growth(grew);
    view.transcript.borrow_mut().dirty = true;
    view.rendered.take();
    view.observed = Some(Instant::now());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_client::ui::coverage_support;
    use serde_json::json;

    #[test]
    fn incremental_messages_deduplicate_and_preserve_history() {
        let (_fixture, mut app, target) = coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let baseline = view
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            .unwrap_or(0);
        let revision = view.snapshot.as_ref().unwrap().revision + 1;
        let index = view.snapshot.as_ref().unwrap().total_messages;
        let event = json!({"cursor":baseline+1,"revision":revision,"kind":"message_finalized","payload":{
            "message_index":index,"message":{"role":"assistant","content":"done"}}});
        assert!(apply(view, &event));
        assert!(apply(view, &event));
        assert_eq!(
            view.snapshot
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .filter(|m| m.message_index == index)
                .count(),
            1
        );
        assert_eq!(index + 1, view.snapshot.as_ref().unwrap().total_messages);
        assert_eq!(
            view.snapshot.as_ref().unwrap().observation_cursor,
            Some(baseline + 1)
        );
    }

    #[test]
    fn reservation_advances_index_without_fabricating_text() {
        let (_fixture, mut app, target) = coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let old = view.snapshot.as_ref().unwrap();
        let (index, cursor, revision, count) = (
            old.total_messages,
            old.observation_cursor.unwrap_or(0) + 1,
            old.revision + 1,
            old.messages.len(),
        );
        assert!(apply(
            view,
            &json!({"cursor":cursor,"revision":revision,"kind":"message_created","payload":{"message_index":index}})
        ));
        assert_eq!(view.snapshot.as_ref().unwrap().total_messages, index + 1);
        assert_eq!(view.snapshot.as_ref().unwrap().messages.len(), count);
    }

    #[test]
    fn sparse_cursor_and_utf8_append_preserve_run_identity() {
        let (_fixture, mut app, target) = coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let baseline = view
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            .unwrap_or(0);
        let revision = view.snapshot.as_ref().unwrap().revision + 1;
        let run = uuid::Uuid::new_v4();
        assert!(apply(
            view,
            &json!({"cursor":baseline+9,"revision":revision,"kind":"run_state","run_id":run,"payload":{"run_id":run,"state":"running","partial_text_bytes":0}})
        ));
        let e = json!({"cursor":baseline+19,"revision":revision,"kind":"text_delta","run_id":run,"payload":{"offset":0,"text":"é"}});
        assert!(apply(view, &e));
        assert!(apply(view, &e));
        assert_eq!(
            view.snapshot
                .as_ref()
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .live_text
                .as_deref(),
            Some("é")
        );
        assert_eq!(
            view.snapshot
                .as_ref()
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .partial_text_bytes,
            2
        );
        assert!(!apply(
            view,
            &json!({"cursor":baseline+21,"revision":revision,"kind":"text_delta","run_id":run,"payload":{"offset":1,"text":"!"}})
        ));
        assert_eq!(
            view.snapshot.as_ref().unwrap().observation_cursor,
            Some(baseline + 19)
        );
    }

    #[test]
    fn malformed_or_unknown_projection_never_advances_cursor() {
        let (_fixture, mut app, target) = coverage_support::app();
        let view = app.views.get_mut(&target).unwrap();
        let before = view.snapshot.as_ref().unwrap().observation_cursor;
        let cursor = before.unwrap_or(0) + 1;
        let revision = view.snapshot.as_ref().unwrap().revision + 1;
        assert!(!apply(
            view,
            &json!({"cursor":cursor,"revision":revision,"kind":"decision","payload":{}})
        ));
        assert!(!apply(
            view,
            &json!({"cursor":cursor,"revision":revision,"kind":"text_delta","payload":{"offset":0,"text":"wrong run"}})
        ));
        assert_eq!(view.snapshot.as_ref().unwrap().observation_cursor, before);
    }
}
