use super::*;
use crate::tools::schema::CompiledSchema;

#[test]
fn all_action_branches_match_serde_without_dispatch() {
    let schema = input_schema();
    let compiled = CompiledSchema::compile(&schema).unwrap();
    let uuid = json!("00112233-4455-4677-8899-aabbccddeeff");
    let samples = json!({"target":"local","session_id":uuid,"command_id":uuid,"incarnation":uuid,"run_id":uuid,
        "expected_revision":1,"offset":0,"after":0,"limit":1,"wait_ms":0,"section":"models","query":"query","workspace":"/workspace","config_path":"/config","task":"task","prompt":"prompt","name":"name"});
    for branch in schema["oneOf"].as_array().unwrap() {
        let action = branch["properties"]["action"]["const"].clone();
        let mut minimal = json!({"action":action});
        for key in branch["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .filter(|k| *k != "action")
        {
            minimal[key] = samples[key].clone();
        }
        compiled.validate(&minimal).unwrap();
        serde_json::from_value::<Action>(minimal.clone()).unwrap();
        for (key, value) in samples.as_object().unwrap() {
            let mut candidate = minimal.clone();
            candidate[key] = value.clone();
            let expected = branch["properties"].get(key).is_some();
            assert_eq!(
                compiled.validate(&candidate).is_ok(),
                expected,
                "{action} / {key}"
            );
            candidate.as_object_mut().unwrap().remove("target");
            if key != "target" {
                assert_eq!(
                    serde_json::from_value::<Action>(candidate.clone())
                        .ok()
                        .is_some_and(|action| crate::tools::action_schema::reject_extra_fields(
                            &candidate, &action
                        )
                        .is_ok()),
                    expected,
                    "serde {action} / {key}"
                );
            }
        }
        for (key, properties) in branch["properties"].as_object().unwrap() {
            if properties["type"]
                .as_array()
                .is_some_and(|v| v.contains(&json!("null")))
            {
                let mut candidate = minimal.clone();
                candidate[key] = Value::Null;
                compiled.validate(&candidate).unwrap();
                serde_json::from_value::<Action>(candidate).unwrap();
            }
        }
    }
    for bad in [
        json!({"action":"inspect","session_id":uuid,"limit":1}),
        json!({"action":"search","query":"  "}),
        json!({"action":"list","limit":129}),
    ] {
        assert!(compiled.validate(&bad).is_err());
    }
    assert!(text(&"é".repeat(2049), 4096).is_err());
    assert!(text(&"é".repeat(2048), 4096).is_ok());
}
#[test]
fn bounded_inspection_keeps_cleanup_and_valid_continuation() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 2048;
    let id = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let request = json!({"action":"inspect","session_id":id,"target":"local"});
    let value = json!({"registration":{"session_id":id,"incarnation":incarnation,"state":"live"},"snapshot":{"session_id":id,"revision":23,"pending_cleanup_run":id,"resources":[{"pending":true}],"run":{"run_id":id,"state":"running","partial_text":"x".repeat(6000)},"messages":[{"content":"x".repeat(6000)}],"turns":[]}});
    let report = output(value.clone(), &ctx, &request).unwrap();
    assert!(report.outcome.incomplete.is_some());
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["snapshot"]["pending_cleanup_run"], json!(id));
    assert_eq!(shown["registration"]["incarnation"], json!(incarnation));
    assert_eq!(shown["snapshot"]["resources"][0]["pending"], true);
    assert_eq!(shown["next_read"]["expected_revision"], 23);
    CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&shown["next_read"])
        .unwrap();
    assert!(report.output.text_fallback().len() <= ctx.max_output_bytes);
    ctx.max_output_bytes = 1;
    let tiny = output(value, &ctx, &request).unwrap();
    assert!(tiny.output.text_fallback().len() <= 1);
    assert!(tiny.outcome.incomplete.is_some());
}
#[test]
fn limited_mutation_never_suggests_repeating_effects() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 1024;
    let id = Uuid::new_v4();
    let request = json!({"action":"submit","command_id":id,"session_id":Uuid::new_v4(),"expected_revision":0,"prompt":"task"});
    let report = output(
        json!({"command_id":id,"result":"x".repeat(9000)}),
        &ctx,
        &request,
    )
    .unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["command_id"], json!(id));
    assert_eq!(shown["next_read"]["action"], "receipt");
    CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&shown["next_read"])
        .unwrap();
}

#[cfg(unix)]
#[test]
fn limited_presentation_preserves_full_receipt_and_deduplication() {
    let root = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let session = Uuid::new_v4();
    let request = json!({"action":"submit","command_id":id,"session_id":session,"expected_revision":0,"prompt":"task"});
    let intent = json!({"version":1,"target":"local","request":request});
    assert!(matches!(
        journal::admit(root.path(), id, &intent).unwrap(),
        journal::Admission::Fresh
    ));
    let value = json!({"command_id":id,"result":{"status":"accepted","large":"x".repeat(9000)}});
    journal::finish(root.path(), id, &value).unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 1024;
    let report = output(value.clone(), &ctx, &request).unwrap();
    assert!(report.outcome.incomplete.is_some());
    assert_eq!(journal::receipt(root.path(), id).unwrap()["result"], value);
    match journal::admit(root.path(), id, &intent).unwrap() {
        journal::Admission::Existing(saved) => assert_eq!(saved, value),
        _ => panic!("must not admit twice"),
    }
    let mut changed = intent.clone();
    changed["request"]["prompt"] = json!("different");
    assert!(journal::admit(root.path(), id, &changed).is_err());
}
