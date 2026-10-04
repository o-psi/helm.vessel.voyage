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

#[test]
fn uncertain_create_receipt_exposes_exact_frozen_resolution_not_mutable_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let submit_id = Uuid::new_v4();
    let start_id = Uuid::new_v4();
    let intent = json!({"target":"local","request":{"action":"create","session_id":start_id,"task":"fixture"}});
    admit(root, submit_id, &intent).unwrap();
    assert!(receipt(root, submit_id).unwrap()["resolution_request"].is_null());
    let command = VesselCommand::StartSettings {
        command_id: start_id,
        session_id: start_id,
        workspace: "/workspace".into(),
        config_path: None,
        settings: serde_json::from_value(json!({"max_output_tokens":0,"reasoning_effort":null}))
            .unwrap(),
        binding: None,
    };
    wire(root, start_id, &serde_json::to_value(&command).unwrap()).unwrap();
    let result = receipt(root, submit_id).unwrap();
    assert_eq!(result["status"], "outcome_unknown");
    let resolution = &result["resolution_request"];
    assert_eq!(resolution["action"], "resolve_create");
    assert_eq!(resolution["command_id"], start_id.to_string());
    assert_eq!(resolution["settings"]["max_output_tokens"], 0);
    assert!(resolution["settings"]["reasoning_effort"].is_null());
    let mut typed = resolution.clone();
    typed.as_object_mut().unwrap().remove("target");
    let action: Action = serde_json::from_value(typed).unwrap();
    assert!(matches!(action, Action::ResolveCreate { command_id, .. } if command_id == start_id));
}

#[test]
fn remote_frozen_resolution_is_schema_valid_and_preserves_destination_and_resets() {
    for settings in [
        json!({}),
        json!({"max_output_tokens":0}),
        json!({"reasoning_effort":null,"temperature":null}),
        json!({"max_output_tokens":2048}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let submit = Uuid::new_v4();
        let start = Uuid::new_v4();
        admit(
            root,
            submit,
            &json!({"target":"remote-b","request":{"action":"create","session_id":start}}),
        )
        .unwrap();
        let command = VesselCommand::StartSettings {
            command_id: start,
            session_id: start,
            workspace: "/remote".into(),
            config_path: None,
            settings: serde_json::from_value(settings.clone()).unwrap(),
            binding: None,
        };
        wire(root, start, &serde_json::to_value(command).unwrap()).unwrap();
        let request = receipt(root, submit).unwrap()["resolution_request"].clone();
        assert_eq!(request["target"], "remote-b");
        assert_eq!(
            serde_json::from_value::<voyage_protocol::start_settings::StartSettings>(
                request["settings"].clone()
            )
            .unwrap(),
            serde_json::from_value::<voyage_protocol::start_settings::StartSettings>(settings)
                .unwrap()
        );
        crate::tools::schema::CompiledSchema::compile(&input_schema())
            .unwrap()
            .validate(&request)
            .unwrap();
        let mut typed = request.clone();
        typed.as_object_mut().unwrap().remove("target");
        assert!(
            matches!(serde_json::from_value::<Action>(typed).unwrap(),Action::ResolveCreate {command_id,..} if command_id == start)
        );
    }
}

#[test]
fn preupgrade_nullbearing_wire_is_preserved_exactly_in_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let submit = Uuid::new_v4();
    let start = Uuid::new_v4();
    admit(
        root,
        submit,
        &json!({"target":"remote-old","request":{"action":"create","session_id":start}}),
    )
    .unwrap();
    // Literal prior-version wire, not generated through the current serializer.
    let old = json!({"op":"start_settings","command_id":start,"session_id":start,"workspace":"/old",
        "config_path":null,"binding":null,"settings":{"model":null,"max_output_tokens":0,
        "context_window":null,"access_mode":null,"terminal_max_count":null,"terminal_max_unread_bytes":null,
        "subagent_max_concurrency":null,"command_timeout_secs":null,"max_output_bytes":null}});
    wire(root, start, &old).unwrap();
    let typed: VesselCommand = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), old);
    let recovered = receipt(root, submit).unwrap()["resolution_request"].clone();
    assert_eq!(recovered["target"], "remote-old");
    assert_eq!(recovered["settings"], old["settings"]);
    crate::tools::schema::CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&recovered)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(
            &std::fs::read(root.join(format!("{start}.start_settings.command.json"))).unwrap()
        )
        .unwrap(),
        old
    );
    let mut changed = old.clone();
    changed["settings"]["max_output_tokens"] = json!(2048);
    assert_ne!(
        serde_json::to_value(serde_json::from_value::<VesselCommand>(changed).unwrap()).unwrap(),
        old
    );
}
