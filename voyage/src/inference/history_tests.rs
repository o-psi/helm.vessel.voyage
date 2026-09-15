use super::*;
fn query(group_by: GroupBy) -> Query {
    Query {
        from: Utc::now() - chrono::Duration::hours(1),
        until: Utc::now() + chrono::Duration::hours(1),
        group_by,
        detail: None,
        offset: 0,
        limit: 100,
        snapshot: None,
    }
}
#[test]
fn history_groups_usage_outcomes_and_request_bytes_from_private_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path().join("inference")).unwrap();
    let project = store.project(root.path()).unwrap();
    let session = Uuid::new_v4();
    store.bind_session(project, session).unwrap();
    let cancel = CancellationToken::new();
    let empty = store
        .history(Scope::Project(project), query(GroupBy::Model), &cancel)
        .unwrap();
    assert_eq!(empty.totals.retained_attempts, 0);
    let mut permits = vec![];
    for i in 0..4 {
        let permit = store
            .admit(&Attribution {
                session,
                run: Uuid::new_v4(),
                agent: None,
                provider: "offline".into(),
                model: format!("model-{}", i % 2),
                purpose: if i < 2 {
                    Purpose::Conversation
                } else {
                    Purpose::Title
                },
            })
            .unwrap();
        if i < 3 {
            store
                .request_bytes(
                    permit.id,
                    voyage_protocol::provider_attempt::RequestBytes {
                        instructions: 10,
                        schemas: 20,
                        history: 30,
                        envelope: 40,
                        total: 100,
                    },
                )
                .unwrap();
            store
                .finish(
                    permit.id,
                    if i == 2 {
                        AttemptOutcome::Failed
                    } else {
                        AttemptOutcome::Completed
                    },
                    Some(10),
                    if i == 1 { None } else { Some(5) },
                )
                .unwrap();
        }
        permits.push(permit);
    }
    for group_by in [
        GroupBy::Session,
        GroupBy::Model,
        GroupBy::Agent,
        GroupBy::Purpose,
        GroupBy::Day,
    ] {
        let history = store
            .history(Scope::Project(project), query(group_by), &cancel)
            .unwrap();
        assert_eq!(history.totals.retained_attempts, 4);
        assert_eq!(history.totals.completed, 2);
        assert_eq!(history.totals.failed, 1);
        assert_eq!(history.totals.unknown, 1);
        assert_eq!(history.totals.input.reported_sum, Some(30));
        assert_eq!(history.totals.output.reported_sum, Some(10));
        assert_eq!(history.totals.input.missing_attempts, 1);
        assert_eq!(history.totals.output.missing_attempts, 2);
        assert_eq!(history.totals.encoded_request_bytes.reported_sum, Some(300));
        assert_eq!(history.totals.encoded_request_bytes.reported_attempts, 3);
        assert_eq!(history.totals.encoded_request_bytes.missing_attempts, 1);
        assert_eq!(history.range_permits, Some(4));
        assert_eq!(
            history
                .groups
                .iter()
                .map(|g| g.totals.retained_attempts)
                .sum::<u64>(),
            4
        );
        for group in &history.groups {
            let mut detail = query(group_by);
            detail.detail = Some(group.key.clone());
            let rows = store
                .history(Scope::Project(project), detail, &cancel)
                .unwrap();
            assert_eq!(rows.attempts.len() as u64, group.totals.retained_attempts);
        }
        let encoded = display_json(&history).unwrap();
        assert_eq!(
            serde_json::from_str::<History>(&encoded).unwrap().snapshot,
            history.snapshot
        );
    }
    let mut page = query(GroupBy::Model);
    page.limit = 1;
    let first = store
        .history(Scope::Session(session), page.clone(), &cancel)
        .unwrap();
    let continuation = first.next.unwrap();
    page.offset = continuation.offset;
    page.snapshot = Some(continuation.snapshot);
    let second = store
        .history(Scope::Session(session), page.clone(), &cancel)
        .unwrap();
    assert!(second.next.is_none());
    assert_ne!(first.groups[0].key, second.groups[0].key);
    store
        .finish(permits[3].id, AttemptOutcome::Failed, None, None)
        .unwrap();
    assert!(
        store
            .history(Scope::Session(session), page, &cancel)
            .is_err()
    );
    cancel.cancel();
    assert!(
        store
            .history(Scope::Project(project), query(GroupBy::Model), &cancel)
            .is_err()
    );
}
#[test]
fn history_query_validation_and_visible_json_escaping() {
    let good = query(GroupBy::Model);
    good.validate().unwrap();
    let mut variants = vec![];
    let mut bad = good.clone();
    bad.until = bad.from;
    variants.push(bad);
    let mut bad = good.clone();
    bad.limit = 0;
    variants.push(bad);
    let mut bad = good.clone();
    bad.limit = 101;
    variants.push(bad);
    let mut bad = good.clone();
    bad.offset = 1;
    variants.push(bad);
    let mut bad = good.clone();
    bad.snapshot = Some("g".repeat(64));
    variants.push(bad);
    let mut bad = good.clone();
    bad.detail = Some(GroupKey::Day {
        utc: "2026-01-01".into(),
    });
    variants.push(bad);
    for bad in variants {
        assert!(bad.validate().is_err());
    }
    let value = serde_json::json!({"\u{061c}":"\u{2028}\u{200b}\u{2066}\u{feff}hello"});
    let text = display_json(&value).unwrap();
    assert!(text.contains("\\u061c"));
    assert!(!text.contains('\u{2028}'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        value
    );
}
