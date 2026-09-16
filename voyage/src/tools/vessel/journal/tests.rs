use super::*;
#[test]
fn durable_intents_preserve_uncertain_outcomes_and_exact_wire_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let id = Uuid::new_v4();
    let intent = json!({"target":"local","request":{"action":"create","session_id":Uuid::new_v4(),"task":"fixture"}});
    assert_eq!(receipt(root, id).unwrap()["status"], "not_found");
    assert!(matches!(
        admit(root, id, &intent).unwrap(),
        Admission::Fresh
    ));
    let unknown = receipt(root, id).unwrap();
    assert_eq!(unknown["status"], "outcome_unknown");
    assert_eq!(
        unknown["intent"]["start_command_id"],
        intent["request"]["session_id"]
    );
    assert!(matches!(
        admit(root, id, &intent).unwrap(),
        Admission::Existing(_)
    ));
    assert!(admit(root, id, &json!({"different":true})).is_err());
    wire(root, id, &json!({"op":"submit","prompt":"exact"})).unwrap();
    assert!(wire(root, id, &json!({"op":"submit","prompt":"exact"})).is_err());
    assert!(wire(root, id, &json!({"op":"submit","prompt":"changed"})).is_err());
    finish(root, id, &json!({"status":"accepted"})).unwrap();
    assert_eq!(receipt(root, id).unwrap()["result"]["status"], "accepted");
    let Admission::Existing(value) = admit(root, id, &intent).unwrap() else {
        panic!()
    };
    assert_eq!(value["status"], "accepted");
    assert!(finish(root, id, &json!({"status":"overwritten"})).is_err());
    let page = operations(root, 0, 1).unwrap();
    assert_eq!(page["operations"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], 1);
    assert!(
        operations(root, 10, 1).unwrap()["operations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn corruption_and_unsafe_paths_are_never_treated_as_missing_receipts() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let id = Uuid::new_v4();
    admit(root, id, &json!({"request":{"action":"submit"}})).unwrap();
    let path = root.join(format!("{id}.intent.json"));
    std::fs::write(&path, b"broken").unwrap();
    assert!(receipt(root, id).is_err());
    assert!(admit(root, id, &json!({})).is_err());
    std::fs::remove_file(&path).unwrap();
    let outside = root.join("outside");
    std::fs::write(&outside, b"{}").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(receipt(root, id).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"{}");
}
