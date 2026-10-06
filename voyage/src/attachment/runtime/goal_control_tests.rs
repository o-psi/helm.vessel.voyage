//! Goal controls wait for brief readers, never replay dispatch or outlive authority.
use super::*;
use crate::attachment::journal::GoalAuthority;
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use voyage_protocol::{
    goals::{GoalAction, GoalLimits, GoalStatus},
    process::RuntimeCommand,
};

#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.0.load(Ordering::SeqCst), "fixture authority revoked");
        Ok(())
    }
}

#[tokio::test]
async fn goal_control_contention_is_bounded_and_rechecks_authority() {
    for outcome in ["released", "revoked", "exhausted"] {
        let root = tempfile::tempdir().unwrap();
        let session = crate::session::Session::new(root.path().into(), "fixture".into());
        let directory = root.path().join("journal");
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        drop(journal);
        let owner = ManagedSessionOwner::open(directory.clone(), session.id)
            .await
            .unwrap();
        owner.initialize_process_commands().await.unwrap();
        let authority = Arc::new(Authority(AtomicBool::new(false)));
        let actor = GoalAuthority {
            installation_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            grant: None,
        };
        owner
            .initialize_command_bindings(actor.principal_id)
            .await
            .unwrap();
        let command = RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60_000) as u64,
            action: GoalAction::Set {
                objective: "Verify fixture output".into(),
                limits: GoalLimits::default(),
                replace_goal_id: None,
                continue_automatically: false,
            },
        };
        let db = Connection::open(directory.join("journal.sqlite3")).unwrap();
        db.execute_batch("BEGIN; SELECT version FROM attachment_schema;")
            .unwrap();
        let worker = owner.clone();
        let check = authority.clone();
        let request = command.clone();
        let binding = actor.clone();
        let before = std::time::Instant::now();
        let task = tokio::spawn(async move {
            worker
                .update_goal_authorized(binding, request, Some(check))
                .await
        });
        tokio::time::sleep(Duration::from_millis(80)).await;
        if task.is_finished() {
            panic!(
                "{outcome}: completed before reader release: {:?}",
                task.await.unwrap()
            );
        }
        if outcome == "released" {
            db.execute_batch("ROLLBACK").unwrap();
        }
        if outcome == "revoked" {
            authority.0.store(true, Ordering::SeqCst);
        }
        let result = tokio::time::timeout(Duration::from_secs(4), task)
            .await
            .unwrap()
            .unwrap();
        if outcome != "released" {
            db.execute_batch("ROLLBACK").unwrap();
        }
        if outcome == "released" {
            let receipt = result.unwrap();
            assert_eq!(receipt["status"], "applied");
            assert_eq!(
                owner.goal().await.unwrap().goal.unwrap().status,
                GoalStatus::Active
            );
            assert_eq!(owner.update_goal(actor, command).await.unwrap(), receipt);
        } else {
            assert!(result.is_err());
            assert!(owner.goal().await.unwrap().goal.is_none());
            if outcome == "revoked" {
                assert!(before.elapsed() < Duration::from_secs(1));
            } else {
                assert!(before.elapsed() >= Duration::from_secs(2));
            }
        }
    }
}
