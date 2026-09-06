use super::*;

fn setup() -> (tempfile::TempDir, Store, Uuid, Uuid) {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("accounting")).unwrap();
    let project = store.project(temp.path()).unwrap();
    let session = Uuid::new_v4();
    store.bind_session(project, session).unwrap();
    (temp, store, project, session)
}
fn edit(scope: Scope, limit: Option<u64>, revision: u64) -> Change {
    Change {
        operation: Uuid::new_v4(),
        scope,
        expected_revision: revision,
        limit,
        warning: limit.map(|n| n.saturating_sub(1)),
        reason: "operator test allowance".into(),
    }
}
fn request(session: Uuid) -> Attribution {
    Attribution {
        session,
        run: Uuid::new_v4(),
        agent: None,
        provider: "openai-chat".into(),
        model: "fixture".into(),
        purpose: Purpose::Conversation,
    }
}
#[test]
fn shared_project_and_narrower_session_are_both_enforced() {
    let (_temp, mut store, project, session) = setup();
    let other = Uuid::new_v4();
    store.bind_session(project, other).unwrap();
    store
        .configure(&edit(Scope::Project(project), Some(3), 0))
        .unwrap();
    store
        .configure(&edit(Scope::Session(session), Some(1), 0))
        .unwrap();
    store.admit(&request(session)).unwrap();
    assert!(store.admit(&request(session)).is_err());
    store.admit(&request(other)).unwrap();
    store.admit(&request(other)).unwrap();
    assert!(store.admit(&request(other)).is_err());
    assert_eq!(store.inspect(Scope::Project(project)).unwrap().consumed, 3);
}
#[test]
fn exact_edit_retry_is_immutable_and_conflict_is_rejected() {
    let (_temp, mut store, project, _) = setup();
    let mut change = edit(Scope::Project(project), Some(2), 0);
    let receipt = store.configure(&change).unwrap();
    assert_eq!(store.configure(&change).unwrap(), receipt);
    change.limit = Some(20);
    assert!(store.configure(&change).is_err());
    assert_eq!(
        store.inspect(Scope::Project(project)).unwrap().limit,
        Some(2)
    );
}
#[test]
fn restart_retains_unknown_send_and_does_not_refund_it() {
    let (temp, mut store, project, session) = setup();
    store
        .configure(&edit(Scope::Project(project), Some(1), 0))
        .unwrap();
    let permit = store.admit(&request(session)).unwrap();
    drop(store);
    let mut reopened = Store::open(temp.path().join("accounting")).unwrap();
    assert!(reopened.admit(&request(session)).is_err());
    let attempts = reopened.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].id, permit.id);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Unknown);
    assert_eq!(attempts[0].input_tokens, None);
}
#[test]
fn binding_is_immutable_and_canonical_project_identity_survives_reopen() {
    let (temp, mut store, project, session) = setup();
    assert_eq!(store.project(temp.path()).unwrap(), project);
    let other_path = temp.path().join("other");
    std::fs::create_dir(&other_path).unwrap();
    let other = store.project(&other_path).unwrap();
    assert_ne!(project, other);
    assert!(store.bind_session(other, session).is_err());
    assert_eq!(store.session_project(session).unwrap(), project);
}
#[test]
fn availability_is_not_inferred_from_numeric_zero() {
    let (_temp, mut store, project, session) = setup();
    let one = store.admit(&request(session)).unwrap();
    store
        .finish(one.id, AttemptOutcome::Completed, Some(0), None)
        .unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows[0].input_tokens, Some(0));
    assert_eq!(rows[0].output_tokens, None);
    assert!(
        store
            .finish(one.id, AttemptOutcome::Failed, None, None)
            .is_err()
    );
}
#[test]
fn override_requires_reason_and_current_revision_without_resetting_usage() {
    let (_temp, mut store, project, session) = setup();
    let scope = Scope::Project(project);
    store.configure(&edit(scope, Some(1), 0)).unwrap();
    store.admit(&request(session)).unwrap();
    let mut increase = edit(scope, Some(2), 1);
    increase.reason.clear();
    assert!(store.configure(&increase).is_err());
    increase.reason = "operator approves one more attempt".into();
    increase.expected_revision = 0;
    assert!(store.configure(&increase).is_err());
    increase.expected_revision = 1;
    store.configure(&increase).unwrap();
    assert_eq!(store.inspect(scope).unwrap().consumed, 1);
    store.admit(&request(session)).unwrap();
    assert!(store.admit(&request(session)).is_err());
}
#[test]
fn final_permit_is_atomic_across_independent_connections() {
    let (temp, mut store, project, session) = setup();
    store
        .configure(&edit(Scope::Project(project), Some(1), 0))
        .unwrap();
    let directory = temp.path().join("accounting");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let mut connection = Store::open(directory.clone()).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                connection.admit(&request(session)).is_ok()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|ok| *ok)
            .count(),
        1
    );
}
#[test]
fn storage_failure_prevents_admission_and_keeps_counter_unchanged() {
    let (_temp, mut store, project, session) = setup();
    store.connection.execute_batch("CREATE TRIGGER injected BEFORE INSERT ON attempts BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(store.admit(&request(session)).is_err());
    assert_eq!(store.inspect(Scope::Project(project)).unwrap().consumed, 0);
}
#[test]
fn numeric_and_text_bounds_fail_before_mutation() {
    let (_temp, mut store, project, _) = setup();
    let mut change = edit(Scope::Project(project), Some(u64::MAX), 0);
    assert!(store.configure(&change).is_err());
    change.limit = Some(1);
    change.warning = Some(2);
    assert!(store.configure(&change).is_err());
    change.warning = None;
    change.reason = "x".repeat(4097);
    assert!(store.configure(&change).is_err());
    assert_eq!(store.inspect(Scope::Project(project)).unwrap().revision, 0);
}

#[test]
fn default_unlimited_capacity_keeps_exact_counts_and_marks_missing_detail() {
    let (_temp, mut store, project, session) = setup();
    let first = store.admit(&request(session)).unwrap();
    store.connection.execute_batch(&format!("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{}) INSERT INTO attempts(id,project,session,record) SELECT printf('filled-%d',x),project,session,record FROM n,attempts WHERE id='{}';", MAX_DETAILS-1, first.id)).unwrap();
    let permit = store.admit(&request(session)).unwrap();
    assert!(!permit.retained);
    let state = store.inspect(Scope::Project(project)).unwrap();
    assert_eq!(state.consumed, 2);
    assert_eq!(state.omitted_attempt_details, 1);
    store
        .configure(&edit(Scope::Project(project), Some(5), 0))
        .unwrap();
    assert!(store.admit(&request(session)).is_err());
    assert_eq!(store.inspect(Scope::Project(project)).unwrap().consumed, 2);
}
#[test]
fn partial_report_survives_failure_and_rejects_counter_regression() {
    let (_temp, mut store, project, session) = setup();
    let permit = store.admit(&request(session)).unwrap();
    store.report(permit.id, Some(8), None).unwrap();
    assert!(store.report(permit.id, Some(7), Some(1)).is_err());
    store
        .finish_reported(permit.id, AttemptOutcome::Failed)
        .unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows[0].input_tokens, Some(8));
    assert_eq!(rows[0].output_tokens, None);
    assert_eq!(rows[0].outcome, AttemptOutcome::Failed);
}
#[test]
fn compatibility_admission_rechecks_limits_in_its_transaction() {
    let (_temp, mut store, project, session) = setup();
    store.admit_native(&request(session), false).unwrap();
    store
        .configure(&edit(Scope::Project(project), Some(5), 0))
        .unwrap();
    assert!(store.admit_native(&request(session), false).is_err());
    assert_eq!(store.inspect(Scope::Project(project)).unwrap().consumed, 1);
}
#[test]
fn refused_unrelated_and_future_schema_databases_keep_original_bytes() {
    for future in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("accounting");
        let store = Store::open(path.clone()).unwrap();
        store
            .connection
            .execute_batch(if future {
                "UPDATE inference_schema SET version=999"
            } else {
                "DROP TABLE inference_schema"
            })
            .unwrap();
        drop(store);
        let database = path.join("journal.sqlite3");
        let before = std::fs::read(&database).unwrap();
        assert!(Store::open(path).is_err());
        assert_eq!(std::fs::read(&database).unwrap(), before);
    }
}
#[test]
fn oversized_database_is_refused_before_sqlite_mutates_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("accounting");
    drop(Store::open(path.clone()).unwrap());
    let database = path.join("journal.sqlite3");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&database)
        .unwrap();
    file.set_len(MAX_DATABASE as u64 + 4096).unwrap();
    let metadata = file.metadata().unwrap();
    assert!(Store::open(path).is_err());
    assert_eq!(std::fs::metadata(database).unwrap().len(), metadata.len());
}
#[test]
fn corrupt_or_oversized_records_fail_boundedly_without_rewriting_them() {
    let (_temp, mut store, project, session) = setup();
    let permit = store.admit(&request(session)).unwrap();
    store
        .connection
        .execute(
            "UPDATE attempts SET record=?2 WHERE id=?1",
            params![permit.id.to_string(), "x".repeat(MAX_RECORD + 1)],
        )
        .unwrap();
    assert!(store.attempts(Scope::Project(project), 0, 100).is_err());
    assert!(
        store
            .finish_reported(permit.id, AttemptOutcome::Completed)
            .is_err()
    );
    let len: i64 = store
        .connection
        .query_row("SELECT length(record) FROM attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(len as usize, MAX_RECORD + 1);
}
#[cfg(unix)]
#[test]
fn private_store_rejects_symlink_directory_and_database() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    drop(Store::open(real.clone()).unwrap());
    let linked = temp.path().join("linked");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    assert!(Store::open(linked).is_err());
    let other = temp.path().join("other");
    drop(Store::open(other.clone()).unwrap());
    std::fs::remove_file(other.join("journal.sqlite3")).unwrap();
    std::os::unix::fs::symlink(real.join("journal.sqlite3"), other.join("journal.sqlite3"))
        .unwrap();
    assert!(Store::open(other).is_err());
}
#[test]
fn repeated_warning_is_recorded_once_per_scope_revision() {
    let (_temp, mut store, project, session) = setup();
    let scope = Scope::Project(project);
    let mut change = edit(scope, Some(4), 0);
    change.warning = Some(1);
    store.configure(&change).unwrap();
    assert_eq!(store.admit(&request(session)).unwrap().warnings.len(), 1);
    assert!(store.admit(&request(session)).unwrap().warnings.is_empty());
    assert_eq!(
        store
            .audit(scope, 0, 100)
            .unwrap()
            .iter()
            .filter(|row| row.event == "warning")
            .count(),
        1
    );
}
#[test]
fn native_usage_validation_distinguishes_absence_zero_and_malformed_values() {
    use crate::provider::reported_usage;
    use serde_json::json;
    assert_eq!(
        reported_usage(&json!({"input":0}), "input", "output").unwrap(),
        crate::provider::ReportedUsage {
            input_tokens: Some(0),
            output_tokens: None
        }
    );
    for value in [
        json!(true),
        json!(-1),
        json!(1.5),
        json!("3"),
        serde_json::from_str("18446744073709551616").unwrap(),
    ] {
        assert!(reported_usage(&json!({"input":value}), "input", "output").is_err());
    }
}

#[test]
fn existing_binding_never_repairs_missing_limits_into_unlimited_authority() {
    let (_temp, mut store, project, session) = setup();
    store
        .configure(&edit(Scope::Session(session), Some(0), 0))
        .unwrap();
    store
        .connection
        .execute(
            "DELETE FROM limits WHERE scope=?1",
            [Scope::Session(session).key()],
        )
        .unwrap();
    assert!(
        store.bind_session(project, session).is_err(),
        "existing session must not regain unlimited authority"
    );
    assert!(store.admit(&request(session)).is_err());
    let rows: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM limits WHERE scope=?1",
            [Scope::Session(session).key()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        rows, 0,
        "corrupt authority must be preserved for explicit repair, not rewritten"
    );
}
#[test]
fn ordinary_denials_cannot_consume_reserved_configuration_audit_capacity() {
    let (_temp, mut store, project, session) = setup();
    let scope = Scope::Project(project);
    store.configure(&edit(scope, Some(0), 0)).unwrap();
    store.connection.execute_batch(&format!("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<{}) INSERT INTO audit(scope,record) SELECT '{}','{{}}' FROM n;",MAX_AUDIT-101,scope.key())).unwrap();
    for _ in 0..100 {
        assert!(store.admit(&request(session)).is_err());
    }
    let count: i64 = store
        .connection
        .query_row("SELECT count(*) FROM audit", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count,
        MAX_AUDIT - 100,
        "ordinary events consumed the configuration reserve"
    );
    store.configure(&edit(scope, None, 1)).unwrap();
    assert_eq!(store.inspect(scope).unwrap().limit, None);
}

#[test]
fn retained_store_refuses_future_schema_without_counting_or_repairing() {
    let (temp, mut store, project, session) = setup();
    let other = rusqlite::Connection::open(temp.path().join("accounting/journal.sqlite3")).unwrap();
    other
        .execute_batch("UPDATE inference_schema SET version=999")
        .unwrap();
    assert!(
        store.admit(&request(session)).is_err(),
        "old writer must refuse changed schema"
    );
    let consumed: i64 = other
        .query_row(
            "SELECT consumed FROM limits WHERE scope=?1",
            [Scope::Project(project).key()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(consumed, 0);
    assert!(store.bind_session(project, session).is_err());
    assert!(
        store
            .configure(&edit(Scope::Project(project), Some(1), 0))
            .is_err()
    );
}
