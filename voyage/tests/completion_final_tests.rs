//! Public coordinator journeys; automatically discovered integration target.
//! No providers, environment writes, or workspace-global persistence.
use std::collections::BTreeSet;
use uuid::Uuid;
use voyage::{
    completion::{
        DispositionKind, FinalOutcome, Obligation, UnresolvedReason,
        runtime::{Coordinator, Review, RunHandle},
    },
    subagent::AgentTreeStore,
    todo::{EntryKind, NewTodo, Priority, TodoId, TodoScope, TodoStatus, TodoStore},
};

struct Fixture {
    _root: tempfile::TempDir,
    coordinator: Coordinator,
    run: RunHandle,
    todos: TodoStore,
    agents: AgentTreeStore,
    session: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(root.path().join("coord"), root.path()).unwrap();
        let session = Uuid::new_v4();
        let run = RunHandle::create(coordinator.clone(), session, Uuid::new_v4())
            .await
            .unwrap();
        let todos = TodoStore::new(
            root.path().join("todos.json"),
            TodoScope::session(root.path().into(), session),
        )
        .with_coordinator(coordinator.clone());
        let agents = AgentTreeStore::new(root.path().join("agents.json"))
            .with_coordinator(coordinator.clone());
        Self {
            _root: root,
            coordinator,
            run,
            todos,
            agents,
            session,
        }
    }
    async fn todo(&self, title: &str) -> TodoId {
        self.todos
            .create(NewTodo {
                title: title.into(),
                description: "isolated coordinator fixture".into(),
                priority: Priority::Normal,
                order: None,
                assignees: BTreeSet::new(),
            })
            .await
            .unwrap()
            .id
    }
    async fn own(&self, id: TodoId) {
        self.run.register(Obligation::Todo(id)).await.unwrap();
    }
    async fn completed(&self, id: TodoId) {
        self.todos
            .set_status(id, TodoStatus::InProgress)
            .await
            .unwrap();
        self.todos
            .append_note(
                id,
                EntryKind::Evidence,
                "Fixture assertions observed expected durable state".into(),
                None,
            )
            .await
            .unwrap();
        self.todos
            .set_status(id, TodoStatus::Completed)
            .await
            .unwrap();
    }
    async fn review(&self, id: TodoId, disposition: DispositionKind) -> anyhow::Result<()> {
        let snapshot = self.run.snapshot(&self.todos, &self.agents, 100).await?;
        self.run
            .account(
                &self.todos,
                &self.agents,
                Obligation::Todo(id),
                Review {
                    revision: snapshot.revision,
                    fingerprint: snapshot.fingerprint,
                    disposition,
                    reason: "Reviewed exact fixture state and user-visible impact".into(),
                },
            )
            .await
    }
}

#[tokio::test]
async fn empty_run_seals_and_resumes_without_losing_identity() {
    let f = Fixture::new().await;
    let initial = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(initial.ready());
    assert_eq!(initial.total, 0);
    assert_eq!(initial.unresolved.len(), 0);
    assert!(f.run.decision().await.unwrap().is_none());
    let decision = f
        .run
        .readiness_lease(&f.todos, &f.agents, 10)
        .await
        .unwrap()
        .seal(FinalOutcome::Completed, None)
        .await
        .unwrap();
    assert_eq!(decision.outcome, FinalOutcome::Completed);
    assert!(decision.readiness.ready());
    let resumed = RunHandle::resume(f.coordinator.clone(), f.session, f.run.run_id())
        .await
        .unwrap();
    assert_eq!(resumed.reference(), f.run.reference());
    assert_eq!(resumed.decision().await.unwrap(), Some(decision.clone()));
    resumed
        .validate_final_decision(&f.todos, &f.agents)
        .await
        .unwrap();
    assert!(
        resumed
            .register(Obligation::Todo(TodoId(Uuid::new_v4())))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resume_refuses_missing_wrong_session_and_duplicate_creation() {
    let f = Fixture::new().await;
    assert!(
        RunHandle::resume(f.coordinator.clone(), f.session, Uuid::new_v4())
            .await
            .is_err()
    );
    assert!(
        RunHandle::resume(f.coordinator.clone(), Uuid::new_v4(), f.run.run_id())
            .await
            .is_err()
    );
    assert!(
        RunHandle::create(f.coordinator.clone(), f.session, f.run.run_id())
            .await
            .is_err()
    );
    assert!(f.run.decision().await.unwrap().is_none());
    assert!(
        f.run
            .snapshot(&f.todos, &f.agents, 1)
            .await
            .unwrap()
            .ready()
    );
}

#[tokio::test]
async fn registration_is_idempotent_and_missing_owned_work_remains_visible() {
    let f = Fixture::new().await;
    let id = TodoId(Uuid::new_v4());
    f.own(id).await;
    let first = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    f.own(id).await;
    let second = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert_eq!(first.revision, second.revision);
    assert_eq!(second.total, 1);
    assert_eq!(second.unresolved.len(), 1);
    assert_eq!(second.unresolved[0].reason, UnresolvedReason::MissingRecord);
    assert!(!second.ready());
    assert!(f.run.owns(Obligation::Todo(id)).await.unwrap());
    assert!(
        f.run
            .read_owned(&f.todos, &f.agents, Obligation::Todo(id))
            .await
            .is_err()
    );
    assert!(
        f.run
            .readiness_lease(&f.todos, &f.agents, 1)
            .await
            .unwrap()
            .seal(FinalOutcome::Completed, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn owned_and_unowned_records_are_not_confused() {
    let f = Fixture::new().await;
    let owned = f.todo("owned").await;
    let other = f.todo("unowned").await;
    f.own(owned).await;
    assert!(!f.run.owns(Obligation::Todo(other)).await.unwrap());
    assert!(
        f.run
            .read_owned(&f.todos, &f.agents, Obligation::Todo(other))
            .await
            .is_err()
    );
    let record = f
        .run
        .read_owned(&f.todos, &f.agents, Obligation::Todo(owned))
        .await
        .unwrap();
    assert_eq!(record["title"], "owned");
    assert_eq!(record["status"], "pending");
    let snapshot = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert_eq!(snapshot.total, 1);
    assert_eq!(snapshot.incomplete, 1);
}

#[tokio::test]
async fn adoption_requires_existing_record_and_current_revision() {
    let f = Fixture::new().await;
    let id = f.todo("adopt existing work").await;
    let initial = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(
        f.run
            .adopt_existing(
                &f.todos,
                &f.agents,
                Obligation::Todo(id),
                initial.revision + 1
            )
            .await
            .is_err()
    );
    assert!(
        f.run
            .adopt_existing(
                &f.todos,
                &f.agents,
                Obligation::Todo(TodoId(Uuid::new_v4())),
                initial.revision
            )
            .await
            .is_err()
    );
    f.run
        .adopt_existing(&f.todos, &f.agents, Obligation::Todo(id), initial.revision)
        .await
        .unwrap();
    assert!(f.run.owns(Obligation::Todo(id)).await.unwrap());
    assert_eq!(
        f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap().total,
        1
    );
}

#[tokio::test]
async fn automatic_status_accounting_does_not_make_legacy_evidence_review_valid() {
    let f = Fixture::new().await;
    let id = f.todo("needs actual evidence").await;
    f.own(id).await;
    f.todos
        .set_status(id, TodoStatus::InProgress)
        .await
        .unwrap();
    f.todos.set_status(id, TodoStatus::Completed).await.unwrap();
    let no_evidence = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(no_evidence.ready());
    assert_eq!(no_evidence.completed, 1);
    assert!(
        f.review(id, DispositionKind::CompletedWithEvidence)
            .await
            .is_err()
    );
    f.todos
        .append_note(
            id,
            EntryKind::Evidence,
            "Observed fixture response".into(),
            None,
        )
        .await
        .unwrap();
    let evidenced = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(evidenced.ready());
    assert_eq!(evidenced.accounted, 1);
    assert_ne!(no_evidence.fingerprint, evidenced.fingerprint);
}

#[tokio::test]
async fn completion_with_evidence_survives_archiving() {
    let f = Fixture::new().await;
    let id = f.todo("finished fixture").await;
    f.own(id).await;
    f.completed(id).await;
    f.todos.archive(id).await.unwrap();
    let state = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert_eq!(state.total, 1);
    assert!(state.ready());
    let value = f
        .run
        .read_owned(&f.todos, &f.agents, Obligation::Todo(id))
        .await
        .unwrap();
    assert!(!value["archived_at"].is_null());
    let decision = f
        .run
        .readiness_lease(&f.todos, &f.agents, 0)
        .await
        .unwrap()
        .seal(FinalOutcome::Completed, None)
        .await
        .unwrap();
    f.run
        .validate_final_decision(&f.todos, &f.agents)
        .await
        .unwrap();
}

#[tokio::test]
async fn clearing_completed_items_keeps_owned_obligations() {
    let f = Fixture::new().await;
    for title in ["one", "two"] {
        let id = f.todo(title).await;
        f.own(id).await;
        f.completed(id).await;
    }
    assert_eq!(f.todos.clear_completed().await.unwrap(), 2);
    let state = f.run.snapshot(&f.todos, &f.agents, 0).await.unwrap();
    assert_eq!(state.total, 2);
    assert_eq!(state.accounted, 2);
    assert!(state.ready());
}

#[tokio::test]
async fn presentation_limits_do_not_hide_unresolved_counts() {
    let f = Fixture::new().await;
    for _ in 0..3 {
        f.own(TodoId(Uuid::new_v4())).await;
    }
    let none = f.run.snapshot(&f.todos, &f.agents, 0).await.unwrap();
    let one = f.run.snapshot(&f.todos, &f.agents, 1).await.unwrap();
    let all = f.run.snapshot(&f.todos, &f.agents, 100).await.unwrap();
    assert_eq!(none.unresolved.len(), 0);
    assert_eq!(one.unresolved.len(), 1);
    assert_eq!(all.unresolved.len(), 3);
    assert_eq!(none.total, 3);
    assert_eq!(none.fingerprint, all.fingerprint);
    assert_eq!(one.fingerprint, all.fingerprint);
    assert_eq!(none.omitted_unresolved, 3);
    assert_eq!(one.omitted_unresolved, 2);
    assert_eq!(all.omitted_unresolved, 0);
    assert!(!none.ready());
}

#[tokio::test]
async fn cancelled_work_is_accounted_incomplete_with_optional_impact_review() {
    let f = Fixture::new().await;
    let id = f.todo("no longer needed").await;
    f.own(id).await;
    f.todos.set_status(id, TodoStatus::Cancelled).await.unwrap();
    assert_eq!(
        f.run
            .snapshot(&f.todos, &f.agents, 10)
            .await
            .unwrap()
            .incomplete,
        1
    );
    f.review(id, DispositionKind::CancelledWithReason)
        .await
        .unwrap();
    let state = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(state.ready());
    assert_eq!(state.accounted, 1);
    let decision = f
        .run
        .readiness_lease(&f.todos, &f.agents, 10)
        .await
        .unwrap()
        .seal(
            FinalOutcome::Incomplete,
            Some("Unfinished work impact was recorded".into()),
        )
        .await
        .unwrap();
    assert_eq!(decision.outcome, FinalOutcome::Incomplete);
}

#[tokio::test]
async fn blocked_work_keeps_impact_and_rejects_completed_disposition() {
    let f = Fixture::new().await;
    let id = f.todo("await parent measurement").await;
    f.own(id).await;
    f.todos
        .set_blockers(id, vec!["parent owns compiler execution".into()])
        .await
        .unwrap();
    assert!(
        f.review(id, DispositionKind::CompletedWithEvidence)
            .await
            .is_err()
    );
    f.review(id, DispositionKind::BlockedWithImpact)
        .await
        .unwrap();
    assert!(
        f.run
            .snapshot(&f.todos, &f.agents, 10)
            .await
            .unwrap()
            .ready()
    );
    let decision = f
        .run
        .readiness_lease(&f.todos, &f.agents, 10)
        .await
        .unwrap()
        .seal(
            FinalOutcome::Incomplete,
            Some("Unfinished work impact was recorded".into()),
        )
        .await
        .unwrap();
    assert_eq!(decision.readiness.accounted, 1);
}

#[tokio::test]
async fn stale_record_fingerprint_and_ledger_revision_are_independent_fences() {
    let f = Fixture::new().await;
    let id = f.todo("review exact state").await;
    f.own(id).await;
    f.todos.set_status(id, TodoStatus::Cancelled).await.unwrap();
    let old = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    f.todos
        .append_note(id, EntryKind::Note, "changed after review".into(), None)
        .await
        .unwrap();
    let review = Review {
        revision: old.revision,
        fingerprint: old.fingerprint,
        disposition: DispositionKind::CancelledWithReason,
        reason: "reviewed cancellation".into(),
    };
    assert!(
        f.run
            .account(&f.todos, &f.agents, Obligation::Todo(id), review)
            .await
            .is_err()
    );
    let current = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    let review = Review {
        revision: current.revision + 1,
        fingerprint: current.fingerprint,
        disposition: DispositionKind::CancelledWithReason,
        reason: "reviewed cancellation".into(),
    };
    assert!(
        f.run
            .account(&f.todos, &f.agents, Obligation::Todo(id), review)
            .await
            .is_err()
    );
    f.review(id, DispositionKind::CancelledWithReason)
        .await
        .unwrap();
}

#[tokio::test]
async fn changed_records_are_accounted_automatically_and_can_be_reviewed_again() {
    let f = Fixture::new().await;
    let id = f.todo("cancelled fixture").await;
    f.own(id).await;
    f.todos.set_status(id, TodoStatus::Cancelled).await.unwrap();
    f.review(id, DispositionKind::CancelledWithReason)
        .await
        .unwrap();
    f.todos
        .append_note(id, EntryKind::Note, "new impact".into(), None)
        .await
        .unwrap();
    let state = f.run.snapshot(&f.todos, &f.agents, 10).await.unwrap();
    assert!(state.ready());
    assert_eq!(state.incomplete, 1);
    f.review(id, DispositionKind::CancelledWithReason)
        .await
        .unwrap();
    assert!(
        f.run
            .snapshot(&f.todos, &f.agents, 10)
            .await
            .unwrap()
            .ready()
    );
}

#[tokio::test]
async fn final_validation_reads_the_saved_decision_and_refuses_later_record_changes() {
    let f = Fixture::new().await;
    let id = f.todo("verified output").await;
    f.own(id).await;
    f.completed(id).await;
    let decision = f
        .run
        .readiness_lease(&f.todos, &f.agents, 10)
        .await
        .unwrap()
        .seal(FinalOutcome::Completed, None)
        .await
        .unwrap();
    assert_eq!(
        f.run
            .validate_final_decision(&f.todos, &f.agents)
            .await
            .unwrap(),
        decision
    );
    f.todos
        .append_note(id, EntryKind::Note, "changed after sealing".into(), None)
        .await
        .unwrap();
    assert!(
        f.run
            .validate_final_decision(&f.todos, &f.agents)
            .await
            .is_err()
    );
    assert_eq!(f.run.decision().await.unwrap(), Some(decision));
}

#[tokio::test]
async fn stores_without_the_same_coordinator_are_not_authoritative() {
    let f = Fixture::new().await;
    let unrelated = tempfile::tempdir().unwrap();
    let todos = TodoStore::new(
        unrelated.path().join("todos"),
        TodoScope::workspace(unrelated.path().into()),
    );
    let agents = AgentTreeStore::new(unrelated.path().join("agents"));
    assert!(f.run.snapshot(&todos, &f.agents, 10).await.is_err());
    assert!(f.run.snapshot(&f.todos, &agents, 10).await.is_err());
    let other = Coordinator::open(unrelated.path().join("coord"), unrelated.path()).unwrap();
    let todos = todos.with_coordinator(other);
    assert!(f.run.snapshot(&todos, &f.agents, 10).await.is_err());
}

#[tokio::test]
async fn interrupted_and_failed_outcomes_require_reasons_but_do_not_erase_work() {
    for outcome in [FinalOutcome::Incomplete, FinalOutcome::Interrupted] {
        let f = Fixture::new().await;
        f.own(f.todo("unfinished at boundary").await).await;
        assert!(
            f.run
                .readiness_lease(&f.todos, &f.agents, 10)
                .await
                .unwrap()
                .seal(outcome.clone(), None)
                .await
                .is_err()
        );
        assert!(f.run.decision().await.unwrap().is_none());
        let decision = f
            .run
            .readiness_lease(&f.todos, &f.agents, 0)
            .await
            .unwrap()
            .seal(
                outcome.clone(),
                Some("provider-free fixture interruption; task remains unfinished".into()),
            )
            .await
            .unwrap();
        assert_eq!(decision.outcome, outcome);
        assert_eq!(decision.readiness.total, 1);
        assert_eq!(decision.readiness.incomplete, 1);
        assert!(decision.readiness.ready());
        assert!(decision.reason.unwrap().contains("unfinished"));
    }
}

#[test]
fn coordinator_refuses_non_directories_and_missing_workspace() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "not a directory").unwrap();
    assert!(Coordinator::open(root.path().join("coord"), &root.path().join("file")).is_err());
    assert!(Coordinator::open(root.path().join("coord"), &root.path().join("missing")).is_err());
    assert!(Coordinator::open(root.path().join("file"), root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn coordinator_refuses_public_permissions_and_symlink_aliases() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let public = root.path().join("public");
    std::fs::create_dir(&public).unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Coordinator::open(public, root.path()).is_err());
    let real = root.path().join("real");
    Coordinator::open(real.clone(), root.path()).unwrap();
    std::os::unix::fs::symlink(real, root.path().join("alias")).unwrap();
    assert!(Coordinator::open(root.path().join("alias"), root.path()).is_err());
}
