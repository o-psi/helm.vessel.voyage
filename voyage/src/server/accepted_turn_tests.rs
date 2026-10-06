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
                        body:
                            json!({"data":[{"id":"fixture","input_modalities":["text","image"]}]})
                                .to_string(),
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
        budget: None,
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
            let snapshot = match state.owner.process_snapshot().await {
                Ok(snapshot) => snapshot,
                Err(error) if matches!(error.downcast_ref::<rusqlite::Error>(),
                    Some(rusqlite::Error::SqliteFailure(code,_)) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked)) => {
                    // This is a bounded observation poll, never mutation replay.
                    // A live journal intentionally refuses rather than blocks
                    // behind a concurrent writer; the outer deadline still applies.
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    continue;
                }
                Err(error) => panic!("snapshot failed: {error:#}"),
            };
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
async fn reserved_goal_turn_uses_native_usage_and_settles_without_claiming_goal_completion() {
    use voyage_protocol::goals::*;
    for remote_enabled in [false, true] {
        let (_root,state,provider)=configured(vec![Reply::chunks(vec![
        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"One step finished"},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}}),
    ])]).await;
        state.config.write().await.vessel.enabled = remote_enabled;
        let response = call(
            &state,
            RuntimeCommand::GoalUpdate {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: deadline(),
                action: GoalAction::Set {
                    objective: "Complete and verify a larger task".into(),
                    limits: GoalLimits::default(),
                    replace_goal_id: None,
                    continue_automatically: true,
                },
            },
        )
        .await;
        assert!(response.error.is_none(), "{response:?}");
        assert!(provider.requests.lock().await.is_empty());
        let goal = state.owner.goal().await.unwrap();
        let reserved = state
            .owner
            .reserve_goal_turn(
                goal.revision,
                state.registration.incarnation,
                "Continue the current goal".into(),
            )
            .await
            .unwrap();
        assert_eq!(reserved.authority.principal_id, state.actor.principal_id);
        let run = accepted(&state, reserved.command).await;
        let snapshot = finished(&state).await;
        assert_eq!(snapshot["run"]["run_id"], run.to_string());
        assert_eq!(snapshot["run"]["state"], "completed");
        let goal = state.owner.goal().await.unwrap().goal.unwrap();
        assert_eq!(goal.status, GoalStatus::Active, "{goal:?}");
        assert_eq!(goal.stop_reason, None);
        assert_eq!(
            (
                goal.usage.runs,
                goal.usage.input_tokens,
                goal.usage.output_tokens,
                goal.usage.unmeasured_runs
            ),
            (1, 20, 4, 0)
        );
        assert_eq!(goal.usage.no_progress_runs, 1);
        assert_eq!(provider.requests.lock().await.len(), 1);
    }
}

async fn bounded_goal_command(
    state: &Arc<State>,
    limits: voyage_protocol::goals::GoalLimits,
) -> RuntimeCommand {
    use voyage_protocol::goals::GoalAction;
    state.config.write().await.vessel.enabled = false;
    let response = call(
        state,
        RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: deadline(),
            action: GoalAction::Set {
                objective: "Bounded offline fixture".into(),
                limits,
                replace_goal_id: None,
                continue_automatically: true,
            },
        },
    )
    .await;
    assert!(response.error.is_none(), "{response:?}");
    state
        .owner
        .reserve_goal_turn(
            state.owner.goal().await.unwrap().revision,
            state.registration.incarnation,
            "Continue within the finite Goal budget".into(),
        )
        .await
        .unwrap()
        .command
}

#[tokio::test]
async fn goal_token_limit_blocks_native_tool_effects_after_the_last_response() {
    use voyage_protocol::goals::*;
    let mut reply = Reply::tool(
        "write_file",
        json!({"path":"goal-must-not-exist.txt","content":"forbidden after budget"}),
    );
    reply.body=reply.body.replace("data: [DONE]",&format!("data: {}\n\ndata: [DONE]",json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})));
    let (_root, state, provider) = configured(vec![reply]).await;
    state.config.write().await.access = Some(crate::config::AccessMode::Unrestricted);
    let command = bounded_goal_command(
        &state,
        GoalLimits {
            tokens: 24,
            ..GoalLimits::default()
        },
    )
    .await;
    accepted(&state, command).await;
    finished(&state).await;
    let goal = state.owner.goal().await.unwrap().goal.unwrap();
    assert_eq!(goal.status, GoalStatus::Limited, "{goal:?}");
    assert_eq!(goal.stop_reason, Some(GoalStopReason::TokenLimit));
    assert_eq!(goal.usage.input_tokens + goal.usage.output_tokens, 24);
    assert!(
        !state
            .registration
            .workspace
            .join("goal-must-not-exist.txt")
            .exists()
    );
    let requests = provider.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "write_file")
    );
}

#[tokio::test]
async fn goal_deadline_cancels_native_inference_and_observes_cleanup() {
    use voyage_protocol::goals::*;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![
        Reply::text("Must not be awaited indefinitely").held(gate),
    ])
    .await;
    let command = bounded_goal_command(
        &state,
        GoalLimits {
            elapsed_ms: 5000,
            ..GoalLimits::default()
        },
    )
    .await;
    accepted(&state, command).await;
    provider.wait_requests(1).await;
    let snapshot = finished(&state).await;
    assert_eq!(snapshot["run"]["state"], "cancelled");
    assert!(snapshot["pending_cleanup_run"].is_null());
    let goal = state.owner.goal().await.unwrap().goal.unwrap();
    assert_eq!(goal.status, GoalStatus::Limited, "{goal:?}");
    assert_eq!(goal.stop_reason, Some(GoalStopReason::TimeLimit));
    assert!(!goal.continuation_authorized);
    assert_eq!(goal.usage.unmeasured_runs, 1);
    assert_eq!(provider.requests.lock().await.len(), 1);
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
        parts: Vec::new(),
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
async fn image_steering_reaches_next_provider_request_in_same_run() {
    use base64::Engine as _;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![
        Reply::tool("list_directory", json!({"path":"."})).held(gate.clone()),
        Reply::text("Image received during run."),
    ])
    .await;
    state.config.write().await.model = "gpt-4o".into();
    let run = accepted(&state, submit("Inspect workspace.")).await;
    provider.wait_requests(1).await;
    let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    let attachment = state
        .owner
        .put_image(
            state.actor.principal_id,
            Uuid::new_v4(),
            "pixel.png".into(),
            bytes,
            None,
        )
        .await
        .unwrap();
    let steering = RuntimeCommand::Steer {
        parts: vec![voyage_protocol::content::ContentPart::Image { attachment }],
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: state.owner.snapshot().await.unwrap().revision,
        expires_at_ms: deadline(),
        run_id: run,
        prompt: String::new(),
    };
    let response = call(&state, steering.clone()).await;
    assert!(response.error.is_none(), "{response:?}");
    assert!(call(&state, steering.clone()).await.error.is_none());
    let mut conflict = steering.clone();
    if let RuntimeCommand::Steer { parts, .. } = &mut conflict
        && let voyage_protocol::content::ContentPart::Image { attachment } = &mut parts[0]
    {
        attachment.id = Uuid::new_v4();
    }
    assert!(call(&state, conflict).await.error.is_some());
    gate.notify_one();
    let snapshot = finished(&state).await;
    assert_eq!(snapshot["run"]["state"], "completed", "{snapshot}");
    let requests = provider.requests.lock().await;
    assert!(requests[1].to_string().contains("data:image/png;base64,"));
    let saved = state.owner.snapshot().await.unwrap();
    assert_eq!(
        saved
            .session
            .messages
            .iter()
            .filter(|m| m.steering.is_some() && !m.parts.is_empty())
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(&saved.session.messages)
            .unwrap()
            .contains("data:image/png;base64,")
    );
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
            parts: Vec::new(),
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

#[tokio::test]
async fn delegated_goal_budget_is_enforced_and_returns_exact_usage_without_goal() {
    use voyage_protocol::execution_budget::*;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![Reply::chunks(vec![json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Bounded child result"},"finish_reason":"stop"}]}),json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})]).held(gate.clone())]).await;
    state.config.write().await.vessel.enabled = false;
    let mut command = submit("Do one bounded child turn");
    let allocation = if let RuntimeCommand::Submit {
        command_id, budget, ..
    } = &mut command
    {
        let allocation = ExecutionBudget {
            session_id: state.registration.session_id,
            command_id: *command_id,
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 100,
            elapsed_ms: 30_000,
            expires_at_ms: deadline(),
        };
        *budget = Some(allocation.clone());
        allocation
    } else {
        unreachable!()
    };
    let run = accepted(&state, command.clone()).await;
    provider.wait_requests(1).await;
    let duplicate = call(&state, command.clone()).await;
    assert!(duplicate.error.is_none(), "{duplicate:?}");
    assert_eq!(duplicate.result["run_id"], run.to_string(), "{duplicate:?}");
    if let RuntimeCommand::Submit {
        budget: Some(b), ..
    } = &mut command
    {
        b.tokens += 1;
    }
    assert!(call(&state, command).await.error.is_some());
    gate.notify_one();
    let snapshot = finished(&state).await;
    let receipt: ExecutionUsage =
        serde_json::from_value(snapshot["execution_usage"].clone()).unwrap();
    assert_eq!(receipt.budget, allocation);
    assert_eq!(receipt.run_id, run);
    assert_eq!((receipt.input_tokens, receipt.output_tokens), (20, 4));
    assert!(receipt.complete && receipt.cleanup_observed, "{receipt:?}");
    assert!(state.owner.goal().await.unwrap().goal.is_none());
    assert_eq!(provider.requests.lock().await.len(), 1);
    assert_eq!(
        state.owner.process_snapshot().await.unwrap()["execution_usage"],
        snapshot["execution_usage"]
    );
}

#[tokio::test]
async fn expired_delegated_goal_budget_never_dispatches_provider() {
    use voyage_protocol::execution_budget::*;
    let (_root, state, provider) = configured(vec![]).await;
    let mut command = submit("Expired bounded child turn");
    if let RuntimeCommand::Submit {
        command_id, budget, ..
    } = &mut command
    {
        *budget = Some(ExecutionBudget {
            session_id: state.registration.session_id,
            command_id: *command_id,
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 100,
            elapsed_ms: 30_000,
            expires_at_ms: 1,
        });
    }
    assert!(call(&state, command).await.error.is_some());
    assert_eq!(provider.requests.lock().await.len(), 0);
}

#[tokio::test]
async fn delegated_goal_token_budget_blocks_a_returned_write_tool() {
    use voyage_protocol::execution_budget::*;
    let mut reply = Reply::tool(
        "write_file",
        json!({"path":"delegated-budget-blocked.txt","content":"must not be written"}),
    );
    reply.body=reply.body.replace("data: [DONE]",&format!("data: {}\n\ndata: [DONE]",json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})));
    let (root, state, provider) = configured(vec![reply]).await;
    {
        let mut config = state.config.write().await;
        config.vessel.enabled = false;
        config.access = Some(crate::config::AccessMode::Unrestricted);
    }
    let mut command = submit("Write a file within the child budget");
    if let RuntimeCommand::Submit {
        command_id, budget, ..
    } = &mut command
    {
        *budget = Some(ExecutionBudget {
            session_id: state.registration.session_id,
            command_id: *command_id,
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 24,
            elapsed_ms: 30_000,
            expires_at_ms: deadline(),
        });
    }
    accepted(&state, command).await;
    let snapshot = finished(&state).await;
    assert!(
        !root
            .path()
            .join("workspace/delegated-budget-blocked.txt")
            .exists()
    );
    assert!(
        !state
            .registration
            .workspace
            .join("delegated-budget-blocked.txt")
            .exists()
    );
    let receipt: ExecutionUsage =
        serde_json::from_value(snapshot["execution_usage"].clone()).unwrap();
    assert_eq!((receipt.input_tokens, receipt.output_tokens), (20, 4));
    assert!(receipt.cleanup_observed);
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn goal_terminal_accounting_retains_admission_until_active_handle_is_released() {
    let (_root, state, provider) = configured(vec![]).await;
    let request = crate::attachment::journal::TurnAdmission {
        budget: None,
        coordination: None,
        operator_name: None,
        command_id: Uuid::new_v4(),
        machine_id: state.actor.installation_id,
        principal_id: state.actor.principal_id,
        session_id: state.registration.session_id,
        expected_revision: 0,
        expires_at_ms: deadline() as i64,
        prompt: "Old run".into(),
        parts: vec![],
    };
    let crate::attachment::runtime::Admission::New(mut run) =
        state.owner.admit(request).await.unwrap()
    else {
        panic!("new run required")
    };
    let finished = run.fail_before_execution().await.unwrap();
    drop(run);
    *state.active.lock().await = Some(ActiveRun {
        id: finished.id,
        inference: json!({}),
        cancel: CancellationToken::new(),
        steering: None,
    });
    let mut command = submit("New human input during terminal accounting");
    if let RuntimeCommand::Submit {
        expected_revision, ..
    } = &mut command
    {
        *expected_revision = state.owner.snapshot().await.unwrap().revision;
    }
    let response = call(&state, command).await;
    assert!(
        response
            .error
            .as_ref()
            .is_some_and(|error| error.contains("terminal accounting")),
        "{response:?}"
    );
    assert_eq!(state.active.lock().await.as_ref().unwrap().id, finished.id);
    assert_eq!(
        state.owner.process_snapshot().await.unwrap()["run"]["run_id"],
        finished.id.to_string()
    );
    assert!(provider.requests.lock().await.is_empty());
}

async fn automatic_goal(state: &Arc<State>, limits: voyage_protocol::goals::GoalLimits) {
    automatic_goal_objective(
        state,
        limits,
        "Finish the larger task; this text is user data.",
    )
    .await;
}
async fn automatic_goal_objective(
    state: &Arc<State>,
    limits: voyage_protocol::goals::GoalLimits,
    objective: &str,
) {
    use voyage_protocol::goals::GoalAction;
    state.config.write().await.vessel.enabled = false;
    let response = call(
        state,
        RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: state.owner.snapshot().await.unwrap().revision,
            expires_at_ms: deadline(),
            action: GoalAction::Set {
                objective: objective.into(),
                limits,
                replace_goal_id: None,
                continue_automatically: true,
            },
        },
    )
    .await;
    assert!(response.error.is_none(), "{response:?}");
}
fn measured_reply() -> Reply {
    Reply::chunks(vec![
        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"One step completed."},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}}),
    ])
}
async fn goal_stopped(state: &Arc<State>) -> voyage_protocol::goals::Goal {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let goal = state.owner.goal().await.unwrap().goal.unwrap();
            if goal.status != voyage_protocol::goals::GoalStatus::Active
                && state.active.lock().await.is_none()
            {
                return goal;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Goal did not stop")
}

#[tokio::test]
async fn goal_driver_continues_once_and_limits_do_not_claim_success() {
    use voyage_protocol::goals::*;
    let second = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![
        measured_reply(),
        measured_reply().held(second.clone()),
    ])
    .await;
    automatic_goal(
        &state,
        GoalLimits {
            runs: 2,
            ..Default::default()
        },
    )
    .await;
    let driver = tokio::spawn(super::goals::drive(state.clone()));
    provider.wait_requests(2).await;
    let goal = state.owner.goal().await.unwrap().goal.unwrap();
    assert_eq!(
        (
            goal.usage.runs,
            goal.usage.input_tokens,
            goal.usage.output_tokens
        ),
        (2, 20, 4)
    );
    // More wakes cannot admit a third turn or replace the current handle.
    for _ in 0..10 {
        state.goal_wake.notify_one();
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.requests.lock().await.len(), 2);
    second.notify_one();
    let stopped = goal_stopped(&state).await;
    assert_eq!(stopped.status, GoalStatus::Limited);
    assert_eq!(stopped.stop_reason, Some(GoalStopReason::RunLimit));
    assert_eq!(
        (
            stopped.usage.runs,
            stopped.usage.input_tokens,
            stopped.usage.output_tokens,
            stopped.usage.unmeasured_runs
        ),
        (2, 40, 8, 0)
    );
    let saved = state.owner.snapshot().await.unwrap();
    assert_eq!(saved.session.run_summaries.len(), 2);
    assert_eq!(saved.session.id, state.registration.session_id);
    let requests = provider.requests.lock().await;
    for request in requests.iter() {
        let messages = request["messages"].as_array().unwrap();
        assert!(messages.iter().any(|m| {
            m["role"] == "user"
                && m["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("Finish the larger task"))
        }));
        assert!(!messages.iter().any(|m| {
            m["role"] == "system"
                && m["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("Finish the larger task"))
        }));
    }
    drop(requests);
    state.shutdown.cancel();
    driver.await.unwrap();
}

#[tokio::test]
async fn goal_driver_pause_during_run_prevents_next_admission_and_keeps_usage() {
    use voyage_protocol::goals::*;
    let gate = Arc::new(tokio::sync::Notify::new());
    let (_root, state, provider) = configured(vec![measured_reply().held(gate.clone())]).await;
    automatic_goal(&state, GoalLimits::default()).await;
    let driver = tokio::spawn(super::goals::drive(state.clone()));
    provider.wait_requests(1).await;
    let goal = state.owner.goal().await.unwrap().goal.unwrap();
    let response = call(
        &state,
        RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: state.owner.snapshot().await.unwrap().revision,
            expires_at_ms: deadline(),
            action: GoalAction::Pause { goal_id: goal.id },
        },
    )
    .await;
    assert!(response.error.is_none(), "{response:?}");
    gate.notify_one();
    let stopped = goal_stopped(&state).await;
    assert_eq!(stopped.status, GoalStatus::Paused);
    assert_eq!(
        (
            stopped.usage.runs,
            stopped.usage.input_tokens,
            stopped.usage.output_tokens
        ),
        (1, 20, 4)
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
    state.shutdown.cancel();
    driver.await.unwrap();
}

#[tokio::test]
async fn goal_driver_rechecks_authority_and_pending_input_before_dispatch() {
    use voyage_protocol::goals::*;
    for changed_authority in [false, true] {
        let (_root, state, provider) = configured(vec![]).await;
        automatic_goal(&state, GoalLimits::default()).await;
        if changed_authority {
            let goal = state.owner.goal().await.unwrap().goal.unwrap();
            state
                .owner
                .update_goal(
                    crate::attachment::journal::GoalAuthority {
                        installation_id: Uuid::new_v4(),
                        principal_id: state.actor.principal_id,
                        grant: None,
                    },
                    RuntimeCommand::GoalUpdate {
                        command_id: Uuid::new_v4(),
                        expected_revision: state.owner.snapshot().await.unwrap().revision,
                        expires_at_ms: deadline(),
                        action: GoalAction::Pause { goal_id: goal.id },
                    },
                )
                .await
                .unwrap();
            state
                .owner
                .update_goal(
                    crate::attachment::journal::GoalAuthority {
                        installation_id: Uuid::new_v4(),
                        principal_id: state.actor.principal_id,
                        grant: None,
                    },
                    RuntimeCommand::GoalUpdate {
                        command_id: Uuid::new_v4(),
                        expected_revision: state.owner.snapshot().await.unwrap().revision,
                        expires_at_ms: deadline(),
                        action: GoalAction::Resume { goal_id: goal.id },
                    },
                )
                .await
                .unwrap();
        } else {
            let input = submit("Pending human input");
            state
                .owner
                .bind_process_command(
                    input.mutation_id().unwrap(),
                    state.actor.principal_id,
                    input,
                )
                .await
                .unwrap();
        }
        super::goals::advance(&state).await.unwrap();
        let stopped = state.owner.goal().await.unwrap().goal.unwrap();
        assert_eq!(
            stopped.status,
            if changed_authority {
                GoalStatus::NeedsAttention
            } else {
                GoalStatus::Active
            }
        );
        assert_eq!(
            stopped.stop_reason,
            if changed_authority {
                Some(GoalStopReason::AuthorityRevoked)
            } else {
                None
            }
        );
        assert_eq!(stopped.usage.runs, 0);
        assert!(provider.requests.lock().await.is_empty());
        super::goals::advance(&state).await.unwrap();
        assert_eq!(state.owner.goal().await.unwrap().goal.unwrap(), stopped);
    }
}

#[tokio::test]
async fn goal_driver_completes_only_after_current_run_tool_evidence() {
    use voyage_protocol::goals::*;
    // First run makes no completion claim; the next run observes a real file,
    // reports those receipts, and finishes. All inference stays on loopback.
    let mut proof = Reply::tool("read_file", json!({"path":"result.txt"}));
    proof.body = proof.body.replace("fixture-call", "proof-call");
    let mut assessment = Reply::tool(
        "goal",
        json!({"action":"report","report":{
            "outcome":"complete","summary":"The result has been verified","evidence":[{"call_id":"proof-call","conclusion":"The file contains the expected result"}]
        }}),
    );
    assessment.body = assessment.body.replace("fixture-call", "assessment-call");
    for reply in [&mut proof, &mut assessment] {
        reply.body=reply.body.replace("data: [DONE]",&format!("data: {}\n\ndata: [DONE]",json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})));
    }
    let (root, state, provider) =
        configured(vec![measured_reply(), proof, assessment, measured_reply()]).await;
    std::fs::write(root.path().join("result.txt"), "expected result\n").unwrap();
    automatic_goal_objective(
        &state,
        GoalLimits::default(),
        "Verify result.txt contains expected result and report the observation.",
    )
    .await;
    let driver = tokio::spawn(super::goals::drive(state.clone()));
    let goal = goal_stopped(&state).await;
    assert_eq!(
        goal.status,
        GoalStatus::Complete,
        "{goal:?} history={}",
        serde_json::to_string(&state.owner.snapshot().await.unwrap().session.messages).unwrap()
    );
    assert_eq!(goal.usage.runs, 2);
    assert_eq!(
        (
            goal.usage.input_tokens,
            goal.usage.output_tokens,
            goal.usage.unmeasured_runs
        ),
        (80, 16, 0)
    );
    assert!(goal.assessment.is_some());
    assert_eq!(provider.requests.lock().await.len(), 4);
    state.shutdown.cancel();
    driver.await.unwrap();
}

#[tokio::test]
async fn goal_driver_rejects_invented_completion_and_stops_without_progress() {
    use voyage_protocol::goals::*;
    let mut report = Reply::tool(
        "goal",
        json!({"action":"report","report":{
            "outcome":"complete","summary":"I claim success without a check","evidence":[{"call_id":"invented","conclusion":"Unsupported claim"}]
        }}),
    );
    report.body=report.body.replace("data: [DONE]",&format!("data: {}\n\ndata: [DONE]",json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})));
    let (_root, state, provider) = configured(vec![report, measured_reply()]).await;
    automatic_goal(
        &state,
        GoalLimits {
            no_progress_runs: 1,
            ..Default::default()
        },
    )
    .await;
    let driver = tokio::spawn(super::goals::drive(state.clone()));
    let goal = goal_stopped(&state).await;
    assert_eq!(goal.status, GoalStatus::Limited, "{goal:?}");
    assert_eq!(goal.stop_reason, Some(GoalStopReason::NoProgress));
    assert!(goal.assessment.is_none());
    assert_eq!(goal.usage.runs, 1);
    assert_eq!(provider.requests.lock().await.len(), 2);
    let saved = state.owner.snapshot().await.unwrap();
    assert!(
        saved
            .session
            .messages
            .iter()
            .any(|m| m.role == crate::model::Role::Tool
                && m.tool_success == Some(false)
                && m.content.contains("Goal evidence is missing"))
    );
    state.shutdown.cancel();
    driver.await.unwrap();
}

#[tokio::test]
async fn conversational_root_goal_creation_continues_to_tool_free_full_assessment() {
    use voyage_protocol::goals::*;
    // Real root tool dispatch and owner driver with deterministic loopback replies:
    // no provider account, live inference or model budget is used.
    let mut create = Reply::tool(
        "goal",
        json!({"action":"create","objective":"Draft and verify the full explanation"}),
    );
    create.body = create.body.replace("fixture-call", "goal-create");
    let mut complete = Reply::tool(
        "goal",
        json!({"action":"status","report":{
            "outcome":"complete","summary":"Every requested explanation section has been drafted and audited"
        }}),
    );
    complete.body = complete.body.replace("fixture-call", "goal-complete");
    for reply in [&mut create, &mut complete] {
        reply.body=reply.body.replace("data: [DONE]",&format!("data: {}\n\ndata: [DONE]",json!({"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":4,"total_tokens":24}})));
    }
    let (_root, state, provider) = configured(vec![
        create,
        measured_reply(),
        measured_reply(),
        complete,
        measured_reply(),
    ])
    .await;
    state.config.write().await.vessel.enabled = false;
    let driver = tokio::spawn(super::goals::drive(state.clone()));
    let response = call(
        &state,
        submit("Keep working until the full explanation has been drafted and verified."),
    )
    .await;
    assert!(response.error.is_none(), "{response:?}");
    let goal = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(goal) = state.owner.goal().await.unwrap().goal
                && goal.status == GoalStatus::Complete
                && state.active.lock().await.is_none()
            {
                break goal;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("root-created Goal did not complete");
    assert_eq!(goal.objective, "Draft and verify the full explanation");
    assert_eq!(goal.limits, GoalLimits::default());
    assert_eq!(goal.usage.runs, 3);
    assert_eq!(goal.usage.unmeasured_runs, 0);
    assert!(goal.assessment.as_ref().unwrap().report.evidence.is_empty());
    assert!(!goal.continuation_authorized);
    assert_eq!(provider.requests.lock().await.len(), 5);
    state.shutdown.cancel();
    driver.await.unwrap();
}
