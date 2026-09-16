use super::*;
use serde_json::json;

#[test]
fn cursor_progress_is_required_only_for_nonterminal_pages() {
    assert_eq!(
        next_cursor(&json!({"next_after":7,"has_more":false}), 7).unwrap(),
        7
    );
    assert_eq!(
        next_cursor(&json!({"next_after":8,"has_more":true}), 7).unwrap(),
        8
    );
    for page in [
        json!({}),
        json!({"next_after":7}),
        json!({"next_after":7,"has_more":true}),
        json!({"next_after":6,"has_more":false}),
        json!({"next_after":8,"has_more":"true"}),
        json!({"next_after":-1,"has_more":false}),
    ] {
        assert!(next_cursor(&page, 7).is_err());
    }
}

#[test]
fn typed_destination_file_is_bounded_and_error_does_not_quote_secrets() {
    let value = json!({"id":Uuid::new_v4(),"recipient_grant_id":Uuid::new_v4(),"recipient_principal_id":Uuid::new_v4(),"recipient_grant_revision":1,"source_vessel_id":Uuid::new_v4(),"source_session_id":Uuid::new_v4(),"event_kinds":[],"expires_at_ms":1000,"notification_ttl_ms":100,"quiet_hours_utc":null});
    let bytes = serde_json::to_vec(&value).unwrap();
    let destination = decode_destination(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&destination).unwrap(), value);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("destination.json");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(load_destination(&path).unwrap(), destination);
    assert!(load_destination(temp.path()).is_err());
    assert!(load_destination(&temp.path().join("absent")).is_err());
    for bytes in [
        b"secret-fixture-not-json".to_vec(),
        vec![b'x'; 65_537],
        b"{}".to_vec(),
    ] {
        let error = decode_destination(&bytes).unwrap_err();
        assert!(!error.to_string().contains("secret-fixture"));
    }
    std::fs::write(path.clone(), vec![b'x'; 65_537]).unwrap();
    assert!(load_destination(&path).is_err());
}
