use super::*;
use crate::tools::schema::CompiledSchema;

#[test]
fn all_action_branches_match_serde_without_dispatch() {
    let schema = input_schema();
    let compiled = CompiledSchema::compile(&schema).unwrap();
    let uuid = json!("00112233-4455-4677-8899-aabbccddeeff");
    let samples = json!({"target":"local","session_id":uuid,"command_id":uuid,"incarnation":uuid,"run_id":uuid,
        "pattern":"error","role":"assistant","path":"","index":0,"expected_revision":1,"offset":0,"after":0,"limit":1,"wait_ms":0,"section":"models","query":"query","workspace":"/workspace","config_path":"/config","task":"task","prompt":"prompt","name":"name"});
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
    assert!(report.outcome.incomplete.is_none());
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["snapshot"]["pending_cleanup_run"], json!(id));
    assert_eq!(shown["registration"]["incarnation"], json!(incarnation));
    assert_eq!(shown["snapshot"]["resources"][0]["pending"], true);
    assert_eq!(shown["details"]["expected_revision"], 23);
    CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&shown["details"])
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

#[test]
fn overview_keeps_progress_and_routes_to_recent_detail() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 4096;
    let id = Uuid::new_v4();
    let request = json!({"action":"inspect","session_id":id,"target":"remote"});
    let value = json!({"registration":{"session_id":id,"incarnation":id,"state":"live"},
        "snapshot":{"session_id":id,"revision":42,"total_messages":1000,"message_offset":999,
        "observation_cursor":77,"inference":{"noise":"x".repeat(100000)},
        "run":{"run_id":id,"state":"running","live_text":"Checking the build.","partial_text":"x".repeat(9000)},
        "messages":[{"role":"assistant","content":"Implementation ready for review.","created_at":"2026-09-10T12:00:00Z"}],
        "decisions":[],"retained_cleanup":{"run_ids":[id],"resources":[]}}});
    let report = output(value, &ctx, &request).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["view"], "overview");
    assert!(shown["snapshot"].get("inference").is_none());
    assert!(shown["snapshot"].get("messages").is_none());
    assert_eq!(
        shown["progress"]["live_text"]["text"],
        "Checking the build."
    );
    assert_eq!(shown["progress"]["latest_assistant"]["read"]["index"], 999);
    assert_eq!(shown["history"]["read"]["offset"], 995);
    assert_eq!(shown["events"]["after"], 77);
    assert_eq!(
        shown["snapshot"]["retained_cleanup"]["run_ids"][0],
        json!(id)
    );
    let schema = CompiledSchema::compile(&input_schema()).unwrap();
    for next in [
        &shown["details"],
        &shown["history"]["read"],
        &shown["events"],
        &shown["progress"]["live_text"]["read"],
        &shown["progress"]["latest_assistant"]["read"],
    ] {
        schema.validate(next).unwrap();
        assert_eq!(next["target"], "remote");
    }
}

#[test]
fn details_pages_fields_and_redacted_unicode_without_losing_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 2048;
    ctx.redactor = crate::tools::Redactor::new(["private-secret".to_string()]).into();
    let id = Uuid::new_v4();
    let text = "α🦀 private-secret ".repeat(200);
    let value = json!({"snapshot":{"revision":3,"cleanup":{"a/b~c":text},"decisions":[{"id":id}]}});
    let request = json!({"action":"details","session_id":id,"target":"remote","path":"/cleanup","limit":1,"expected_revision":3});
    let report = output(value.clone(), &ctx, &request).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    let mut next = shown["entries"][0]["value"]["read"].clone();
    assert_eq!(next["path"], "/cleanup/a~1b~0c");
    let mut assembled = String::new();
    let schema = CompiledSchema::compile(&input_schema()).unwrap();
    loop {
        schema.validate(&next).unwrap();
        let report = output(value.clone(), &ctx, &next).unwrap();
        let page: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
        assert!(report.output.text_fallback().len() <= ctx.max_output_bytes);
        assembled.push_str(page["data"].as_str().unwrap());
        if page["has_more"] == false {
            break;
        }
        next = page["next_read"].clone();
    }
    assert_eq!(assembled, ctx.redactor.redact(&text));
    assert!(!assembled.contains("private-secret"));
    next["expected_revision"] = json!(2);
    let changed = output(value.clone(), &ctx, &next).unwrap();
    let changed: Value = serde_json::from_str(&changed.output.text_fallback()).unwrap();
    assert_eq!(changed["status"], "revision_changed");
    schema.validate(&changed["next_read"]).unwrap();
    let page = output(
        value,
        &ctx,
        &json!({"action":"details","session_id":id,"limit":1}),
    )
    .unwrap();
    let page: Value = serde_json::from_str(&page.output.text_fallback()).unwrap();
    assert_eq!(page["entries"][0]["field"], "cleanup");
    assert_eq!(page["has_more"], true);
    schema.validate(&page["next_read"]).unwrap();
}

#[test]
fn full_message_chunks_preserve_json_and_history_has_escape_from_large_entries() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 1024;
    ctx.redactor = crate::tools::Redactor::new(["private-secret".to_string()]).into();
    let id = Uuid::new_v4();
    let message = json!({"role":"assistant","content":"α🦀 private-secret ".repeat(1000)});
    let source = json!({"session_id":id,"revision":4,"message":message});
    let mut request = json!({"action":"message","session_id":id,"index":7,"expected_revision":4,"target":"remote"});
    let mut assembled = String::new();
    loop {
        let report = output(source.clone(), &ctx, &request).unwrap();
        let page: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
        assert!(report.output.text_fallback().len() <= ctx.max_output_bytes);
        assembled.push_str(page["data"].as_str().unwrap());
        if page["has_more"] == false {
            break;
        }
        request = page["next_read"].clone();
        CompiledSchema::compile(&input_schema())
            .unwrap()
            .validate(&request)
            .unwrap();
    }
    let recovered: Value = serde_json::from_str(&assembled).unwrap();
    assert_eq!(
        recovered["content"],
        ctx.redactor.redact(message["content"].as_str().unwrap())
    );
    let history = output(
        json!({"revision":4,"message_offset":7,"messages":[message]}),
        &ctx,
        &json!({"action":"history","session_id":id,"offset":7,"limit":1,"target":"remote"}),
    )
    .unwrap();
    let shown: Value = serde_json::from_str(&history.output.text_fallback()).unwrap();
    assert_eq!(shown["messages"][0]["read"]["action"], "message");
    assert_eq!(shown["messages"][0]["read"]["index"], 7);
    CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&shown["messages"][0]["read"])
        .unwrap();
}

#[test]
fn overview_labels_older_run_text_when_snapshot_has_only_tool_chatter() {
    let root = tempfile::tempdir().unwrap();
    let ctx = crate::tools::reliability_tests::context(root.path());
    let id = Uuid::new_v4();
    let report = output(json!({"snapshot":{"revision":7,"run":{"run_id":id,"state":"running","partial_text":"Earlier update","live_text":""},"messages":[{"role":"tool","content":"output"}]}}),
        &ctx,&json!({"action":"inspect","session_id":id})).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert!(shown["progress"]["latest_assistant_unavailable"].is_string());
    assert_eq!(shown["progress"]["run_text"]["text"], "Earlier update");
    assert!(shown["progress"].get("live_text").is_none());
    assert_eq!(
        shown["progress"]["run_text"]["read"]["action"],
        "run_output"
    );
}

#[test]
fn history_pages_fit_and_preserve_every_message_boundary() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 2048;
    let id = Uuid::new_v4();
    let source = json!({"revision":9,"message_offset":10,"total_messages":20,"has_more":false,
        "messages":(10..20).map(|i|json!({"message_index":i,"role":"assistant","content":"🦀".repeat(4000)})).collect::<Vec<_>>()});
    let request =
        json!({"action":"history","session_id":id,"offset":10,"limit":10,"target":"remote"});
    let report = output(source, &ctx, &request).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    let count = shown["messages"].as_array().unwrap().len();
    assert!(count > 0 && count < 10);
    assert_eq!(shown["next_read"]["offset"], 10 + count);
    assert_eq!(shown["previous_read"]["offset"], 10 - count);
    assert_eq!(shown["has_more"], true);
    assert!(report.output.text_fallback().len() <= 2048);
    let schema = CompiledSchema::compile(&input_schema()).unwrap();
    for next in [
        &shown["next_read"],
        &shown["previous_read"],
        &shown["messages"][0]["read"],
    ] {
        schema.validate(next).unwrap();
        assert_eq!(next["target"], "remote");
        assert_eq!(next["expected_revision"], 9);
    }
}

#[test]
fn search_output_rewinds_to_withheld_match_and_reports_unsearched_records() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.max_output_bytes = 2400;
    let id = Uuid::new_v4();
    let request = json!({"action":"history_search","session_id":id,"pattern":"error","limit":10,"target":"remote"});
    let source = json!({"revision":9,"offset":0,"next_offset":32,"total_messages":100,
        "entries":[{"kind":"unsearched","message_index":1,"reason":"source_limit"},
            {"kind":"match","message_index":10,"excerpt":"x".repeat(512)},
            {"kind":"match","message_index":20,"excerpt":"x".repeat(512)},
            {"kind":"match","message_index":30,"excerpt":"x".repeat(512)}]});
    let report = output(source, &ctx, &request).unwrap();
    let shown: Value = serde_json::from_str(&report.output.text_fallback()).unwrap();
    assert_eq!(shown["unsearched_messages"], 1);
    assert!(report.outcome.incomplete.is_some());
    assert!(shown["entries"].as_array().unwrap().len() < 4);
    let next = shown["next_read"]["offset"].as_u64().unwrap();
    assert!([10, 20, 30].contains(&next));
    assert_eq!(shown["scanned_messages"], next);
    assert_eq!(shown["next_read"]["pattern"], "error");
    CompiledSchema::compile(&input_schema())
        .unwrap()
        .validate(&shown["next_read"])
        .unwrap();
}
