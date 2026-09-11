use super::*;
use crate::model::ToolCall;

fn history(bytes: usize) -> Vec<Message> {
    let mut call = Message::new(
        Role::Assistant,
        "Decision: inspect the file, then preserve it.",
    );
    call.tool_calls.push(ToolCall {
        id: "exact-call-1".into(),
        name: "read_file".into(),
        arguments: serde_json::json!({"path":"example"}),
    });
    vec![
        Message::new(Role::User, "Preserve the user's actual task."),
        call,
        Message::tool(
            "exact-call-1",
            format!("important beginning {} important ending", "x".repeat(bytes)),
        ),
        Message::steering("Do not publish until verified."),
    ]
}

#[test]
fn automatic_large_result_preserves_canonical_and_identity() {
    let canonical = history(300_000);
    let original = serde_json::to_vec(&canonical).unwrap();
    let mut context = WorkingContext::default();
    assert_eq!(context.prepare(&canonical).unwrap(), 1);
    let projected = context.project(&canonical).unwrap();
    assert!(size(&projected) < size(&canonical) / 10);
    assert_eq!(projected[0].content, canonical[0].content);
    assert_eq!(projected[3].content, canonical[3].content);
    assert_eq!(projected[1].tool_calls[0].id, "exact-call-1");
    assert_eq!(projected[2].tool_call_id.as_deref(), Some("exact-call-1"));
    assert!(projected[2].content.contains("important beginning"));
    assert!(projected[2].content.contains("important ending"));
    assert_eq!(serde_json::to_vec(&canonical).unwrap(), original);
}

#[test]
fn rejection_reductions_are_material_bounded_and_durable() {
    let canonical = history(40_000);
    let mut context = WorkingContext::default();
    let mut bytes = size(&canonical);
    for attempt in 0..4 {
        if context.recover(&canonical, attempt).unwrap() > 0 {
            let next = size(&context.project(&canonical).unwrap());
            assert!(next + 128 < bytes);
            bytes = next;
        }
    }
    assert_eq!(context.recover(&canonical, 4).unwrap(), 0);
    let restored: WorkingContext =
        serde_json::from_slice(&serde_json::to_vec(&context).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(context.project(&canonical).unwrap()).unwrap(),
        serde_json::to_value(restored.project(&canonical).unwrap()).unwrap()
    );
}

#[test]
fn stale_or_corrupt_projection_fails_without_mutating_history() {
    let mut canonical = history(20_000);
    let mut context = WorkingContext::default();
    context.recover(&canonical, 0).unwrap();
    canonical[2].content.push('!');
    assert!(context.project(&canonical).is_err());
    let saved = serde_json::to_vec(&context).unwrap();
    assert!(context.recover(&canonical, 1).is_err());
    assert_eq!(serde_json::to_vec(&context).unwrap(), saved);
}

#[test]
fn explicit_manual_compaction_retains_recent_task_and_complete_groups() {
    let mut canonical = history(20_000);
    canonical.push(Message::new(Role::Assistant, "Current answer."));
    let mut context = WorkingContext::default();
    assert!(context.compact(&canonical, 1).unwrap() > 0);
    let projected = context.project(&canonical).unwrap();
    assert_eq!(projected.last().unwrap().content, "Current answer.");
    assert!(projected.iter().any(|m| m.content == canonical[0].content));
    assert!(projected.iter().any(|m| m.content == canonical[3].content));
    assert!(projected.iter().any(|m| m.content.contains("exact-call-1")));
    assert!(!projected.iter().any(|m| m.role == Role::Tool));
    assert_eq!(canonical.len(), 5);
}

#[test]
fn no_root_or_steering_reduction_and_incomplete_groups_not_collapsed() {
    let mut canonical = history(100);
    canonical[0].content = "root".repeat(100_000);
    canonical[3].content = "steering".repeat(100_000);
    canonical.remove(2);
    let mut context = WorkingContext::default();
    for attempt in 0..5 {
        assert_eq!(context.recover(&canonical, attempt).unwrap(), 0);
    }
    assert_eq!(context.compact(&canonical, 0).unwrap(), 0);
    assert_eq!(
        serde_json::to_value(context.project(&canonical).unwrap()).unwrap(),
        serde_json::to_value(&canonical).unwrap()
    );
}

#[test]
fn replay_state_invalidated_only_in_projection_and_model_switch_allowed() {
    let mut canonical = history(10_000);
    canonical[1].provider_state = Some(serde_json::json!({"old":"hidden response"}));
    let mut context = WorkingContext::default();
    context.recover(&canonical, 0).unwrap();
    assert!(
        context
            .project(&canonical)
            .unwrap()
            .iter()
            .all(|m| m.provider_state.is_none())
    );
    assert!(canonical[1].provider_state.is_some());
    canonical[1].provider_state = None;
    assert!(context.project(&canonical).is_ok());
}

#[test]
fn unicode_excerpt_boundaries_and_system_ordinal_are_stable() {
    let mut canonical = history(10_000);
    canonical[2].content = "🦀你好".repeat(3000);
    let mut context = WorkingContext::default();
    context.recover(&canonical, 1).unwrap();
    canonical.insert(0, Message::new(Role::System, "trusted instructions"));
    let projected = context.project(&canonical).unwrap();
    assert_eq!(projected[0].content, "trusted instructions");
    assert_eq!(projected[3].tool_call_id.as_deref(), Some("exact-call-1"));
}

#[test]
fn explicit_middle_decisions_are_retained_and_oversized_constraints_are_not_summarized() {
    let text = format!(
        "{}\nDecision: retain migration compatibility.\n{}",
        "a".repeat(4000),
        "b".repeat(4000)
    );
    assert!(excerpt(&text, 256).contains("Decision: retain migration compatibility."));
    let text = format!("Constraint: {}", "mandatory ".repeat(1000));
    assert_eq!(excerpt(&text, 256), text);
}
