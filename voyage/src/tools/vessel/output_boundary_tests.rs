//! Public observation limits cannot erase uncertainty or suggest repeated effects.
use super::*;
use serde_json::json;

#[test]
fn withheld_and_unknown_statuses_remain_independent_of_small_output_budget() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    for budget in [4096, 180, 1] {
        context.max_output_bytes = budget;
        let report = output(
            json!({"status":"outcome_unknown","detail":"x".repeat(5000)}),
            &context,
            &json!({"action":"submit","command_id":"fixture-command","target":"remote"}),
        )
        .unwrap();
        assert_eq!(report.outcome.execution, ExecutionOutcome::Unknown);
        assert_eq!(
            report.outcome.incomplete,
            Some(IncompleteReason::OutputLimit)
        );
        assert!(report.output.is_error);
        assert!(report.output.text_fallback().len() <= budget);
    }
    context.max_output_bytes = 4096;
    for status in [
        "cursor_expired",
        "cursor_mismatch",
        "cursor_required",
        "source_changed",
        "snapshot_unavailable",
        "revision_changed",
        "path_unavailable",
    ] {
        let report = output(
            json!({"status":status}),
            &context,
            &json!({"action":"details"}),
        )
        .unwrap();
        assert_eq!(report.outcome.incomplete, Some(IncompleteReason::Withheld));
        assert!(report.output.is_error);
    }
}

#[test]
fn provider_attempt_trimming_pins_revision_and_exact_first_withheld_offset() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.max_output_bytes = 1800;
    let request = json!({"action":"provider_attempts","session_id":"fixture-session","target":"remote","offset":5,"limit":4});
    let result = json!({"revision":23,"offset":5,"total":12,
        "attempts":(5..9).map(|i|json!({"index":i,"detail":"x".repeat(700)})).collect::<Vec<_>>()});
    let report = output(result, &context, &request).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    let count = shown["attempts"].as_array().unwrap().len();
    assert!(count > 0 && count < 4);
    assert_eq!(shown["next_offset"], 5 + count);
    assert_eq!(shown["next_read"]["offset"], 5 + count);
    assert_eq!(shown["next_read"]["expected_revision"], 23);
    assert_eq!(shown["next_read"]["target"], "remote");
    assert_eq!(shown["has_more"], true);
    assert!(report.output.text_fallback().len() <= context.max_output_bytes);
}

#[test]
fn event_gap_returns_fresh_inspection_while_normal_page_preserves_cursor() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    for action in ["follow", "wait"] {
        let request = json!({"action":action,"session_id":"fixture-session","target":"remote","after":7,"limit":1});
        let report = output(
            json!({"cursor":15,"events":[],"replay_gap":true}),
            &context,
            &request,
        )
        .unwrap();
        let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
        assert_eq!(
            shown["next_read"],
            json!({"action":"inspect","session_id":"fixture-session","target":"remote"})
        );
        let report = output(
            json!({"cursor":15,"events":[],"replay_gap":false}),
            &context,
            &request,
        )
        .unwrap();
        let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
        assert_eq!(shown["next_read"]["after"], 15);
        assert_eq!(shown["next_read"]["action"], action);
        assert_eq!(shown["next_read"]["limit"], 1);
    }
}

#[test]
fn oversized_nonpage_action_keeps_explicit_missing_state_without_invented_continuation() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.max_output_bytes = 1024;
    let report = output(
        json!({"detail":"x".repeat(5000)}),
        &context,
        &json!({"action":"capabilities"}),
    )
    .unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["status"], "output_limit");
    assert_eq!(shown["complete"], false);
    assert!(shown.get("next_read").is_none());
    assert!(shown["detail"].as_str().unwrap().contains("unknown"));
    assert_eq!(
        report.outcome.incomplete,
        Some(IncompleteReason::OutputLimit)
    );
}
