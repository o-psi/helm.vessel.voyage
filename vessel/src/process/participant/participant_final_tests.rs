use super::*;
use crate::process::{database, test_support::Fixture};
use serde_json::json;

fn binding(f: &Fixture, g: &ProcessGrant) -> ParticipantBinding {
    ParticipantBinding {
        binding_id: Uuid::new_v4(),
        revision: 1,
        parent_vessel_id: Uuid::new_v4(),
        parent_session_id: g.session_id,
        principal_id: g.principal_id,
        workspace: f.0.clone(),
        config_path: None,
        max_context_bytes: 32768,
        max_assignments: 2,
        expires_at_ms: store::now().unwrap() + 60_000,
        revoked: false,
        cancel_existing: false,
    }
}
fn request(b: &ParticipantBinding) -> AssignmentRequest {
    serde_json::from_value(json!({"assignment_id":Uuid::new_v4(),"binding_id":b.binding_id,
        "binding_revision":b.revision,"parent_vessel_id":b.parent_vessel_id,"parent_session_id":b.parent_session_id,
        "parent_run_id":Uuid::new_v4(),"expires_at_ms":store::now().unwrap()+30_000,"task":"offline bounded task",
        "context":[{"role":"user","content":"untrusted fixture"}],
        "policy":{"access":"read_only","inherit_env":[],"github_enabled":false,"timeout_secs":10,"max_output_bytes":1024,"max_subagents":1}})).unwrap()
}
async fn setup() -> (Fixture, Supervisor, ProcessGrant, ParticipantBinding) {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.session();
    g.rights = vec![
        ProcessRight::Execute,
        ProcessRight::History,
        ProcessRight::Cancel,
    ];
    let b = binding(&f, &g);
    s.participant_admin(VesselCommand::AcceptParticipant {
        command_id: Uuid::new_v4(),
        binding: b.clone(),
    })
    .await
    .unwrap();
    (f, s, g, b)
}

#[tokio::test]
async fn administration_replays_and_rejects_invalid_binding_variants() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let g = f.session();
    let b = binding(&f, &g);
    let command = VesselCommand::AcceptParticipant {
        command_id: Uuid::new_v4(),
        binding: b.clone(),
    };
    let first = s.participant_admin(command.clone()).await.unwrap();
    assert_eq!(first, s.participant_admin(command).await.unwrap());
    for i in 0..11 {
        let mut bad = b.clone();
        bad.binding_id = Uuid::new_v4();
        match i {
            0 => bad.binding_id = Uuid::nil(),
            1 => bad.revision = 2,
            2 => bad.revoked = true,
            3 => bad.cancel_existing = true,
            4 => bad.max_context_bytes = 0,
            5 => bad.max_assignments = 257,
            6 => bad.expires_at_ms = 0,
            7 => bad.expires_at_ms = u64::MAX,
            8 => bad.workspace = PathBuf::from("relative"),
            9 => bad.config_path = Some(f.0.join("missing")),
            _ => bad.binding_id = b.binding_id,
        }
        assert!(
            s.participant_admin(VesselCommand::AcceptParticipant {
                command_id: Uuid::new_v4(),
                binding: bad
            })
            .await
            .is_err()
        );
    }
    let remove = VesselCommand::RemoveParticipant {
        command_id: Uuid::new_v4(),
        binding_id: b.binding_id,
        expected_revision: 1,
        cancel: true,
    };
    let result = s.participant_admin(remove.clone()).await.unwrap();
    assert_eq!(result["disposition"], "cancel");
    assert_eq!(result["revision"], 2);
    let retry = s.participant_admin(remove).await.unwrap();
    assert_eq!(retry["revoked"], true);
    assert_eq!(retry["cancel_existing"], true);
    assert!(
        s.participant_admin(VesselCommand::RemoveParticipant {
            command_id: Uuid::new_v4(),
            binding_id: b.binding_id,
            expected_revision: 1,
            cancel: false
        })
        .await
        .is_err()
    );
    assert!(
        s.participant_admin(VesselCommand::Notifications {
            operation: voyage_protocol::notifications::NotificationOperation::Attention
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn fence_is_positive_no_admission_evidence_and_late_assign_cannot_dispatch() {
    let (f, s, g, b) = setup().await;
    let r = request(&b);
    let result = s.fence_assignment(&g, r.clone()).await.unwrap();
    assert_eq!(result["state"], "cancelled");
    assert_eq!(result["cleanup_observed"], true);
    assert_eq!(s.fence_assignment(&g, r.clone()).await.unwrap(), result);
    assert_eq!(s.assign(&g, r.clone()).await.unwrap(), result);
    assert!(!registry::directory(&f.0, r.assignment_id).exists());
    let saved: Assignment =
        store::load_bounded(&assignment_path(&f.0, r.assignment_id), 2 * 1024 * 1024).unwrap();
    assert!(saved.cancellation_requested);
    assert!(saved.cancel.is_none());
    let mut changed = r.clone();
    changed.task.push('!');
    assert!(s.fence_assignment(&g, changed).await.is_err());
    let mut denied = g.clone();
    denied.rights.clear();
    assert!(s.fence_assignment(&denied, r.clone()).await.is_err());
    denied = g.clone();
    denied.principal_id = Uuid::new_v4();
    assert!(s.fence_assignment(&denied, r.clone()).await.is_err());
    denied = g.clone();
    denied.session_id = Uuid::new_v4();
    assert!(
        s.observe_assignment(&denied, r.assignment_id, false)
            .await
            .is_err()
    );
    let mut mismatch = request(&b);
    mismatch.parent_vessel_id = Uuid::new_v4();
    assert!(s.fence_assignment(&g, mismatch).await.is_err());
    let mut nil = request(&b);
    nil.assignment_id = Uuid::nil();
    assert!(s.fence_assignment(&g, nil).await.is_err());
}

#[tokio::test]
async fn admission_validates_disclosure_and_persists_uncertain_child_exactly_once() {
    let (f, s, g, b) = setup().await;
    let r = request(&b);
    for i in 0..9 {
        let mut bad = r.clone();
        bad.assignment_id = Uuid::new_v4();
        match i {
            0 => bad.parent_run_id = Uuid::nil(),
            1 => bad.binding_revision += 1,
            2 => bad.expires_at_ms = 0,
            3 => bad.expires_at_ms = u64::MAX,
            4 => bad.task = " ".into(),
            5 => bad.task = "x".repeat(8193),
            6 => bad.context[0].role = "system".into(),
            7 => bad.context[0].content = "x".repeat(32769),
            _ => bad.context = vec![bad.context[0].clone(); 65],
        }
        let id = bad.assignment_id;
        assert!(s.assign(&g, bad).await.is_err());
        assert!(!assignment_path(&f.0, id).exists());
    }
    // The absent executable intentionally fails after the durable reservation and child grant.
    assert!(s.assign(&g, r.clone()).await.is_err());
    let path = assignment_path(&f.0, r.assignment_id);
    let saved: Assignment = store::load_bounded(&path, 2 * 1024 * 1024).unwrap();
    assert_eq!(saved.observation.child_session_id, r.assignment_id);
    let child: ProcessGrant = store::load(&store::grant_path(&f.0, saved.child_grant_id)).unwrap();
    assert_eq!(child.principal_id, g.principal_id);
    assert_eq!(child.session_id, r.assignment_id);
    assert_eq!(child.participant_binding.unwrap().binding_id, b.binding_id);
    let replay = s.assign(&g, r.clone()).await.unwrap();
    assert_eq!(replay["cleanup_observed"], false);
    let again: Assignment = store::load_bounded(&path, 2 * 1024 * 1024).unwrap();
    assert_eq!(again.child_grant_id, saved.child_grant_id);
    assert_eq!(again.start_command_id, saved.start_command_id);
    let mut foreign = g.clone();
    foreign.rights.clear();
    assert!(s.assign(&foreign, r).await.is_err());
}

#[tokio::test]
async fn assignment_mutex_capacity_preserves_existing_identity() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let id = Uuid::new_v4();
    let first = s.assignment_lock(id).await.unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &s.assignment_lock(id).await.unwrap()
    ));
    {
        let mut locks = s.assignment_locks.lock().await;
        for _ in 1..4096 {
            locks.insert(
                Uuid::new_v4(),
                std::sync::Arc::new(tokio::sync::Mutex::new(())),
            );
        }
    }
    assert!(s.assignment_lock(Uuid::new_v4()).await.is_err());
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &s.assignment_lock(id).await.unwrap()
    ));
}

#[tokio::test]
async fn observation_persists_terminal_result_before_stop_and_never_repolls() {
    use tokio::net::UnixListener;
    let (f, s, g, b) = setup().await;
    let req = request(&b);
    let id = req.assignment_id;
    s.fence_assignment(&g, req).await.unwrap();
    let path = assignment_path(&f.0, id);
    let mut a: Assignment = store::load_bounded(&path, 2 * 1024 * 1024).unwrap();
    a.observation.cleanup_observed = false;
    a.cancellation_requested = false;
    a.observation.state = "accepted".into();
    store::save_bounded(&path, &a, 2 * 1024 * 1024).unwrap();
    let mut r = f.registration();
    r.session_id = id;
    r.state = ProcessState::Live;
    database::save(&f.0, &r).await.unwrap();
    let dir = registry::directory(&f.0, id);
    registry::private_directory(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("runtime.sock")).unwrap();
    let run = Uuid::new_v4();
    let inc = r.incarnation;
    let server = tokio::spawn(async move {
        for stop in [false, true] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
            if stop {
                assert!(matches!(request.command, RuntimeCommand::Stop));
            } else {
                assert!(matches!(request.command, RuntimeCommand::Snapshot));
            }
            write_frame(&mut stream,&RuntimeResponse{protocol:PROCESS_PROTOCOL,session_id:id,incarnation:inc,resumed_from:None,outcome_unknown:false,
                result:if stop {json!({})} else {json!({"revision":3,"run":{"run_id":run,"state":"completed"},"pending_cleanup_run":null})},error:None}).await.unwrap();
        }
    });
    let observed = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        s.observe_assignment(&g, id, false),
    )
    .await
    .unwrap()
    .unwrap();
    server.await.unwrap();
    assert_eq!(observed["state"], "completed");
    assert_eq!(observed["cleanup_observed"], true);
    assert_eq!(observed["result"]["revision"], 3);
    assert_eq!(s.observe_assignment(&g, id, false).await.unwrap(), observed);
}

#[tokio::test]
async fn active_cancellation_reuses_durable_command_and_revokes_child_before_rpc() {
    use tokio::net::UnixListener;
    let (f, s, g, b) = setup().await;
    let req = request(&b);
    let id = req.assignment_id;
    s.fence_assignment(&g, req).await.unwrap();
    let path = assignment_path(&f.0, id);
    let mut a: Assignment = store::load_bounded(&path, 2 * 1024 * 1024).unwrap();
    a.observation.cleanup_observed = false;
    a.observation.state = "accepted".into();
    a.cancellation_requested = false;
    store::save_bounded(&path, &a, 2 * 1024 * 1024).unwrap();
    let child_path = store::grant_path(&f.0, a.child_grant_id);
    let mut child = g.clone();
    child.grant_id = a.child_grant_id;
    child.session_id = id;
    store::save(&child_path, &child).unwrap();
    let mut r = f.registration();
    r.session_id = id;
    r.state = ProcessState::Live;
    database::save(&f.0, &r).await.unwrap();
    let dir = registry::directory(&f.0, id);
    registry::private_directory(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("runtime.sock")).unwrap();
    let run = Uuid::new_v4();
    let inc = r.incarnation;
    let server = tokio::spawn(async move {
        let mut cancellations = Vec::new();
        for index in 0..4 {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let req: RuntimeRequest = read_frame(&mut stream).await.unwrap();
            if index % 2 == 1 {
                assert!(store::load::<ProcessGrant>(&child_path).unwrap().revoked);
                assert!(matches!(req.command, RuntimeCommand::Cancel { .. }));
                cancellations.push(serde_json::to_value(req.command).unwrap());
            } else {
                assert!(matches!(req.command, RuntimeCommand::Snapshot));
            }
            write_frame(
                &mut stream,
                &RuntimeResponse {
                    protocol: PROCESS_PROTOCOL,
                    session_id: id,
                    incarnation: inc,
                    resumed_from: None,
                    result: json!({"revision":9,"run":{"run_id":run,"state":"running"}}),
                    error: None,
                    outcome_unknown: false,
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(cancellations[0], cancellations[1]);
        assert_eq!(cancellations[0]["expected_revision"], 9);
    });
    for cancel in [true, false] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            s.observe_assignment(&g, id, cancel),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result["state"], "cancellation_requested");
        assert_eq!(result["cleanup_observed"], false);
    }
    server.await.unwrap();
    let saved: Assignment = store::load_bounded(&path, 2 * 1024 * 1024).unwrap();
    assert!(saved.cancellation_requested);
    assert!(saved.cancel.is_some());
}
