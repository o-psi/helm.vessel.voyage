use super::*;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
#[test]
fn immutable_artifacts_verify_session_metadata_digest_and_chunk_bounds() {
    let root = tempfile::tempdir().unwrap();
    let session = Uuid::new_v4();
    let mut store = Store::open(root.path(), session).unwrap();
    let id = Uuid::new_v4();
    let metadata = store
        .put(id, "fixture.txt", "text/plain", b"fixture bytes")
        .unwrap();
    assert_eq!(
        store
            .put(id, "fixture.txt", "text/plain", b"fixture bytes")
            .unwrap(),
        metadata
    );
    assert!(
        store
            .put(id, "fixture.txt", "text/plain", b"changed")
            .is_err()
    );
    assert_eq!(store.get(id).unwrap().1, b"fixture bytes");
    let chunk = store.chunk(id, 2, 4).unwrap();
    assert_eq!(
        STANDARD
            .decode(chunk["data_base64"].as_str().unwrap())
            .unwrap(),
        b"xtur"
    );
    assert_eq!(chunk["next_offset"], 6);
    assert_eq!(chunk["eof"], false);
    assert!(store.chunk(id, 100, 4).is_err());
    assert!(store.chunk(id, 0, 0).is_err());
    let mut wrong = metadata.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(store.resolve(&wrong).is_err());
    assert!(Store::open(root.path(), Uuid::new_v4()).is_err());
    for (name, mime) in [
        ("../bad", "text/plain"),
        ("bad\nname", "text/plain"),
        ("ok", "notmime"),
        ("ok", crate::tools::evidence::MIME),
    ] {
        assert!(store.put(Uuid::new_v4(), name, mime, b"x").is_err());
    }
}
#[test]
fn mcp_mixed_resources_are_typed_deduplicated_and_reject_ambiguous_payloads() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path(), Uuid::new_v4()).unwrap();
    let value = json!({"content":[{"type":"text","text":"hello"},{"type":"audio","mimeType":"audio/wav","data":STANDARD.encode(b"synthetic audio")},{"type":"resource","resource":{"uri":"fixture://resource","mimeType":"text/plain","blob":STANDARD.encode(b"embedded")}}],"structuredContent":{"ok":true},"isError":false});
    let first = store.ingest_mcp(&value).unwrap();
    let second = store.ingest_mcp(&value).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    for bad in [
        json!({"content":[],"isError":"wrong"}),
        json!({"content":[],"structuredContent":[]}),
        json!({"content":[{"type":"resource","resource":{"uri":"fixture://x","text":"x","blob":"eA=="}}]}),
        json!({"content":[{"type":"image","mimeType":"audio/wav","data":"eA=="}]}),
        json!({"content":[{"type":"unknown"}]}),
    ] {
        assert!(store.ingest_mcp(&bad).is_err());
    }
}
