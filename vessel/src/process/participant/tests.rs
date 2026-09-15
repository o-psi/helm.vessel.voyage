use super::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

struct Fixture {
    supervisor: Supervisor,
    assignment: Assignment,
    grant: ProcessGrant,
}
impl Fixture {
    async fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("participant-{}", Uuid::new_v4()));
        registry::private_directory(&directory).unwrap();
        registry::private_directory(&directory.join("sessions")).unwrap();
        initialize(&directory).unwrap();
        super::super::database::initialize(&directory)
            .await
            .unwrap();
        let supervisor = Supervisor {
            binary: directory.join("must-not-launch"),
            model_slots: Arc::new(Semaphore::new(1)),
            devices: super::super::accounts::device_service(directory.clone()).unwrap(),
            enrollment_workers: Mutex::new(HashMap::new()),
            assignment_locks: Mutex::new(HashMap::new()),
            lifecycle_locks: Mutex::new(HashMap::new()),
            registrations: super::super::database::Registrations::new(directory.clone()),
            directory,
        };
        let id = Uuid::new_v4();
        let parent = Uuid::new_v4();
        let principal = Uuid::new_v4();
        let grant: ProcessGrant = serde_json::from_value(serde_json::json!({
            "grant_id":Uuid::new_v4(),"principal_id":principal,"session_id":parent,
            "workspace":supervisor.directory,"revision":1,"rights":["execute","history","cancel"],
            "expires_at_ms":u64::MAX,"revoked":false,"token_hash":"fixture"
        }))
        .unwrap();
        let request: AssignmentRequest = serde_json::from_value(serde_json::json!({
            "assignment_id":id,"binding_id":Uuid::new_v4(),"binding_revision":1,
            "parent_vessel_id":Uuid::new_v4(),"parent_session_id":parent,"parent_run_id":Uuid::new_v4(),
            "expires_at_ms":u64::MAX,"task":"fixture","context":[],
            "policy":{"access":"read_only","inherit_env":[],"github_enabled":false,"timeout_secs":10,"max_output_bytes":1024,"max_subagents":1}
        })).unwrap();
        let assignment = Assignment {
            observation: AssignmentObservation {
                assignment_id: id,
                participant_vessel_id: Uuid::new_v4(),
                parent_session_id: parent,
                parent_run_id: request.parent_run_id,
                child_session_id: id,
                child_incarnation: None,
                run_id: None,
                state: "prepared".into(),
                cleanup_observed: false,
                result: None,
            },
            request,
            principal_id: principal,
            source_grant: GrantBinding {
                grant_id: grant.grant_id,
                revision: 1,
                principal_id: principal,
            },
            child_grant_id: Uuid::new_v4(),
            start_command_id: Uuid::new_v4(),
            cancel: None,
            cancellation_requested: false,
        };
        let f = Self {
            supervisor,
            assignment,
            grant,
        };
        f.save();
        f
    }
    fn save(&self) {
        store::save_bounded(
            &assignment_path(
                &self.supervisor.directory,
                self.assignment.request.assignment_id,
            ),
            &self.assignment,
            2 * 1024 * 1024,
        )
        .unwrap();
    }
    fn load(&self) -> Assignment {
        store::load_bounded(
            &assignment_path(
                &self.supervisor.directory,
                self.assignment.request.assignment_id,
            ),
            2 * 1024 * 1024,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.supervisor.directory).unwrap();
    }
}

#[tokio::test]
async fn exact_retry_observes_uncertain_reservation_without_dispatch() {
    let f = Fixture::new().await;
    // No binding, child registration or executable exists. Redispatch would fail.
    for _ in 0..2 {
        let value = f
            .supervisor
            .assign(&f.grant, f.assignment.request.clone())
            .await
            .unwrap();
        assert_eq!(value["state"], "cleanup_unknown");
        assert_eq!(value["cleanup_observed"], false);
    }
    let mut changed = f.assignment.request.clone();
    changed.task.push('!');
    assert!(f.supervisor.assign(&f.grant, changed).await.is_err());
    let mut foreign = f.grant.clone();
    foreign.grant_id = Uuid::new_v4();
    assert!(
        f.supervisor
            .assign(&foreign, f.assignment.request.clone())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_survives_polling_and_registration_failure() {
    let f = Fixture::new().await;
    let id = f.assignment.request.assignment_id;
    let value = f
        .supervisor
        .observe_assignment(&f.grant, id, true)
        .await
        .unwrap();
    assert_eq!(value["state"], "cleanup_unknown");
    assert_eq!(value["cleanup_observed"], false);
    assert!(f.load().cancellation_requested);
    f.supervisor.assignment_locks.lock().await.clear(); // Reopen persisted state, not an in-memory flag.
    f.supervisor
        .observe_assignment(&f.grant, id, false)
        .await
        .unwrap();
    assert!(f.load().cancellation_requested);
    assert!(!f.load().observation.cleanup_observed);
}

#[tokio::test]
async fn legacy_revoked_grant_recovers_cancellation_and_tombstone_stays_terminal() {
    let mut f = Fixture::new().await;
    let path = store::grant_path(&f.supervisor.directory, f.assignment.child_grant_id);
    registry::private_directory(path.parent().unwrap().parent().unwrap()).unwrap();
    registry::private_directory(path.parent().unwrap()).unwrap();
    let mut child = f.grant.clone();
    child.revoked = true;
    store::save(&path, &child).unwrap();
    let receipt = assignment_path(&f.supervisor.directory, f.assignment.request.assignment_id);
    let mut legacy = serde_json::to_value(&f.assignment).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("cancellation_requested");
    store::save_bounded(&receipt, &legacy, 2 * 1024 * 1024).unwrap();
    f.supervisor
        .observe_assignment(&f.grant, f.assignment.request.assignment_id, false)
        .await
        .unwrap();
    assert!(f.load().cancellation_requested);
    assert!(store::load::<ProcessGrant>(&path).unwrap().revoked);
    f.assignment.observation.state = "cancelled".into();
    f.assignment.observation.cleanup_observed = true;
    f.save();
    let value = f
        .supervisor
        .assign(&f.grant, f.assignment.request.clone())
        .await
        .unwrap();
    assert_eq!(value["state"], "cancelled");
    assert_eq!(value["cleanup_observed"], true);
}
