use super::*;
async fn read(state: &Arc<State>, command: &RuntimeCommand) -> Result<Value> {
    inspect(&state.owner, &state.registration, command, &state.directory).await
}
#[tokio::test]
async fn saved_observation_never_implies_cleanup_or_wakes_execution() {
    let (_root, state) = crate::server::tests::fixture().await;
    let health = read(&state, &RuntimeCommand::Health).await.unwrap();
    assert_eq!(health["suspended"], true);
    assert!(health["pid"].is_null());
    let snapshot = read(&state, &RuntimeCommand::Snapshot).await.unwrap();
    assert_eq!(snapshot["recovery_pending"], true);
    assert_eq!(snapshot["observation"], "saved");
    assert_eq!(snapshot["suspended"], false);
    let history = read(
        &state,
        &RuntimeCommand::History {
            offset: 0,
            limit: 10,
            expected_revision: Some(0),
        },
    )
    .await
    .unwrap();
    assert!(history["messages"].as_array().unwrap().is_empty());
    let receipt = read(
        &state,
        &RuntimeCommand::Receipt {
            command_id: uuid::Uuid::new_v4(),
        },
    )
    .await
    .unwrap();
    assert_eq!(receipt["status"], "unknown");
    assert!(
        read(
            &state,
            &RuntimeCommand::Events {
                after: 0,
                limit: 10,
                wait_ms: 10001
            }
        )
        .await
        .is_err()
    );
    assert!(
        read(&state, &RuntimeCommand::Decisions)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(state.active.lock().await.is_none());
}
#[tokio::test]
async fn retirement_proof_is_bound_to_incarnation_and_cleanup() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, state) = crate::server::tests::fixture().await;
    let path = state.directory.join("stopped.json");
    assert!(check_retired(&state.directory, &state.registration, true, false).is_err());
    let evidence = json!({"session_id":state.registration.session_id,"incarnation":state.registration.incarnation,"cleanup_observed":true,"suspended":true});
    for field in ["session_id", "incarnation", "cleanup_observed", "suspended"] {
        let mut bad = evidence.clone();
        bad[field] = if field.ends_with("id") || field == "incarnation" {
            json!(uuid::Uuid::new_v4())
        } else {
            json!(false)
        };
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            check_retired(&state.directory, &state.registration, true, false).is_err(),
            "{field}"
        );
    }
    std::fs::write(path, serde_json::to_vec(&evidence).unwrap()).unwrap();
    check_retired(&state.directory, &state.registration, true, false).unwrap();
}

#[tokio::test]
async fn saved_reads_ignore_missing_cleanup_but_never_relinquished_authority() {
    let (_root, state) = crate::server::tests::fixture().await;
    let mut registration = state.registration.clone();
    std::fs::write(state.directory.join("runtime.sock"), b"stale endpoint").unwrap();
    check_retired(&state.directory, &registration, true, true).unwrap();
    assert!(check_retired(&state.directory, &registration, false, false).is_err());
    registration.state = voyage_protocol::process::ProcessState::Relinquished;
    assert!(check_retired(&state.directory, &registration, true, true).is_err());
    assert!(check_retired(&state.directory, &registration, false, false).is_err());
}

#[tokio::test]
async fn suspended_authentication_binds_all_envelope_fields() {
    let (_root, state) = crate::server::tests::fixture().await;
    let original = RuntimeRequest {
        protocol: PROCESS_PROTOCOL,
        session_id: state.registration.session_id,
        incarnation: state.registration.incarnation,
        token: state.registration.token.clone(),
        authorization: None,
        command: RuntimeCommand::Health,
    };
    authenticate(&state.registration, &original).unwrap();
    for field in ["protocol", "session", "incarnation", "token", "length"] {
        let mut request = original.clone();
        match field {
            "protocol" => request.protocol += 1,
            "session" => request.session_id = uuid::Uuid::new_v4(),
            "incarnation" => request.incarnation = uuid::Uuid::new_v4(),
            "token" => request.token = "x".repeat(request.token.len()),
            _ => request.token.push('x'),
        }
        assert!(
            authenticate(&state.registration, &request).is_err(),
            "{field}"
        );
    }
}

#[tokio::test]
async fn suspended_observations_reject_stale_and_missing_history_without_execution() {
    let (_root, state) = crate::server::tests::fixture().await;
    for command in [
        RuntimeCommand::History {
            offset: 0,
            limit: 1,
            expected_revision: Some(99),
        },
        RuntimeCommand::MessageChunk {
            expected_revision: 0,
            index: 0,
            offset: 0,
            limit: 64,
        },
        RuntimeCommand::RunOutput {
            run_id: uuid::Uuid::new_v4(),
            offset: 0,
            limit: 64,
        },
        RuntimeCommand::Rename {
            command_id: uuid::Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: u64::MAX,
            name: "not an observation".into(),
        },
    ] {
        assert!(read(&state, &command).await.is_err());
    }
    let events = read(
        &state,
        &RuntimeCommand::Events {
            after: 0,
            limit: 10,
            wait_ms: 0,
        },
    )
    .await
    .unwrap();
    assert!(events.is_object());
    assert!(state.active.lock().await.is_none());
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
}
