use super::*;
#[test]
fn details_pages_objects_arrays_text_and_scalars_with_exact_continuations() {
    let source = json!({"snapshot":{"revision":9,"a/b~c":[{"long":"x".repeat(600)},2,3],"text":"界abc界","scalar":true}});
    let mut request = json!({"session_id":Uuid::nil(),"target":"fixture","expected_revision":9,"path":"/a~1b~0c","limit":1});
    let page = details(&source, &request, 1, 4).unwrap();
    assert_eq!(page["total"], 3);
    assert_eq!(page["offset_unit"], "entries");
    assert_eq!(page["entries"][0]["value"]["detail_omitted"], true);
    assert_eq!(page["entries"][0]["value"]["read"]["path"], "/a~1b~0c/0");
    assert_eq!(page["next_read"]["target"], "fixture");
    request = page["next_read"].clone();
    let page = details(&source, &request, 2, 4).unwrap();
    assert_eq!(page["entries"][0]["index"], 1);
    assert_eq!(page["has_more"], false);
    request["path"] = json!("/text");
    request["offset"] = json!(0);
    let page = details(&source, &request, 1, 4).unwrap();
    assert_eq!(page["data"], "界a");
    assert_eq!(page["offset_unit"], "redacted_utf8_bytes");
    let page = details(&source, &page["next_read"], 1, 100).unwrap();
    assert_eq!(page["data"], "bc界");
    assert_eq!(page["has_more"], false);
    request["path"] = json!("/scalar");
    assert_eq!(details(&source, &request, 1, 4).unwrap()["value"], true);
    request["path"] = json!("");
    let page = details(&source, &request, 1, 4).unwrap();
    assert_eq!(page["offset_unit"], "fields");
    assert_eq!(page["total"], 4);
    assert_eq!(page["next_read"]["expected_revision"], 9);
}
#[test]
fn details_refuse_invalid_offsets_and_report_stale_or_missing_sources() {
    let source = json!({"snapshot":{"revision":9,"a":[],"o":{},"s":"界","n":null}});
    for (path, offset) in [("/a", 1), ("/o", 1), ("/s", 1), ("/s", 4), ("/n", 1)] {
        assert!(details(&source, &json!({"path":path,"offset":offset}), 1, 8).is_err());
    }
    let stale = details(
        &source,
        &json!({"expected_revision":8,"target":"remote"}),
        1,
        8,
    )
    .unwrap();
    assert_eq!(stale["status"], "revision_changed");
    assert_eq!(stale["next_read"]["action"], "inspect");
    assert_eq!(stale["next_read"]["target"], "remote");
    assert_eq!(
        details(&source, &json!({"path":"/missing"}), 1, 8).unwrap()["status"],
        "path_unavailable"
    );
    assert!(details(&source, &json!({"path":"x".repeat(4097)}), 1, 8).is_err());
}
#[test]
fn overview_distinguishes_live_activity_historical_messages_and_accumulated_text() {
    let request = json!({"session_id":Uuid::nil(),"target":"remote"});
    let source = json!({"registration":{"state":"running"},"snapshot":{
        "revision":4,"total_messages":22,"message_offset":20,"observation_cursor":8,
        "cleanup":{"blocked":"x".repeat(600)},
        "run":{"run_id":Uuid::nil(),"live_text":"live界text","partial_text":"old text"},
        "messages":[{"role":"assistant","content":"historical assistant"},{"role":"tool","content":"done","tool_calls":[{"name":"read_file"}]}],
        "turns":[{"large":"x".repeat(600)}]
    }});
    let view = overview(&source, &request, 6);
    assert!(view["snapshot"]["run"].get("live_text").is_none());
    assert!(view["snapshot"]["run"].get("partial_text").is_none());
    assert_eq!(view["progress"]["live_text"]["text"], "live");
    assert_eq!(view["progress"]["latest_assistant"]["message_index"], 20);
    assert_eq!(
        view["progress"]["latest_assistant"]["read"]["expected_revision"],
        4
    );
    assert_eq!(
        view["progress"]["last_message"]["tool_names"],
        json!(["read_file"])
    );
    assert_eq!(view["history"]["read"]["offset"], 17);
    assert_eq!(view["events"]["after"], 8);
    assert_eq!(view["snapshot"]["cleanup"]["detail_omitted"], true);
    assert_eq!(view["progress"]["latest_turn"]["read"]["path"], "/turns/0");
    let fallback = overview(
        &json!({"snapshot":{"run":{"partial_text":"earlier","run_id":Uuid::nil()},"messages":[]}}),
        &request,
        4,
    );
    assert_eq!(fallback["progress"]["run_text"]["text"], "earl");
    assert!(fallback["progress"]["latest_assistant_unavailable"].is_string());
    assert!(fallback["progress"].get("live_text").is_none());
}
#[test]
fn detail_descriptions_preserve_types_and_escape_pointer_components() {
    let request = json!({"session_id":Uuid::nil()});
    let snapshot = json!({"revision":2});
    assert_eq!(pointer("/root", "a~/b"), "/root/a~0~1b");
    assert_eq!(described(&json!(3), &request, &snapshot, "/x", 10), 3);
    assert_eq!(
        described(&json!([1, 2, 3]), &request, &snapshot, "/x", 1)["count"],
        3
    );
    assert_eq!(
        described(&json!({"a":1}), &request, &snapshot, "/x", 1)["fields"],
        1
    );
    let text = described(&json!("界界界"), &request, &snapshot, "/x", 6);
    assert_eq!(text["bytes"], 9);
    assert_eq!(text["text"], "界");
    assert_eq!(prefix("界x", 2), "");
    assert_eq!(prefix("界x", 3), "界");
}
