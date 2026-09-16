//! Exercise the same private Unix socket endpoint used by the supervisor, rather
//! than bypassing authentication and lifecycle with direct journal calls.
use super::*;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;

struct SocketFixture {
    _root: tempfile::TempDir,
    state: Arc<State>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl SocketFixture {
    async fn new() -> Self {
        let (root, state) = crate::server::tests::fixture().await;
        let task = tokio::spawn(crate::server::transport::listen(
            state.directory.clone(),
            state.clone(),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !state.directory.join("runtime.sock").exists() {
                assert!(!task.is_finished());
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        Self {
            _root: root,
            state,
            task,
        }
    }
    async fn call(&self, command: RuntimeCommand) -> RuntimeResponse {
        let mut socket = UnixStream::connect(self.state.directory.join("runtime.sock"))
            .await
            .unwrap();
        write_frame(&mut socket, &request(&self.state, command))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut socket))
            .await
            .unwrap()
            .unwrap()
    }
    async fn stop(self) -> Value {
        self.state.shutdown.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(10), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!self.state.directory.join("runtime.sock").exists());
        let stopped: Value = serde_json::from_slice(
            &std::fs::read(self.state.directory.join("stopped.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            stopped["session_id"],
            self.state.registration.session_id.to_string()
        );
        assert_eq!(
            stopped["incarnation"],
            self.state.registration.incarnation.to_string()
        );
        assert_eq!(stopped["cleanup_observed"], true);
        stopped
    }
}
fn expiry() -> u64 {
    (chrono::Utc::now().timestamp_millis() + 60_000) as u64
}
fn rename(id: Uuid, revision: u64, name: &str) -> RuntimeCommand {
    RuntimeCommand::Rename {
        command_id: id,
        expected_revision: revision,
        expires_at_ms: expiry(),
        name: name.into(),
    }
}

#[tokio::test]
async fn supervisor_endpoint_is_private_and_health_proves_incarnation() {
    let fixture = SocketFixture::new().await;
    let metadata = std::fs::metadata(fixture.state.directory.join("runtime.sock")).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let response = fixture.call(RuntimeCommand::Health).await;
    assert!(response.error.is_none());
    assert!(!response.outcome_unknown);
    assert_eq!(response.session_id, fixture.state.registration.session_id);
    assert_eq!(response.incarnation, fixture.state.registration.incarnation);
    assert_eq!(
        response.result["session_id"],
        fixture.state.registration.session_id.to_string()
    );
    let stopped = fixture.stop().await;
    assert_eq!(stopped["suspended"], false);
}

#[tokio::test]
async fn socket_rename_is_durable_and_replays_same_receipt() {
    let fixture = SocketFixture::new().await;
    let id = Uuid::new_v4();
    let command = rename(id, 0, "socket rename");
    let first = fixture.call(command.clone()).await;
    assert!(first.error.is_none(), "{first:?}");
    assert_eq!(first.result["revision"], 1);
    let replay = fixture.call(command).await;
    assert_eq!(replay.result, first.result);
    let receipt = fixture
        .call(RuntimeCommand::Receipt { command_id: id })
        .await;
    assert_eq!(receipt.result, first.result);
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 1);
    fixture.stop().await;
}

#[tokio::test]
async fn socket_stale_mutation_is_definite_and_receipt_is_replayable() {
    let fixture = SocketFixture::new().await;
    let id = Uuid::new_v4();
    let command = rename(id, 12, "stale");
    let first = fixture.call(command.clone()).await;
    assert!(first.error.is_some());
    assert!(!first.outcome_unknown);
    assert_eq!(first.result["status"], "rejected");
    let replay = fixture.call(command).await;
    assert_eq!(replay.result, first.result);
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn socket_resolve_fences_missing_command_before_later_delivery() {
    let fixture = SocketFixture::new().await;
    let id = Uuid::new_v4();
    let command = rename(id, 0, "must not dispatch");
    let resolved = fixture
        .call(RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(command.clone())),
        })
        .await;
    assert!(resolved.error.is_none());
    assert_eq!(resolved.result["status"], "not_admitted");
    let delayed = fixture.call(command).await;
    assert!(delayed.error.is_some());
    assert!(!delayed.outcome_unknown);
    assert_eq!(delayed.result["status"], "not_admitted");
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn socket_resolve_of_completed_command_preserves_actual_outcome() {
    let fixture = SocketFixture::new().await;
    let id = Uuid::new_v4();
    let command = rename(id, 0, "resolved result");
    let completed = fixture.call(command.clone()).await;
    assert!(completed.error.is_none());
    let resolved = fixture
        .call(RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(command)),
        })
        .await;
    assert_eq!(resolved.result, completed.result);
    fixture.stop().await;
}

#[tokio::test]
async fn concurrent_stale_metadata_writers_cannot_both_advance_revision() {
    let fixture = SocketFixture::new().await;
    let first = rename(Uuid::new_v4(), 0, "first writer");
    let second = rename(Uuid::new_v4(), 0, "second writer");
    let (a, b) = tokio::join!(fixture.call(first), fixture.call(second));
    assert_ne!(a.error.is_none(), b.error.is_none());
    assert!(!a.outcome_unknown);
    assert!(!b.outcome_unknown);
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 1);
    fixture.stop().await;
}

#[tokio::test]
async fn malformed_connection_does_not_poison_next_authenticated_request() {
    use tokio::io::AsyncWriteExt;
    let fixture = SocketFixture::new().await;
    let mut socket = UnixStream::connect(fixture.state.directory.join("runtime.sock"))
        .await
        .unwrap();
    socket
        .write_all(&[0, 0, 0, 3, b'?', b'?', b'?'])
        .await
        .unwrap();
    socket.shutdown().await.unwrap();
    drop(socket);
    let response = fixture.call(RuntimeCommand::Health).await;
    assert!(response.error.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn wrong_token_connection_is_closed_without_dispatching_metadata() {
    let fixture = SocketFixture::new().await;
    let mut socket = UnixStream::connect(fixture.state.directory.join("runtime.sock"))
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let mut envelope = request(&fixture.state, rename(id, 0, "unauthenticated"));
    envelope.token = "wrong token".into();
    write_frame(&mut socket, &envelope).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            read_frame::<RuntimeResponse>(&mut socket)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert!(
        fixture
            .state
            .owner
            .process_receipt(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 0);
    assert!(fixture.call(RuntimeCommand::Health).await.error.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn wrong_incarnation_is_closed_without_creating_a_receipt() {
    let fixture = SocketFixture::new().await;
    let mut socket = UnixStream::connect(fixture.state.directory.join("runtime.sock"))
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let mut envelope = request(&fixture.state, rename(id, 0, "old owner"));
    envelope.incarnation = Uuid::new_v4();
    write_frame(&mut socket, &envelope).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            read_frame::<RuntimeResponse>(&mut socket)
        )
        .await
        .unwrap()
        .is_err()
    );
    assert!(
        fixture
            .state
            .owner
            .process_receipt(id)
            .await
            .unwrap()
            .is_none()
    );
    fixture.stop().await;
}

#[tokio::test]
async fn event_long_poll_does_not_hold_exclusive_resolve_barrier() {
    let fixture = SocketFixture::new().await;
    let initial = fixture
        .call(RuntimeCommand::Events {
            after: 0,
            limit: 128,
            wait_ms: 0,
        })
        .await;
    assert!(initial.error.is_none());
    let cursor = initial.result["cursor"].as_u64().unwrap();
    let id = Uuid::new_v4();
    let (events, resolved) = tokio::join!(
        fixture.call(RuntimeCommand::Events {
            after: cursor,
            limit: 128,
            wait_ms: 250
        }),
        fixture.call(RuntimeCommand::Resolve {
            command_id: id,
            original: None
        }),
    );
    assert!(events.error.is_none());
    assert!(resolved.error.is_none());
    assert_eq!(resolved.result["status"], "not_admitted");
    fixture.stop().await;
}

#[tokio::test]
async fn event_pages_advance_after_metadata_and_replay_is_stable() {
    let fixture = SocketFixture::new().await;
    let initial = fixture
        .call(RuntimeCommand::Events {
            after: 0,
            limit: 128,
            wait_ms: 0,
        })
        .await;
    let cursor = initial.result["cursor"].as_u64().unwrap();
    let result = fixture
        .call(rename(Uuid::new_v4(), 0, "observed name"))
        .await;
    assert!(result.error.is_none());
    let first = fixture
        .call(RuntimeCommand::Events {
            after: cursor,
            limit: 1,
            wait_ms: 0,
        })
        .await;
    assert!(first.error.is_none());
    assert_eq!(first.result["projection"], "public-v1");
    assert_eq!(first.result["replay_gap"], false);
    assert_eq!(first.result["events"].as_array().unwrap().len(), 1);
    assert!(first.result["cursor"].as_u64().unwrap() > cursor);
    let replay = fixture
        .call(RuntimeCommand::Events {
            after: cursor,
            limit: 1,
            wait_ms: 0,
        })
        .await;
    assert_eq!(replay.result, first.result);
    fixture.stop().await;
}

#[tokio::test]
async fn invalid_event_bounds_leave_owner_responsive() {
    let fixture = SocketFixture::new().await;
    for command in [
        RuntimeCommand::Events {
            after: 0,
            limit: 0,
            wait_ms: 0,
        },
        RuntimeCommand::Events {
            after: 0,
            limit: 129,
            wait_ms: 0,
        },
        RuntimeCommand::Events {
            after: u64::MAX,
            limit: 1,
            wait_ms: 0,
        },
        RuntimeCommand::Events {
            after: 0,
            limit: 1,
            wait_ms: 10_001,
        },
    ] {
        assert!(fixture.call(command).await.error.is_some());
    }
    assert!(fixture.call(RuntimeCommand::Health).await.error.is_none());
    fixture.stop().await;
}

#[tokio::test]
async fn history_read_validation_never_creates_process_receipts() {
    let fixture = SocketFixture::new().await;
    for command in [
        RuntimeCommand::History {
            offset: 0,
            limit: 0,
            expected_revision: None,
        },
        RuntimeCommand::History {
            offset: 1,
            limit: 1,
            expected_revision: None,
        },
        RuntimeCommand::History {
            offset: 0,
            limit: 1,
            expected_revision: Some(1),
        },
        RuntimeCommand::MessageChunk {
            index: 0,
            offset: 0,
            limit: 10,
            expected_revision: 0,
        },
        RuntimeCommand::RunOutput {
            run_id: Uuid::new_v4(),
            offset: 0,
            limit: 10,
        },
    ] {
        let response = fixture.call(command).await;
        assert!(response.error.is_some());
        assert!(response.outcome_unknown);
        assert!(response.result.is_null());
    }
    assert_eq!(fixture.state.owner.snapshot().await.unwrap().revision, 0);
    fixture.stop().await;
}

#[tokio::test]
async fn empty_history_page_has_stable_revision_and_offset_contract() {
    let fixture = SocketFixture::new().await;
    let response = fixture
        .call(RuntimeCommand::History {
            offset: 0,
            limit: 128,
            expected_revision: Some(0),
        })
        .await;
    assert!(response.error.is_none());
    assert_eq!(response.result["revision"], 0);
    assert_eq!(response.result["messages"], json!([]));
    assert_eq!(response.result["message_offset"], 0);
    assert_eq!(response.result["next_offset"], 0);
    assert_eq!(response.result["total_messages"], 0);
    assert_eq!(response.result["has_more"], false);
    fixture.stop().await;
}

#[tokio::test]
async fn idle_suspension_writes_observed_stopped_record_and_removes_socket() {
    let fixture = SocketFixture::new().await;
    crate::server::suspension::suspend(&fixture.state)
        .await
        .unwrap();
    assert!(fixture.state.shutdown.is_cancelled());
    assert!(
        fixture
            .state
            .suspend_requested
            .load(std::sync::atomic::Ordering::Acquire)
    );
    let stopped = fixture.stop().await;
    assert_eq!(stopped["suspended"], true);
    assert!(stopped["archive"].is_null());
    assert!(stopped["deletion"].is_null());
}

#[tokio::test]
async fn stop_command_is_acknowledged_before_clean_endpoint_shutdown() {
    let fixture = SocketFixture::new().await;
    let response = fixture.call(RuntimeCommand::Stop).await;
    assert!(response.error.is_none());
    assert!(!response.outcome_unknown);
    let stopped = fixture.stop().await;
    assert_eq!(stopped["cleanup_observed"], true);
}

#[tokio::test]
async fn regular_file_at_endpoint_is_not_deleted_by_listener() {
    let (_root, state) = crate::server::tests::fixture().await;
    let endpoint = state.directory.join("runtime.sock");
    std::fs::write(&endpoint, "preserve unrelated file").unwrap();
    let error = crate::server::transport::listen(state.directory.clone(), state.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unsafe runtime endpoint"));
    assert_eq!(
        std::fs::read_to_string(endpoint).unwrap(),
        "preserve unrelated file"
    );
    assert!(!state.directory.join("stopped.json").exists());
}

#[tokio::test]
async fn symlink_at_endpoint_is_not_followed_or_removed() {
    use std::os::unix::fs::symlink;
    let (_root, state) = crate::server::tests::fixture().await;
    let target = state.directory.join("unrelated");
    std::fs::write(&target, "preserve target").unwrap();
    let endpoint = state.directory.join("runtime.sock");
    symlink(&target, &endpoint).unwrap();
    assert!(
        crate::server::transport::listen(state.directory.clone(), state.clone())
            .await
            .is_err()
    );
    assert!(
        std::fs::symlink_metadata(endpoint)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "preserve target");
}

#[tokio::test]
async fn stale_owned_socket_is_replaced_and_new_endpoint_serves_health() {
    let (_root, state) = crate::server::tests::fixture().await;
    let endpoint = state.directory.join("runtime.sock");
    let stale = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
    drop(stale);
    let task = tokio::spawn(crate::server::transport::listen(
        state.directory.clone(),
        state.clone(),
    ));
    let mut socket = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match UnixStream::connect(&endpoint).await {
                Ok(socket) => break socket,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .unwrap();
    write_frame(&mut socket, &request(&state, RuntimeCommand::Health))
        .await
        .unwrap();
    let response: RuntimeResponse = read_frame(&mut socket).await.unwrap();
    assert!(response.error.is_none());
    state.shutdown.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!endpoint.exists());
    assert!(state.directory.join("stopped.json").exists());
}
