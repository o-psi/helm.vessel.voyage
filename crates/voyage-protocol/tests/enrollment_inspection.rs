use voyage_protocol::enrollment::inspection::{Audit, MAX_CURSOR, MAX_PAGE, Request};
#[test]
fn inspection_request_defaults_bounds_and_unknown_fields_are_strict() {
    let request: Request = serde_json::from_str("{}").unwrap();
    assert_eq!(request.limit, 100);
    request.validate().unwrap();
    for limit in [0, MAX_PAGE as u16 + 1, u16::MAX] {
        assert!(
            Request {
                limit,
                cursor: None,
                after: 0
            }
            .validate()
            .is_err()
        );
    }
    for cursor in [
        String::new(),
        "x".repeat(MAX_CURSOR + 1),
        "x/y".into(),
        "x\n".into(),
    ] {
        assert!(
            Request {
                limit: 1,
                cursor: Some(cursor),
                after: 0
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Request {
            limit: 1,
            cursor: Some("x.y".into()),
            after: 1
        }
        .validate()
        .is_err()
    );
    assert!(
        Request {
            limit: 1,
            cursor: None,
            after: u64::MAX
        }
        .validate()
        .is_err()
    );
    for value in [
        r#"{"limit":1,"limit":2}"#,
        r#"{"secret":"canary"}"#,
        r#"{"cursor":123}"#,
    ] {
        assert!(serde_json::from_str::<Request>(value).is_err());
    }
}
#[test]
fn audit_has_no_arbitrary_text_projection() {
    let text = r#"{"version":1,"owner_id":"11111111-1111-4111-8111-111111111111","through":1,"events":[{"sequence":1,"machine_id":null,"kind":"invitation_created","time_ms":1}],"next":null,"retention":"all_retained"}"#;
    let page: Audit = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&page).unwrap(), text);
    assert!(
        serde_json::from_str::<Audit>(&text.replace("invitation_created", "secret-canary"))
            .is_err()
    );
    assert!(
        serde_json::from_str::<Audit>(
            &text.replace("\"time_ms\":1", "\"time_ms\":1,\"key\":\"canary\"")
        )
        .is_err()
    );
}
