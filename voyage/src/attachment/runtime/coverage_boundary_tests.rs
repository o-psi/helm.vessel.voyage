//! Durable workflow and retained-resource contracts through the runtime owner.
//! No provider/tool effects or native process-cleanup claims.
use super::*;
use crate::workflow::{Invocation, Scope, secrets::SecretInputs};
use serde_json::json;

async fn accepted() -> (tempfile::TempDir, ManagedSessionOwner, RunOwner) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let session = crate::session::Session::new(root.path().into(), "fixture".into());
    let mut journal = Journal::open(directory.clone()).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session.id)
        .await
        .unwrap();
    owner.initialize_process_commands().await.unwrap();
    let Admission::New(run) = owner
        .admit(TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
            prompt: "fixture".into(),
            parts: vec![],
        })
        .await
        .unwrap()
    else {
        panic!("new run required")
    };
    (root, owner, run)
}
fn invocation() -> Invocation {
    Invocation {
        id: "fixture".into(),
        version: "1.0.0".into(),
        digest: "a".repeat(64),
        scope: Scope::Repository,
        inputs: [("public".into(), json!("visible"))].into_iter().collect(),
    }
}
fn secrets() -> SecretInputs {
    let document:crate::workflow::Document=serde_json::from_value(json!({
        "schema_version":1,"id":"fixture","version":"1.0.0","description":"Fixture","prompt":"{{secret}}", "parameters":{"secret":{"type":"string","secret":true,"required":true}}
    })).unwrap();
    SecretInputs::collect(
        &document,
        vec![("secret".into(), "fixture-private-value".into())],
    )
    .unwrap()
}
#[tokio::test]
async fn workflow_metadata_is_durable_but_secret_authority_is_exact_run_and_volatile() {
    let (_root, owner, mut run) = accepted().await;
    let run_id = run.record().await.unwrap().id;
    let before = owner.snapshot().await.unwrap().revision;
    run.bind_workflow(invocation(), secrets()).await.unwrap();
    let bindings = run.workflow_bindings.as_ref().unwrap();
    assert!(bindings.matches_run(run_id));
    assert!(!bindings.matches_run(Uuid::new_v4()));
    assert!(
        bindings
            .resolve(Uuid::new_v4(), &["secret".into()])
            .is_err()
    );
    let environment = bindings.resolve(run_id, &["secret".into()]).unwrap();
    assert_eq!(
        environment.iter().collect::<Vec<_>>(),
        vec![("HELM_WORKFLOW_SECRET", "fixture-private-value")]
    );
    assert!(!format!("{environment:?}").contains("fixture-private"));
    let saved = owner.snapshot().await.unwrap();
    assert_eq!(saved.revision, before + 1);
    assert_eq!(saved.session.workflow_runs.len(), 1);
    let public = serde_json::to_string(&saved.session).unwrap();
    assert!(!public.contains("fixture-private-value"));
    assert_eq!(saved.session.workflow_runs[0].inputs["public"], "visible");
    assert!(run.bind_workflow(invocation(), secrets()).await.is_err());
    assert_eq!(owner.snapshot().await.unwrap().revision, saved.revision);
    run.fail_before_execution().await.unwrap();
}
#[tokio::test]
async fn running_workflow_refusal_does_not_publish_metadata_or_retain_secret_bindings() {
    let (_root, owner, mut run) = accepted().await;
    run.storage(|store| store.journal.mark_running(&store.guard, store.run_id))
        .await
        .unwrap();
    let before = owner.snapshot().await.unwrap();
    assert!(run.bind_workflow(invocation(), secrets()).await.is_err());
    assert!(run.workflow_bindings.is_none());
    let after = owner.snapshot().await.unwrap();
    assert_eq!(after.revision, before.revision);
    assert!(after.session.workflow_runs.is_empty());
}
#[tokio::test]
async fn resource_recovery_refuses_live_turn_and_never_relabels_foreign_resource_as_clean() {
    let (root, owner, run) = accepted().await;
    owner.initialize_session_resources().await.unwrap();
    let id = Uuid::new_v4();
    owner
        .session_resource_adopt(id, run.record().await.unwrap().id, "root_terminals".into())
        .await
        .unwrap();
    assert!(owner.recover_process_cleanup().await.is_err());
    let resources = owner.session_resources().await.unwrap();
    assert_eq!(resources[0]["state"], "owned");
    assert!(owner.session_resource_closed(Uuid::new_v4()).await.is_err());
    let db = rusqlite::Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute(
        "UPDATE process_session_resources SET state='cleanup_unknown' WHERE id=?1",
        [id.to_string()],
    )
    .unwrap();
    drop(run);
    owner.recover_process_cleanup().await.unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM process_session_resources WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "observed");
    // This invokes the fenced caller contract with a synthetic unknown resource;
    // it is not evidence of a native descendant/process observer.
    assert!(
        owner
            .session_resources()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn resource_attestation_requires_interruption_and_exact_actor_on_reconciliation() {
    let (root, owner, run) = accepted().await;
    owner.initialize_session_resources().await.unwrap();
    let id = Uuid::new_v4();
    owner
        .session_resource_adopt(id, run.record().await.unwrap().id, "browser".into())
        .await
        .unwrap();
    let actor = super::super::local_actor::LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    assert!(owner.attest_session_resource(id, actor).await.is_err());
    let db = rusqlite::Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute(
        "UPDATE process_session_resources SET state='cleanup_unknown' WHERE id=?1",
        [id.to_string()],
    )
    .unwrap();
    owner.attest_session_resource(id, actor).await.unwrap();
    owner.attest_session_resource(id, actor).await.unwrap();
    let different = super::super::local_actor::LocalActor {
        principal_id: Uuid::new_v4(),
        ..actor
    };
    assert!(owner.attest_session_resource(id, different).await.is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM process_resource_attestations WHERE id=?1",
            [id.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM process_session_resources WHERE id=?1",
            [id.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "operator_attested"
    );
    assert!(
        owner
            .session_resources()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
