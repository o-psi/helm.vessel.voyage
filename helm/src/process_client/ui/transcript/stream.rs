//! Provider disclosures are a separate display channel, not effort configuration.
use super::super::state::Snapshot;
use super::{Key, Row, State, layout::note};
use crate::process_client::safe;
use voyage_protocol::reasoning_preview::ReasoningKind;

pub(super) fn reasoning(out: &mut Vec<Row>, snapshot: &Snapshot, state: &State, width: u16) {
    let Some(run) = &snapshot.run else { return };
    for block in &run.reasoning_previews {
        // A different final key makes completion collapse even a live expanded block.
        let id = format!(
            "reasoning:{}:{}:{:?}:{}",
            block.attempt_id, block.index, block.kind, block.finalized
        );
        let key = Key::Tool(id.clone());
        let expanded = state.tool_expanded.contains(&id);
        let label = match block.kind {
            ReasoningKind::Summary => "Reasoning summary",
            ReasoningKind::Thinking => "Provider-exposed thinking",
        };
        let status = if block.finalized {
            "saved disclosure"
        } else if run.active() && snapshot.recovery_notice.is_none() && !snapshot.recovery_pending {
            "streaming · provisional"
        } else {
            "unfinished disclosure"
        };
        note(
            out,
            key.clone(),
            format!(
                "{} {label} · {status} · Ctrl+Shift+Up/Down then Ctrl+Space, or double-click to {}",
                if expanded { "▼" } else { "▶" },
                if expanded { "hide" } else { "show" }
            ),
            width,
        );
        if expanded {
            note(
                out,
                key.clone(),
                "Provider disclosure, not the answer; visibility does not change reasoning effort.",
                width,
            );
            let text = safe(&block.text);
            for line in text.lines().take(128) {
                note(out, key.clone(), line, width);
            }
            if block.truncated || text.lines().count() > 128 {
                note(
                    out,
                    key,
                    "… reasoning display bounded; older blocks may be omitted",
                    width,
                );
            }
        }
    }
}

/// Preserve reading/expansion when the final call replaces a stable provisional
/// key. A fragmented ID is correlation only; transfer only after a canonical
/// assistant call with that complete ID exists.
pub(super) fn reconcile(snapshot: &Option<Snapshot>, state: &mut State) {
    let Some(snapshot) = snapshot else { return };
    if let Some(run) = &snapshot.run {
        for block in &run.reasoning_previews {
            if !block.finalized {
                continue;
            }
            let live = format!(
                "reasoning:{}:{}:{:?}:false",
                block.attempt_id, block.index, block.kind
            );
            // Finalization intentionally collapses disclosure, but its keyboard
            // reading position must follow the new key. Do not copy expansion.
            state.tool_expanded.remove(&live);
            if let Some(anchor) = &mut state.anchor
                && anchor.key == Key::Tool(live)
            {
                anchor.key = Key::Tool(format!(
                    "reasoning:{}:{}:{:?}:true",
                    block.attempt_id, block.index, block.kind
                ));
                anchor.offset = 0;
            }
        }
        for preview in &run.tool_previews {
            if let Some(id) = &preview.call_id {
                state.preview_calls.insert(
                    format!("preview:{}:{}", preview.attempt_id, preview.index),
                    id.clone(),
                );
            }
        }
    }
    let messages = if state.loaded_revision == Some(snapshot.revision) {
        &state.messages
    } else {
        &snapshot.messages
    };
    let canonical: std::collections::BTreeSet<_> = messages
        .iter()
        .flat_map(|m| &m.tool_calls)
        .map(|c| c.id.clone())
        .collect();
    let mappings: Vec<_> = state
        .preview_calls
        .iter()
        .filter(|(_, id)| canonical.contains(*id))
        .map(|(key, id)| (key.clone(), id.clone()))
        .collect();
    for (key, id) in mappings {
        if state.tool_expanded.remove(&key) {
            state.tool_expanded.insert(id.clone());
        }
        if let Some(anchor) = &mut state.anchor
            && anchor.key == Key::Tool(key.clone())
        {
            anchor.key = Key::Tool(id);
        }
        state.preview_calls.remove(&key);
    }
    // Correlation cache is display-only and bounded, including interrupted runs.
    while state.preview_calls.len() > voyage_protocol::tool_preview::MAX_CALLS {
        state.preview_calls.pop_first();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_id_preserves_provisional_anchor_then_transfers_only_to_canonical() {
        let mut snapshot: Snapshot = serde_json::from_value(serde_json::json!({
            "session_id":uuid::Uuid::nil(),"revision":1,"name":null,"model":"fixture","messages":[],
            "run":{"run_id":uuid::Uuid::nil(),"state":"running","tool_previews":[{
                "attempt_id":uuid::Uuid::nil(),"index":1,"call_id":"ca","name":"shell","arguments":"{","truncated":false
            }]}
        })).unwrap();
        let mut state = State::default();
        let key = format!("preview:{}:1", uuid::Uuid::nil());
        state.tool_expanded.insert(key.clone());
        state.anchor = Some(super::super::Anchor {
            key: Key::Tool(key.clone()),
            offset: 2,
        });
        reconcile(&Some(snapshot.clone()), &mut state);
        snapshot.run.as_mut().unwrap().tool_previews[0].call_id = Some("call".into());
        reconcile(&Some(snapshot.clone()), &mut state);
        assert_eq!(state.anchor.as_ref().unwrap().key, Key::Tool(key.clone()));
        assert!(state.tool_expanded.contains(&key));
        snapshot.messages.push(serde_json::from_value(serde_json::json!({
            "role":"assistant","content":"","tool_calls":[{"id":"call","name":"shell","arguments":{"command":"true"}}]
        })).unwrap());
        snapshot.run.as_mut().unwrap().tool_previews.clear();
        reconcile(&Some(snapshot), &mut state);
        assert_eq!(state.anchor.as_ref().unwrap().key, Key::Tool("call".into()));
        assert_eq!(state.anchor.as_ref().unwrap().offset, 2);
        assert!(state.tool_expanded.contains("call"));
        assert!(!state.tool_expanded.contains(&key));
    }
    #[test]
    fn finalized_reasoning_migrates_anchor_without_expansion_and_preserves_later_toggle() {
        for kind in ["summary", "thinking"] {
            let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
                "session_id":uuid::Uuid::nil(),"revision":1,"name":null,"model":"fixture","messages":[],
                "run":{"run_id":uuid::Uuid::nil(),"state":"completed","reasoning_previews":[{
                    "attempt_id":uuid::Uuid::nil(),"index":0,"kind":kind,"text":"disclosed text","truncated":false,"finalized":true
                }]}
            })).unwrap();
            let block = &snapshot.run.as_ref().unwrap().reasoning_previews[0];
            let live = format!("reasoning:{}:0:{:?}:false", block.attempt_id, block.kind);
            let saved = format!("reasoning:{}:0:{:?}:true", block.attempt_id, block.kind);
            let mut state = State::default();
            state.tool_expanded.insert(live.clone());
            state.anchor = Some(super::super::Anchor {
                key: Key::Tool(live.clone()),
                offset: 7,
            });
            reconcile(&Some(snapshot.clone()), &mut state);
            assert_eq!(state.anchor.as_ref().unwrap().key, Key::Tool(saved.clone()));
            assert_eq!(state.anchor.as_ref().unwrap().offset, 0);
            assert!(!state.tool_expanded.contains(&live));
            assert!(!state.tool_expanded.contains(&saved));
            let mut rows = Vec::new();
            reasoning(&mut rows, &snapshot, &state, 100);
            assert!(
                !rows
                    .iter()
                    .any(|row| row.line.to_string().contains("disclosed text"))
            );
            // A subsequent human expansion must survive repeated snapshots.
            state.tool_expanded.insert(saved.clone());
            reconcile(&Some(snapshot.clone()), &mut state);
            assert!(state.tool_expanded.contains(&saved));
            let unrelated = Key::Tool("unrelated".into());
            state.anchor.as_mut().unwrap().key = unrelated.clone();
            reconcile(&Some(snapshot), &mut state);
            assert_eq!(state.anchor.as_ref().unwrap().key, unrelated);
        }
    }
    #[test]
    fn reasoning_collapses_by_default_and_on_completion_and_labels_interruption() {
        let mut snapshot: Snapshot = serde_json::from_value(serde_json::json!({
            "session_id":uuid::Uuid::nil(),"revision":1,"name":null,"model":"fixture","messages":[],
            "run":{"run_id":uuid::Uuid::nil(),"state":"running","reasoning_previews":[{
                "attempt_id":uuid::Uuid::nil(),"index":0,"kind":"summary","text":"disclosed text","truncated":false,"finalized":false
            }]}
        })).unwrap();
        let mut state = State::default();
        for width in [30, 100] {
            let mut rows = Vec::new();
            reasoning(&mut rows, &snapshot, &state, width);
            assert!(
                !rows
                    .iter()
                    .any(|r| r.line.to_string().contains("disclosed text"))
            );
        }
        state
            .tool_expanded
            .insert(format!("reasoning:{}:0:Summary:false", uuid::Uuid::nil()));
        let mut rows = Vec::new();
        reasoning(&mut rows, &snapshot, &state, 100);
        assert!(
            rows.iter()
                .any(|r| r.line.to_string().contains("disclosed text"))
        );
        snapshot.run.as_mut().unwrap().state = "failed".into();
        rows.clear();
        reasoning(&mut rows, &snapshot, &state, 100);
        assert!(
            rows.iter()
                .any(|r| r.line.to_string().contains("unfinished disclosure"))
        );
        snapshot.run.as_mut().unwrap().reasoning_previews[0].finalized = true;
        rows.clear();
        reasoning(&mut rows, &snapshot, &state, 100);
        assert!(
            !rows
                .iter()
                .any(|r| r.line.to_string().contains("disclosed text"))
        );
    }
}
