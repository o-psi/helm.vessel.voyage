use super::*;
use serde_json::json;
#[test]
fn secret_text_is_redacted_but_structured_metadata_and_binary_are_refused() {
    let redactor = Redactor::new(["fixture-secret".into()]);
    let mut text = json!({"content":[{"type":"text","text":"before fixture-secret after"}],"description":"fixture-secret"});
    scrub(&mut text, &redactor, "").unwrap();
    assert!(!text.to_string().contains("fixture-secret"));
    assert!(
        text["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("before")
    );
    for mut value in [
        json!({"structuredContent":{"text":"fixture-secret"}}),
        json!({"structured_content":["fixture-secret"]}),
        json!({"content":[{"type":"resource_link","uri":"https://fixture-secret.invalid"}]}),
        json!({"data":STANDARD.encode(b"fixture-secret")}),
        json!({"blob":"fixture-secret"}),
        json!({"fixture-secret":"innocent"}),
    ] {
        assert!(scrub(&mut value, &redactor, "").is_err());
    }
    let split = json!({"content":[{"text":"fixture-"},{"resource":{"text":"secret"}}]});
    assert!(reject_split_text(&split, &redactor).is_err());
    assert!(reject_split_text(&json!({}), &redactor).is_ok());
}
#[test]
fn ingestion_requires_session_scope_and_enforces_limits() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    let value = json!({"content":[{"type":"text","text":"fixture"}],"isError":false});
    assert!(
        ingest(value.clone(), &context)
            .unwrap_err()
            .to_string()
            .contains("session-owned")
    );
    context.artifact_scope = Some(crate::artifacts::Scope {
        directory: root.path().join("artifacts"),
        session: uuid::Uuid::new_v4(),
    });
    let result = ingest(value.clone(), &context).unwrap();
    assert_eq!(result.text_fallback(), "fixture");
    context.max_output_bytes = 1;
    assert!(
        ingest(value, &context)
            .unwrap_err()
            .to_string()
            .contains("max_output_bytes")
    );
}
