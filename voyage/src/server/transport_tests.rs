use super::*;
use voyage_protocol::process::RuntimeCommand;

fn request(state: &State, command: RuntimeCommand) -> RuntimeRequest {
    RuntimeRequest {
        protocol: PROCESS_PROTOCOL,
        session_id: state.registration.session_id,
        incarnation: state.registration.incarnation,
        token: state.registration.token.clone(),
        authorization: None,
        command,
    }
}

async fn exchange(state: Arc<State>, request: RuntimeRequest) -> (RuntimeResponse, Result<()>) {
    let (server, mut client) = UnixStream::pair().unwrap();
    let directory = state.directory.clone();
    let task = tokio::spawn(respond(server, state, directory));
    write_frame(&mut client, &request).await.unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut client))
        .await
        .unwrap()
        .unwrap();
    (response, task.await.unwrap())
}

#[test]
fn token_comparison_checks_every_byte_and_length() {
    assert!(token_matches("", ""));
    assert!(token_matches("synthetic-token", "synthetic-token"));
    for (a, b) in [
        ("a", ""),
        ("", "a"),
        ("a", "b"),
        ("abc", "abd"),
        ("abc", "xbc"),
        ("é", "aa"),
    ] {
        assert!(!token_matches(a, b));
    }
}

#[tokio::test]
async fn framed_health_has_authenticated_identity_and_definite_outcome() {
    let (_root, state) = crate::server::tests::fixture().await;
    let (response, done) = exchange(state.clone(), request(&state, RuntimeCommand::Health)).await;
    done.unwrap();
    assert_eq!(response.protocol, PROCESS_PROTOCOL);
    assert_eq!(response.session_id, state.registration.session_id);
    assert_eq!(response.incarnation, state.registration.incarnation);
    assert!(!response.outcome_unknown);
    assert!(response.error.is_none());
    assert!(response.resumed_from.is_none());
    assert_eq!(
        response.result["session_id"],
        state.registration.session_id.to_string()
    );
}

#[tokio::test]
async fn invalid_envelopes_are_closed_without_a_response_or_journal_effect() {
    let (_root, state) = crate::server::tests::fixture().await;
    for field in ["protocol", "session", "incarnation", "token"] {
        let mut req = request(&state, RuntimeCommand::Health);
        match field {
            "protocol" => req.protocol += 1,
            "session" => req.session_id = Uuid::new_v4(),
            "incarnation" => req.incarnation = Uuid::new_v4(),
            _ => req.token.push('!'),
        }
        let (server, mut client) = UnixStream::pair().unwrap();
        let task = tokio::spawn(respond(server, state.clone(), state.directory.clone()));
        write_frame(&mut client, &req).await.unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("authentication rejected"),
            "{field}: {error}"
        );
        assert!(read_frame::<RuntimeResponse>(&mut client).await.is_err());
    }
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
}

#[tokio::test]
async fn framed_rejection_is_definite_but_unclassified_read_failure_is_uncertain() {
    let (_root, state) = crate::server::tests::fixture().await;
    let id = Uuid::new_v4();
    let rename = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 99,
        expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
        name: "stale request".into(),
    };
    let (rejected, done) = exchange(state.clone(), request(&state, rename.clone())).await;
    done.unwrap();
    assert!(!rejected.outcome_unknown);
    assert!(rejected.error.is_some());
    assert_eq!(rejected.result["status"], "rejected");
    let (replay, done) = exchange(state.clone(), request(&state, rename)).await;
    done.unwrap();
    assert_eq!(replay.result, rejected.result);
    let (unknown, done) = exchange(
        state.clone(),
        request(
            &state,
            RuntimeCommand::RunOutput {
                run_id: Uuid::new_v4(),
                offset: 0,
                limit: 100,
            },
        ),
    )
    .await;
    done.unwrap();
    assert!(unknown.outcome_unknown);
    assert!(unknown.error.is_some());
    assert!(unknown.result.is_null());
}

#[tokio::test]
async fn suspension_response_proves_no_dispatch_even_for_valid_mutation() {
    let (_root, state) = crate::server::tests::fixture().await;
    state
        .suspend_requested
        .store(true, std::sync::atomic::Ordering::Release);
    state.shutdown.cancel();
    let id = Uuid::new_v4();
    let req = request(
        &state,
        RuntimeCommand::Rename {
            command_id: id,
            expected_revision: 0,
            expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
            name: "never dispatched".into(),
        },
    );
    let (response, done) = exchange(state.clone(), req).await;
    done.unwrap();
    assert!(!response.outcome_unknown);
    assert!(response.error.is_none());
    assert_eq!(response.result["status"], "suspending");
    assert_eq!(response.result["not_dispatched"], true);
    assert!(state.owner.process_receipt(id).await.unwrap().is_none());
    assert_eq!(state.owner.snapshot().await.unwrap().session.revision, 0);
}

#[path = "accepted_turn_tests.rs"]
mod accepted_turn_tests;

#[path = "retained_lifecycle_tests.rs"]
mod retained_lifecycle_tests;

#[path = "supervisor_socket_tests.rs"]
mod supervisor_socket_tests;
