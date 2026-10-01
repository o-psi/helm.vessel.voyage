use super::*;
use serde_json::json;

#[test]
fn wake_requires_same_boot_exact_successful_retirement_without_stop() {
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let admission = json!({"session_id":session,"incarnation":incarnation,"boot_id":boot});
    let completion = json!({"session_id":session,"incarnation":incarnation,"boot_id":boot,
        "cleanup_observed":true,"handoff_completed":true,"child_exit_code":0,
        "child_exited_successfully":true,"stop_reason":null});
    let evaluate = |a: &serde_json::Value, c: &serde_json::Value, current_boot, stop_absent| {
        suspension_facts(
            &serde_json::from_value(a.clone()).unwrap(),
            &serde_json::from_value(c.clone()).unwrap(),
            session,
            incarnation,
            current_boot,
            stop_absent,
        )
    };
    assert!(evaluate(&admission, &completion, boot, true));
    assert!(!evaluate(&admission, &completion, Uuid::new_v4(), true));
    assert!(!evaluate(&admission, &completion, boot, false));
    for field in ["session_id", "incarnation", "boot_id"] {
        let mut changed = admission.clone();
        changed[field] = json!(Uuid::new_v4());
        assert!(
            !evaluate(&changed, &completion, boot, true),
            "admission {field}"
        );
        let mut changed = completion.clone();
        changed[field] = json!(Uuid::new_v4());
        assert!(
            !evaluate(&admission, &changed, boot, true),
            "completion {field}"
        );
    }
    for (field, value) in [
        ("cleanup_observed", json!(false)),
        ("handoff_completed", json!(false)),
        ("child_exit_code", json!(null)),
        ("child_exit_code", json!(1)),
        ("child_exited_successfully", json!(false)),
        ("stop_reason", json!("requested")),
        ("stop_reason", json!("authority_changed")),
        ("stop_reason", json!("handoff_failed")),
    ] {
        let mut changed = completion.clone();
        changed[field] = value;
        assert!(
            !evaluate(&admission, &changed, boot, true),
            "completion {field}"
        );
    }
}
