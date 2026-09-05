use serde_json::{Value, json};

pub(super) fn input_schema() -> Value {
    // Shared visible properties keep compatible templates useful. Per-action
    // branches narrow nullable union fields and reject every irrelevant key.
    let properties = json!({
        "action":{"type":"string","enum":["create","list","edit","status","block","dependencies","assign","note","progress","evidence","reorder","remove","archive","clear_completed"],"description":"Choose one action; send only its allowed fields."},
        "id":{"type":"string","format":"uuid","description":"Existing todo ID; absent for create/list/clear_completed."},
        "title":{"type":["string","null"],"description":"Required nonempty title for create; optional replacement for edit."},
        "description":{"type":["string","null"],"description":"Create defaults to empty text; edit omission/null preserves it."},
        "priority":{"type":["string","null"],"enum":["low","normal","high","critical",null],"description":"Create defaults to normal; edit omission/null preserves it."},
        "status":{"type":["string","null"],"enum":["pending","in_progress","blocked","completed","cancelled",null],"description":"Required for status; optional list filter. Blocking needs block with blockers first."},
        "order":{"type":["integer","null"],"minimum":i64::MIN,"maximum":i64::MAX,"description":"Required integer for reorder; create omission/null selects next position."},
        "assignees":{"type":"array","items":{"type":"string"},"description":"Create/assign names; omission or [] leaves/sets no assignees."},
        "blockers":{"type":"array","items":{"type":"string"},"description":"Block reasons. Omission or [] clears reasons and reopens blocked work as pending."},
        "add":{"type":"array","items":{"type":"string","format":"uuid"},"description":"Dependencies to add; defaults to []."},
        "remove":{"type":"array","items":{"type":"string","format":"uuid"},"description":"Dependencies to remove; defaults to []."},
        "text":{"type":"string","description":"Required nonempty entry for note/progress/evidence; not accepted by status."},
        "author":{"type":["string","null"],"description":"Optional attribution for note/progress/evidence; defaults to null."},
        "include_archived":{"type":"boolean","description":"List archived records too; defaults to false."}
    });
    let actions: [(&str, &str, &[&str], &[&str], &[&str]); 14] = [
        (
            "create",
            "Create work; description/priority/order/assignees have defaults.",
            &["title"],
            &["description", "priority", "order", "assignees"],
            &["order"],
        ),
        (
            "list",
            "Inspect tasks with optional status/archive filters.",
            &[],
            &["include_archived", "status"],
            &["status"],
        ),
        (
            "edit",
            "Replace supplied fields; omitted/null fields stay unchanged.",
            &["id"],
            &["title", "description", "priority"],
            &["title", "description", "priority"],
        ),
        (
            "status",
            "Change status without adding evidence or blocker reasons.",
            &["id", "status"],
            &[],
            &[],
        ),
        (
            "block",
            "Set blocker reasons, or clear them with [].",
            &["id"],
            &["blockers"],
            &[],
        ),
        (
            "dependencies",
            "Atomically add/remove dependency IDs.",
            &["id"],
            &["add", "remove"],
            &[],
        ),
        (
            "assign",
            "Replace assignees; omitted list clears assignments.",
            &["id"],
            &["assignees"],
            &[],
        ),
        (
            "note",
            "Append background context, not completion evidence.",
            &["id", "text"],
            &["author"],
            &["author"],
        ),
        (
            "progress",
            "Append progress, not completion evidence.",
            &["id", "text"],
            &["author"],
            &["author"],
        ),
        (
            "evidence",
            "Append concrete verification evidence.",
            &["id", "text"],
            &["author"],
            &["author"],
        ),
        (
            "reorder",
            "Set an explicit signed ordering value.",
            &["id", "order"],
            &[],
            &[],
        ),
        (
            "remove",
            "Remove an eligible unreferenced todo.",
            &["id"],
            &[],
            &[],
        ),
        (
            "archive",
            "Archive one completed/cancelled todo.",
            &["id"],
            &[],
            &[],
        ),
        (
            "clear_completed",
            "Archive all completed todos; no per-item ID.",
            &[],
            &[],
            &[],
        ),
    ];
    let variants=actions.into_iter().map(|(action,description,required,optional,nullable)| {
        let mut allowed=serde_json::Map::new();
        allowed.insert("action".into(),json!({"enum":[action]}));
        for key in required.iter().chain(optional.iter()) {
            let constraint=if !nullable.contains(key) {
                properties[*key]["type"].as_array().map(|types|json!({"type":types.iter().find(|kind|**kind!="null").unwrap()})).unwrap_or_else(||json!({}))
            }else{json!({})};
            allowed.insert((*key).into(),constraint);
        }
        let required=std::iter::once("action").chain(required.iter().copied()).collect::<Vec<_>>();
        json!({"type":"object","description":description,"properties":allowed,"required":required,"additionalProperties":false})
    }).collect::<Vec<_>>();
    let id = "00112233-4455-4677-8899-aabbccddeeff";
    json!({"type":"object","required":["action"],"properties":properties,"additionalProperties":false,"oneOf":variants,"examples":[
        {"action":"create","title":"Check release readiness"},
        {"action":"list"},
        {"action":"edit","id":id,"description":"Review the project test report"},
        {"action":"status","id":id,"status":"in_progress"},
        {"action":"block","id":id,"blockers":["Awaiting operator approval"]},
        {"action":"dependencies","id":id,"add":["11223344-5566-4788-99aa-bbccddeeff00"]},
        {"action":"assign","id":id,"assignees":["reviewer"]},
        {"action":"note","id":id,"text":"Release review is still pending"},
        {"action":"progress","id":id,"text":"Reviewed the project test report"},
        {"action":"evidence","id":id,"text":"Example evidence: name the reviewed test report and its actual result here"},
        {"action":"reorder","id":id,"order":1},
        {"action":"remove","id":id},
        {"action":"archive","id":id},
        {"action":"clear_completed"}
    ]})
}
