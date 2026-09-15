use super::*;
use crate::github::{publication::Action, repository::Object};

fn fixture() -> (tempfile::TempDir, Store, Owner) {
    let root = tempfile::tempdir().unwrap();
    let owner = Owner::new(root.path(), Some(Uuid::new_v4()), Some(Uuid::new_v4())).unwrap();
    let store = Store::open(root.path().join("journal")).unwrap();
    (root, store, owner)
}
fn prepare(store: &mut Store, owner: &Owner, body: &str) -> Operation {
    store
        .prepare(
            Draft {
                object: Object::parse("https://github.com/example/project/issues/17").unwrap(),
                action: Action::Comment { body: body.into() },
            },
            Actor {
                id: 42,
                login: "offline-actor".into(),
            },
            "policy-v1".into(),
            None,
            None,
            owner.clone(),
        )
        .unwrap()
}
fn receipt(operation: &Operation) -> Receipt {
    Receipt {
        id: 19,
        url: format!("{}#issuecomment-19", operation.draft.object.url()),
        evidence: ReceiptEvidence::ApiResponse,
    }
}

#[test]
fn committed_send_is_durable_and_never_replayed() {
    let (root, mut store, owner) = fixture();
    let prepared = prepare(&mut store, &owner, "exact content");
    assert_eq!(prepare(&mut store, &owner, "exact content").id, prepared.id);
    assert!(store.forget_exact(&prepared).is_err());
    assert!(store.publish(&prepared, receipt(&prepared)).is_err());
    let sending = store.begin_send(&prepared).unwrap();
    assert_eq!(sending.state, State::Sending);
    drop(store);
    let mut store = Store::open(root.path().join("journal")).unwrap();
    assert_eq!(
        prepare(&mut store, &owner, "exact content").state,
        State::Sending
    );
    assert!(store.begin_send(&prepared).is_err());
    assert!(store.cancel(prepared.id, &prepared.digest, &owner).is_err());
    assert!(store.forget_exact(&sending).is_err());
    let mut invalid = receipt(&sending);
    invalid.url = "https://github.com/other/project/issues/17#issuecomment-19".into();
    assert!(store.publish(&sending, invalid).is_err());
    assert_eq!(
        store.inspect(sending.id, &owner).unwrap().state,
        State::Sending
    );
    let published = store.publish(&sending, receipt(&sending)).unwrap();
    assert_eq!(published.state, State::Published);
    assert!(store.publish(&sending, receipt(&sending)).is_err());
    assert_eq!(prepare(&mut store, &owner, "exact content").id, prepared.id);
    assert!(store.forget_exact(&sending).is_err());
    store.forget_exact(&published).unwrap();
    assert!(store.inspect(prepared.id, &owner).is_err());
    let audit = store.audit().unwrap();
    assert_eq!(audit.entries.len(), 1);
    assert_eq!(audit.entries[0].former_state, State::Published);
    assert_eq!(audit.entries[0].receipt.as_ref().unwrap().id, 19);
    assert!(store.clear_audit("stale").is_err());
    assert_eq!(store.clear_audit(&audit.digest).unwrap(), 1);
    assert!(store.audit().unwrap().entries.is_empty());
}

#[test]
fn cancellation_disposition_and_exact_snapshot_guards() {
    let (_root, mut store, owner) = fixture();
    let first = prepare(&mut store, &owner, "cancel me");
    assert!(store.cancel(first.id, "wrong digest", &owner).is_err());
    let cancelled = store.cancel_exact(&first).unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    assert!(store.cancel_exact(&first).is_err());
    assert!(store.begin_send(&cancelled).is_err());
    let second = prepare(&mut store, &owner, "cancel me");
    assert_ne!(first.id, second.id);
    assert!(store.dispose_exact(&second, "not sent".into()).is_err());
    let sending = store.begin_send(&second).unwrap();
    assert!(
        store
            .dispose_exact(&second, "stale preview".into())
            .is_err()
    );
    for note in ["".to_owned(), "  ".into(), "x".repeat(1025)] {
        assert!(store.dispose_exact(&sending, note).is_err());
    }
    let disposed = store
        .dispose_exact(&sending, "operator confirmed uncertainty".into())
        .unwrap();
    assert_eq!(disposed.state, State::Disposed);
    assert!(store.begin_send(&disposed).is_err());
    store
        .forget(cancelled.id, &cancelled.digest, &owner)
        .unwrap();
    store.forget_exact(&disposed).unwrap();
    let audit = store.audit().unwrap();
    assert_eq!(audit.entries.len(), 2);
    assert_eq!(
        audit.entries[1].disposition.as_deref(),
        Some("operator confirmed uncertainty")
    );
    let third = prepare(&mut store, &owner, "another uncertain send");
    store.begin_send(&third).unwrap();
    assert_eq!(
        store
            .dispose(third.id, &third.digest, &owner, "not retried".into())
            .unwrap()
            .state,
        State::Disposed
    );
}

#[test]
fn owners_pagination_and_admin_views_are_scoped() {
    let (root, mut store, owner) = fixture();
    let other = Owner::new(root.path(), Some(Uuid::new_v4()), None).unwrap();
    let op = prepare(&mut store, &owner, "one");
    let other_op = prepare(&mut store, &other, "one");
    assert_ne!(op.digest, other_op.digest);
    assert!(store.inspect(op.id, &other).is_err());
    assert!(store.cancel(op.id, &op.digest, &other).is_err());
    assert_eq!(store.list(&owner, 0).unwrap().len(), 1);
    assert!(store.list(&owner, 1).unwrap().is_empty());
    assert_eq!(store.admin_list(0).unwrap().len(), 2);
    assert!(store.admin_list(2).unwrap().is_empty());
    assert_eq!(store.admin_inspect(op.id).unwrap().id, op.id);
    let mut next_run = owner.clone();
    next_run.run = Some(Uuid::new_v4());
    assert_eq!(store.inspect(op.id, &next_run).unwrap().id, op.id);
    assert_ne!(prepare(&mut store, &next_run, "one").digest, op.digest);
    assert!(Owner::new(root.path(), None, Some(Uuid::new_v4())).is_err());
    assert!(Owner::new(&root.path().join("missing"), None, None).is_err());
    for bad in [
        Owner {
            workspace: "invalid".into(),
            ..owner.clone()
        },
        Owner {
            session: Some(Uuid::nil()),
            ..owner.clone()
        },
        Owner {
            run: Some(Uuid::nil()),
            ..owner.clone()
        },
    ] {
        assert!(store.list(&bad, 0).is_err());
    }
}

#[test]
fn decoder_rejects_corruption_in_each_durable_binding() {
    let (_root, mut store, owner) = fixture();
    let operation = prepare(&mut store, &owner, "immutable");
    let valid = serde_json::to_value(&operation).unwrap();
    assert_eq!(decode(&valid.to_string()).unwrap().id, operation.id);
    for (pointer, replacement) in [
        ("/id", serde_json::json!(Uuid::nil())),
        ("/actor/id", serde_json::json!(0)),
        ("/actor/login", serde_json::json!("")),
        ("/owner/workspace", serde_json::json!("bad")),
        ("/digest", serde_json::json!("bad")),
        ("/draft/action/body", serde_json::json!("substitution")),
        ("/expires_at", serde_json::json!(operation.created_at)),
        ("/state", serde_json::json!("published")),
        ("/state", serde_json::json!("disposed")),
        ("/disposition", serde_json::json!("unexpected")),
    ] {
        let mut bad = valid.clone();
        *bad.pointer_mut(pointer).unwrap() = replacement;
        assert!(decode(&bad.to_string()).is_err(), "{pointer}");
    }
    assert!(decode("not json").is_err());
    assert!(decode(&" ".repeat(MAX_RECORD_BYTES + 1)).is_err());
    let mut extra = valid;
    extra["unknown"] = serde_json::json!(true);
    assert!(decode(&extra.to_string()).is_err());
}

#[test]
fn index_schema_and_audit_corruption_fail_closed() {
    for column in ["digest", "state", "data"] {
        let (_root, mut store, owner) = fixture();
        let operation = prepare(&mut store, &owner, "protected");
        store
            .connection
            .execute(&format!("UPDATE operations SET {column}='corrupt'"), [])
            .unwrap();
        assert!(store.inspect(operation.id, &owner).is_err());
        assert!(store.admin_inspect(operation.id).is_err());
        assert!(store.list(&owner, 0).is_err());
        assert!(store.admin_list(0).is_err());
        assert!(store.begin_send(&operation).is_err());
        assert!(store.forget_exact(&operation).is_err());
    }
    let (root, mut store, owner) = fixture();
    let operation = prepare(&mut store, &owner, "audit");
    let cancelled = store.cancel_exact(&operation).unwrap();
    store.forget_exact(&cancelled).unwrap();
    store
        .connection
        .execute("UPDATE audit SET digest='corrupt'", [])
        .unwrap();
    assert!(store.audit().is_err());
    assert!(store.clear_audit("anything").is_err());
    store
        .connection
        .execute("UPDATE github_schema SET version=99", [])
        .unwrap();
    assert!(store.list(&owner, 0).is_err());
    drop(store);
    assert!(Store::open(root.path().join("journal")).is_err());
}

#[test]
fn expiry_and_pull_request_bindings_are_enforced() {
    let (_root, mut store, owner) = fixture();
    let mut operation = prepare(&mut store, &owner, "expired");
    operation.created_at = Utc::now() - chrono::Duration::hours(1);
    operation.expires_at = operation.created_at + chrono::Duration::minutes(15);
    store
        .connection
        .execute(
            "UPDATE operations SET data=?1 WHERE id=?2",
            params![
                serde_json::to_string(&operation).unwrap(),
                operation.id.to_string()
            ],
        )
        .unwrap();
    assert!(store.begin_send(&operation).is_err());
    assert_eq!(
        store.cancel_exact(&operation).unwrap().state,
        State::Cancelled
    );
    let mut draft = operation.draft.clone();
    draft.object = Object::parse("https://github.com/example/project/pull/17").unwrap();
    let head = "a".repeat(40);
    let base = crate::github::context::Base {
        sha: "b".repeat(40),
        repository: 7,
        reference: "main".into(),
    };
    for (head, base) in [
        (None, None),
        (Some(head.clone()), None),
        (None, Some(base.clone())),
    ] {
        assert!(
            store
                .prepare(
                    draft.clone(),
                    operation.actor.clone(),
                    "policy".into(),
                    head,
                    base,
                    owner.clone()
                )
                .is_err()
        );
    }
    let pr = store
        .prepare(
            draft,
            operation.actor,
            "policy".into(),
            Some(head),
            Some(base),
            owner,
        )
        .unwrap();
    assert!(pr.observed_base.is_some());
    assert_ne!(pr.digest, operation.digest);
}
