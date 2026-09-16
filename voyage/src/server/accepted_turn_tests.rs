//! End-to-end process protocol tests. The only HTTP peer is this test's loopback
//! listener. No shell, provider credential, environment override or child agent is
//! needed to drive admission, inference, persistence and observed cleanup.
use super::*;
use serde_json::{Value, json};
use std::collections::VecDeque;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Reply {
    status: u16,
    body: String,
    gate: Option<Arc<tokio::sync::Notify>>,
}
impl Reply {
    fn text(text: &str) -> Self {
        Self::chunks(vec![
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":"stop"}]} ),
        ])
    }
    fn chunks(chunks: Vec<Value>) -> Self {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str(&format!("data: {chunk}\n\n"));
        }
        body.push_str("data: [DONE]\n\n");
        Self {
            status: 200,
            body,
            gate: None,
        }
    }
    fn tool(name: &str, arguments: Value) -> Self {
        Self::chunks(vec![
            json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"fixture-call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
        ])
    }
    fn error(status: u16) -> Self {
        Self {
            status,
            body: json!({"error":{"message":"scripted refusal","type":"invalid_request_error"}})
                .to_string(),
            gate: None,
        }
    }
    fn held(mut self, gate: Arc<tokio::sync::Notify>) -> Self {
        self.gate = Some(gate);
        self
    }
}
struct ProviderFixture {
    url: String,
    requests: Arc<tokio::sync::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ProviderFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ProviderFixture {
    async fn new(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            let mut replies: VecDeque<_> = replies.into();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut block = [0u8; 4096];
                    let n = socket.read(&mut block).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&block[..n]);
                    assert!(bytes.len() < 2 * 1024 * 1024);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut block = [0u8; 4096];
                    let n = socket.read(&mut block).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&block[..n]);
                }
                let reply = if headers.starts_with("GET /v1/models ") {
                    Reply {
                        status: 200,
                        body: json!({"data":[{"id":"fixture"}]}).to_string(),
                        gate: None,
                    }
                } else {
                    assert!(
                        headers.starts_with("POST /v1/chat/completions "),
                        "{headers}"
                    );
                    let body: Value =
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                    captured.lock().await.push(body);
                    replies
                        .pop_front()
                        .unwrap_or_else(|| Reply::text("Finished."))
                };
                if let Some(gate) = reply.gate {
                    gate.notified().await;
                }
                let content_type = if reply.body.starts_with("data:") {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    content_type,
                    reply.body.len(),
                    reply.body
                );
                // Cancellation may deliberately close a held response's socket.
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    async fn wait_requests(&self, count: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while self.requests.lock().await.len() < count {
                assert!(!self.task.is_finished(), "scripted provider exited");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("provider request was not dispatched");
    }
}
fn deadline() -> u64 {
    (chrono::Utc::now().timestamp_millis() + 120_000) as u64
}
async fn configured(replies: Vec<Reply>) -> (tempfile::TempDir, Arc<State>, ProviderFixture) {
    let (root, state) = crate::server::tests::fixture().await;
    let provider = ProviderFixture::new(replies).await;
    *state.config.write().await = Config {
        provider: crate::config::ProviderKind::OpenaiChat,
        model: "fixture".into(),
        base_url: Some(provider.url.clone()),
        api_key_required: false,
        api_key_env: "VOYAGE_SCRIPTED_TEST_UNUSED_KEY".into(),
        access: Some(crate::config::AccessMode::ReadOnly),
        provider_retry_attempts: 1,
        provider_response_timeout_ms: 10_000,
        provider_stream_idle_ms: 10_000,
        ..Default::default()
    };
    (root, state, provider)
}
fn submit(prompt: &str) -> RuntimeCommand {
    RuntimeCommand::Submit {
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: deadline(),
        prompt: prompt.into(),
    }
}
async fn call(state: &Arc<State>, command: RuntimeCommand) -> RuntimeResponse {
    let (response, done) = exchange(state.clone(), request(state, command)).await;
    done.unwrap();
    response
}
async fn accepted(state: &Arc<State>, command: RuntimeCommand) -> Uuid {
    let response = call(state, command).await;
    assert!(response.error.is_none(), "{response:?}");
    assert!(!response.outcome_unknown);
    assert_eq!(response.result["status"], "accepted");
    assert_eq!(response.result["duplicate"], false);
    response.result["run_id"].as_str().unwrap().parse().unwrap()
}
async fn finished(state: &Arc<State>) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            state.cleanup.advance(1).await.unwrap();
            let snapshot = state.owner.process_snapshot().await.unwrap();
            if state.active.lock().await.is_none()
                && !snapshot["run"].is_null()
                && !matches!(
                    snapshot["run"]["state"].as_str(),
                    Some("accepted" | "running")
                )
                && snapshot["pending_cleanup_run"].is_null()
            {
                state
                    .controls
                    .shutdown_retained(&state.owner)
                    .await
                    .unwrap();
                return snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run did not finish with observed cleanup")
}

#[tokio::test]
async fn accepted_stream_persists_user_assistant_usage_and_terminal_checkpoint() {
    let (_root, state, provider) = configured(vec![Reply::chunks(vec![
        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Hello "},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"content":"世界"},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}}),
    ])]).await;
    let run = accepted(&state, submit("Say hello.")).await;
    let snapshot = finished(&state).await;
    assert_eq!(snapshot["run"]["run_id"], run.to_string());
    assert_eq!(snapshot["run"]["state"], "completed");
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.role == crate::model::Role::User && m.content == "Say hello.")
    );
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.role == crate::model::Role::Assistant && m.content == "Hello 世界")
    );
    assert!(saved.revision > 0);
    assert_eq!(provider.requests.lock().await.len(), 1);
    let requests = provider.requests.lock().await;
    assert_eq!(requests[0]["model"], "fixture");
    assert_eq!(requests[0]["stream"], true);
    assert!(snapshot["pending_cleanup_run"].is_null());
    assert_eq!(state.owner.session_resources().await.unwrap(), json!([]));
}

#[tokio::test]
async fn active_duplicate_returns_original_run_without_second_request() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("Only once.").held(gate.clone())]).await;
    let command = submit("One accepted command.");
    let run = accepted(&state, command.clone()).await;
    provider.wait_requests(1).await;
    let replay = call(&state, command).await;
    assert!(replay.error.is_none());
    assert_eq!(replay.result["run_id"], run.to_string());
    assert_eq!(replay.result["duplicate"], true);
    gate.notify_one();
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn conflicting_duplicate_is_rejected_without_replacing_inflight_prompt() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("Original.").held(gate.clone())]).await;
    let command = submit("Original prompt.");
    accepted(&state, command.clone()).await;
    provider.wait_requests(1).await;
    let mut conflict = command;
    if let RuntimeCommand::Submit { prompt, .. } = &mut conflict {
        *prompt = "Replacement prompt.".into();
    }
    let refusal = call(&state, conflict).await;
    assert!(refusal.error.is_some());
    gate.notify_one();
    finished(&state).await;
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        !saved
            .session
            .messages
            .iter()
            .any(|m| m.content == "Replacement prompt.")
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn cancellation_through_process_protocol_finalizes_held_provider_turn() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("Too late.").held(gate.clone())]).await;
    let run = accepted(&state, submit("Wait for cancellation.")).await;
    provider.wait_requests(1).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    let command = RuntimeCommand::Cancel {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: deadline(),
        run_id: run,
    };
    let response = call(&state, command).await;
    assert!(response.error.is_none(), "{response:?}");
    let snapshot = finished(&state).await;
    gate.notify_one();
    assert_eq!(snapshot["run"]["state"], "cancelled");
    assert!(
        !state
            .owner
            .snapshot()
            .await
            .unwrap()
            .session
            .messages
            .iter()
            .any(|m| m.content == "Too late.")
    );
}

#[tokio::test]
async fn provider_http_failure_finishes_admitted_turn_without_retry_or_unknown_cleanup() {
    for status in [400, 401, 403, 404, 422] {
        let (_root, state, provider) = configured(vec![Reply::error(status)]).await;
        accepted(&state, submit("Fail once.")).await;
        let snapshot = finished(&state).await;
        assert_eq!(snapshot["run"]["state"], "failed", "{status}: {snapshot}");
        assert_eq!(provider.requests.lock().await.len(), 1);
        assert!(snapshot["pending_cleanup_run"].is_null());
    }
}

#[tokio::test]
async fn read_tool_turn_returns_tool_result_to_second_provider_request() {
    let (_root, state, provider) = configured(vec![
        Reply::tool("read_file", json!({"path":"fixture.txt"})),
        Reply::text("The file contains isolated fixture content."),
    ])
    .await;
    std::fs::write(
        state.registration.workspace.join("fixture.txt"),
        "isolated fixture content",
    )
    .unwrap();
    accepted(&state, submit("Read fixture.txt and summarize it.")).await;
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    let requests = provider.requests.lock().await;
    assert_eq!(requests.len(), 2);
    let messages = requests[1]["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| {
        m["role"] == "tool"
            && m["content"]
                .as_str()
                .is_some_and(|s| s.contains("isolated fixture content"))
    }));
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.role == crate::model::Role::Tool)
    );
}

#[tokio::test]
async fn missing_file_tool_error_is_returned_to_model_not_a_transport_failure() {
    let (_root, state, provider) = configured(vec![
        Reply::tool("read_file", json!({"path":"missing-file.txt"})),
        Reply::text("The requested file is unavailable."),
    ])
    .await;
    accepted(&state, submit("Read missing-file.txt.")).await;
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    let requests = provider.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool")
    );
    assert!(
        !state
            .registration
            .workspace
            .join("missing-file.txt")
            .exists()
    );
}

#[tokio::test]
async fn unknown_tool_result_is_checkpointed_and_model_can_recover() {
    let (_root, state, provider) = configured(vec![
        Reply::tool("fixture_nonexistent_tool", json!({})),
        Reply::text("No such tool is available."),
    ])
    .await;
    accepted(&state, submit("Use only available tools.")).await;
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    assert_eq!(provider.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn forbidden_write_in_read_only_turn_does_not_create_file() {
    let (_root, state, provider) = configured(vec![
        Reply::tool(
            "write_file",
            json!({"path":"must-not-exist.txt","content":"forbidden"}),
        ),
        Reply::text("Writing is unavailable in read-only mode."),
    ])
    .await;
    accepted(&state, submit("Check the current access boundary.")).await;
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    assert!(
        !state
            .registration
            .workspace
            .join("must-not-exist.txt")
            .exists()
    );
    assert_eq!(provider.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn accepted_content_turn_preserves_text_parts_in_history() {
    let (_root, state, _provider) = configured(vec![Reply::text("Two text parts received.")]).await;
    let command = RuntimeCommand::SubmitContent {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: deadline(),
        content: vec![
            voyage_protocol::content::ContentPart::Text {
                text: "First part.".into(),
            },
            voyage_protocol::content::ContentPart::Text {
                text: "Second part.".into(),
            },
        ],
    };
    accepted(&state, command).await;
    assert_eq!(finished(&state).await["run"]["state"], "completed");
    let saved = state.owner.snapshot().await.unwrap();
    let user = saved
        .session
        .messages
        .iter()
        .find(|m| m.role == crate::model::Role::User)
        .unwrap();
    assert!(user.content.contains("First part."));
    assert!(user.content.contains("Second part."));
}

#[tokio::test]
async fn operator_read_turn_finishes_without_provider_inference() {
    let (_root, state, provider) = configured(vec![]).await;
    std::fs::write(
        state.registration.workspace.join("operator.txt"),
        "operator-owned input",
    )
    .unwrap();
    accepted(
        &state,
        RuntimeCommand::OperatorTool {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: deadline(),
            name: "read_file".into(),
            arguments: json!({"path":"operator.txt"}),
        },
    )
    .await;
    let snapshot = finished(&state).await;
    assert_eq!(snapshot["run"]["state"], "completed");
    assert!(provider.requests.lock().await.is_empty());
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.content.contains("operator-owned input"))
    );
}

#[tokio::test]
async fn active_control_inventory_and_stale_run_checks_cross_framed_transport() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("Controls observed.").held(gate.clone())]).await;
    let run = accepted(&state, submit("Hold while controls are inspected.")).await;
    provider.wait_requests(1).await;
    for section in [
        "tools",
        "policy",
        "todos",
        "subagents",
        "subagents_archive",
        "terminals",
        "workflows",
    ] {
        let response = call(
            &state,
            RuntimeCommand::Controls {
                run_id: Some(run),
                section: section.into(),
            },
        )
        .await;
        assert!(response.error.is_none(), "{section}: {response:?}");
        assert_eq!(response.result["run_id"], run.to_string());
    }
    let stale = call(
        &state,
        RuntimeCommand::Controls {
            run_id: Some(Uuid::new_v4()),
            section: "tools".into(),
        },
    )
    .await;
    assert!(stale.error.is_some());
    gate.notify_one();
    finished(&state).await;
}

#[tokio::test]
async fn active_lifecycle_rejection_does_not_interrupt_accepted_turn() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("Still completed.").held(gate.clone())]).await;
    accepted(&state, submit("Stay active.")).await;
    provider.wait_requests(1).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    let response = call(
        &state,
        RuntimeCommand::Clear {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: deadline(),
            confirm_session_id: state.owner.session_id(),
        },
    )
    .await;
    assert!(response.error.is_some());
    assert!(!response.outcome_unknown);
    assert_eq!(response.result["status"], "rejected");
    gate.notify_one();
    assert_eq!(finished(&state).await["run"]["state"], "completed");
}

#[tokio::test]
async fn durable_history_and_attempt_pages_remain_readable_after_execution() {
    let (_root, state, _provider) = configured(vec![Reply::text("Persisted answer.")]).await;
    let run = accepted(&state, submit("Persist this exchange.")).await;
    finished(&state).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    // Read through the owner after automatic suspension, not through a dead transport.
    let history = state
        .owner
        .process_history(0, 128, Some(revision))
        .await
        .unwrap();
    assert!(history["messages"].as_array().unwrap().len() >= 2);
    assert_eq!(history["has_more"], false);
    assert!(state.owner.process_history(0, 128, Some(0)).await.is_err());
    let attempts = state
        .owner
        .process_provider_attempts(Some(run), 0, 32, Some(revision))
        .await
        .unwrap();
    assert_eq!(attempts["revision"], revision);
    assert!(
        state
            .owner
            .process_provider_attempts(Some(run), 0, 0, Some(revision))
            .await
            .is_err()
    );
    assert!(
        state
            .owner
            .process_provider_attempts(Some(run), 0, 32, Some(0))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn steering_is_delivered_at_next_safe_boundary_and_replayed_by_identity() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![
        Reply::tool("list_directory", json!({"path":"."})).held(gate.clone()),
        Reply::text("Incorporated the operator update."),
    ])
    .await;
    let run = accepted(&state, submit("Inspect the workspace.")).await;
    provider.wait_requests(1).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    let steering = RuntimeCommand::Steer {
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: deadline(),
        run_id: run,
        prompt: "Also mention that this is an isolated fixture.".into(),
    };
    let first = call(&state, steering.clone()).await;
    assert!(first.error.is_none(), "{first:?}");
    let second = call(&state, steering).await;
    assert!(second.error.is_none(), "{second:?}");
    gate.notify_one();
    let snapshot = finished(&state).await;
    assert_eq!(snapshot["run"]["state"], "completed");
    let saved = state.owner.snapshot().await.unwrap();
    assert_eq!(
        saved
            .session
            .messages
            .iter()
            .filter(|m| m.content == "Also mention that this is an isolated fixture.")
            .count(),
        1
    );
    let requests = provider.requests.lock().await;
    assert!(requests.len() >= 2);
    assert!(requests[1].to_string().contains("isolated fixture"));
}

#[tokio::test]
async fn stale_steering_is_definitely_refused_without_delivery() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) =
        configured(vec![Reply::text("No stale update.").held(gate.clone())]).await;
    let run = accepted(&state, submit("Wait for a stale update.")).await;
    provider.wait_requests(1).await;
    let response = call(
        &state,
        RuntimeCommand::Steer {
            coordination: None,
            command_id: Uuid::new_v4(),
            expected_revision: u64::MAX,
            expires_at_ms: deadline(),
            run_id: run,
            prompt: "Never deliver this stale update.".into(),
        },
    )
    .await;
    assert!(response.error.is_some());
    assert!(!response.outcome_unknown);
    gate.notify_one();
    finished(&state).await;
    assert!(
        !state
            .owner
            .snapshot()
            .await
            .unwrap()
            .session
            .messages
            .iter()
            .any(|m| m.content == "Never deliver this stale update.")
    );
}

#[tokio::test]
async fn wrong_run_cancel_does_not_cancel_current_executor() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![
        Reply::text("Still the current run.").held(gate.clone()),
    ])
    .await;
    let run = accepted(&state, submit("Keep executing.")).await;
    provider.wait_requests(1).await;
    let response = call(
        &state,
        RuntimeCommand::Cancel {
            command_id: Uuid::new_v4(),
            expected_revision: state.owner.snapshot().await.unwrap().revision,
            expires_at_ms: deadline(),
            run_id: Uuid::new_v4(),
        },
    )
    .await;
    assert!(response.error.is_some());
    assert_eq!(state.active.lock().await.as_ref().unwrap().id, run);
    assert!(
        !state
            .active
            .lock()
            .await
            .as_ref()
            .unwrap()
            .cancel
            .is_cancelled()
    );
    gate.notify_one();
    assert_eq!(finished(&state).await["run"]["state"], "completed");
}

#[tokio::test]
async fn unfinished_stream_is_persisted_as_failed_not_successful_completion() {
    let (_root, state, provider) = configured(vec![Reply {
        status: 200,
        body: "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n".into(),
        gate: None,
    }]).await;
    accepted(&state, submit("Require a complete response.")).await;
    let snapshot = finished(&state).await;
    assert_ne!(snapshot["run"]["state"], "completed");
    assert_eq!(provider.requests.lock().await.len(), 1);
    assert!(snapshot["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn malformed_stream_json_does_not_leave_active_executor_or_cleanup() {
    let (_root, state, provider) = configured(vec![Reply {
        status: 200,
        body: "data: {not-json}\n\ndata: [DONE]\n\n".into(),
        gate: None,
    }])
    .await;
    accepted(&state, submit("Require valid provider frames.")).await;
    assert_eq!(finished(&state).await["run"]["state"], "failed");
    assert_eq!(provider.requests.lock().await.len(), 1);
    assert!(state.active.lock().await.is_none());
}

#[tokio::test]
async fn accepted_empty_assistant_does_not_invent_nonempty_provider_text() {
    let (_root, state, _provider) = configured(vec![Reply::text("")]).await;
    accepted(&state, submit("Empty scripted response.")).await;
    let snapshot = finished(&state).await;
    assert!(!matches!(
        snapshot["run"]["state"].as_str(),
        Some("accepted" | "running")
    ));
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.content == "Empty scripted response.")
    );
    assert!(snapshot["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn unicode_message_chunks_reconstruct_canonical_persisted_json() {
    let (_root, state, _provider) = configured(vec![Reply::text("Hello 世界 🌊 café.")]).await;
    accepted(&state, submit("Persist Unicode safely.")).await;
    finished(&state).await;
    let saved = state.owner.snapshot().await.unwrap();
    let index = saved
        .session
        .messages
        .iter()
        .position(|m| m.content == "Hello 世界 🌊 café.")
        .unwrap() as u64;
    let mut offset = 0;
    let mut bytes = String::new();
    loop {
        let page = state
            .owner
            .process_message_chunk(index, offset, 7, saved.revision)
            .await
            .unwrap();
        assert_eq!(page["offset"], offset);
        assert_eq!(page["encoding"], "public_message_json_utf8");
        bytes.push_str(page["data"].as_str().unwrap());
        let next = page["next_offset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
        if page["has_more"] == false {
            break;
        }
    }
    let message: Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(message["content"], "Hello 世界 🌊 café.");
    assert!(
        state
            .owner
            .process_message_chunk(index, offset + 1, 7, saved.revision)
            .await
            .is_err()
    );
    assert!(
        state
            .owner
            .process_message_chunk(index, 0, 7, saved.revision + 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn history_one_message_pages_are_lossless_after_tool_execution() {
    let (_root, state, _provider) = configured(vec![
        Reply::tool("list_directory", json!({"path":"."})),
        Reply::text("Listed the directory."),
    ])
    .await;
    accepted(&state, submit("List the workspace.")).await;
    finished(&state).await;
    let saved = state.owner.snapshot().await.unwrap();
    let mut offset = 0;
    let mut count = 0;
    loop {
        let page = state
            .owner
            .process_history(offset, 1, Some(saved.revision))
            .await
            .unwrap();
        assert_eq!(page["message_offset"], offset);
        let messages = page["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        count += messages.len();
        offset = page["next_offset"].as_u64().unwrap();
        if page["has_more"] == false {
            break;
        }
    }
    assert_eq!(count, saved.session.messages.len());
    let empty = state
        .owner
        .process_history(offset, 1, Some(saved.revision))
        .await
        .unwrap();
    assert_eq!(empty["messages"], json!([]));
    assert!(
        state
            .owner
            .process_history(offset + 1, 1, Some(saved.revision))
            .await
            .is_err()
    );
}
