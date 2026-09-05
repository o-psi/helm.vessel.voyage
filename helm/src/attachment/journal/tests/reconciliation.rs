use super::*;
use crate::model::ToolCall;

fn interrupted() -> (
    tempfile::TempDir,
    Journal,
    Session,
    TurnAdmission,
    ExecutionGuard,
    RunRecord,
    LocalReconcileRequest,
) {
    let (dir, mut journal, session, admission) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &admission, 1).unwrap().run;
    journal.register_local_cleanup(&guard, run.id).unwrap();
    journal.mark_running(&guard, run.id).unwrap();
    let mut call = Message::new(Role::Assistant, "canonical prefix");
    call.tool_calls = vec![ToolCall {
        id: "call-1".into(),
        name: "subagent.wait".into(),
        arguments: serde_json::json!({"id":"child"}),
    }];
    call.provider_state = Some(serde_json::json!({"version":1,"opaque":"PRIVATE_PROVIDER_CANARY"}));
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    messages.push(call);
    journal
        .checkpoint_canonical(
            &guard,
            run.id,
            &messages,
            &Usage {
                input_tokens: 17,
                output_tokens: 9,
            },
        )
        .unwrap();
    journal
        .append_text(&guard, run.id, "provisional partial")
        .unwrap();
    journal
        .finish(&guard, run.id, RunState::Cancelled, Some("cancelled"), None)
        .unwrap();
    journal
        .confirm_local_cleanup_observed(&guard, run.id)
        .unwrap();
    let request = LocalReconcileRequest {
        session_id: session.id,
        run_id: run.id,
        installation_id: admission.machine_id,
        principal_id: admission.principal_id,
        expected_revision: journal.load_session(session.id).unwrap().revision,
    };
    (dir, journal, session, admission, guard, run, request)
}

#[test]
fn explicit_reconciliation_preserves_prefix_provider_state_usage_and_terminal_without_replay() {
    let (_dir, mut journal, session, mut admission, guard, run, request) = interrupted();
    let before = journal.load_session(session.id).unwrap();
    let prefix = serde_json::to_value(&before.session.messages).unwrap();
    let events_before: i64 = journal
        .connection
        .query_row(
            "SELECT next_sequence FROM sessions WHERE id=?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    admission.command_id = Uuid::new_v4();
    admission.expected_revision = before.revision;
    assert!(journal.admit_turn(&guard, &admission, 1).is_err());
    let outcome = journal.reconcile_local_tools(&guard, &request).unwrap();
    assert!(!outcome.duplicate);
    assert_eq!(outcome.tool_call_ids, ["call-1"]);
    assert_eq!(outcome.revision, before.revision + 1);
    let current = journal.load_session(session.id).unwrap();
    assert_eq!(
        serde_json::to_value(&current.session.messages[..before.session.messages.len()]).unwrap(),
        prefix
    );
    assert_eq!(
        serde_json::to_value(&current.session.usage).unwrap(),
        serde_json::to_value(&before.session.usage).unwrap()
    );
    let result = current.session.messages.last().unwrap();
    assert_eq!(result.role, Role::Tool);
    assert_eq!(result.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(result.tool_success, Some(false));
    assert!(result.content.contains("unknown") && result.content.contains("not retried"));
    assert!(result.provider_state.is_none());
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Cancelled);
    assert_eq!(
        journal.run(run.id).unwrap().partial_text,
        "provisional partial"
    );
    assert_eq!(
        journal
            .connection
            .query_row(
                "SELECT next_sequence FROM sessions WHERE id=?1",
                [session.id.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        events_before
    );
    admission.expected_revision = current.revision;
    journal.admit_turn(&guard, &admission, 1).unwrap();
    let retry = journal.reconcile_local_tools(&guard, &request).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.revision, outcome.revision);
    assert_eq!(retry.tool_call_ids, outcome.tool_call_ids);
}

#[test]
fn reconciliation_requires_exact_actor_guard_revision_and_resolved_cleanup() {
    let (_dir, mut journal, session, _admission, guard, run, correct) = interrupted();
    for field in 0..5 {
        let mut wrong = correct.clone();
        match field {
            0 => wrong.session_id = Uuid::new_v4(),
            1 => wrong.run_id = Uuid::new_v4(),
            2 => wrong.installation_id = Uuid::new_v4(),
            3 => wrong.principal_id = Uuid::new_v4(),
            _ => wrong.expected_revision += 1,
        }
        assert!(journal.reconcile_local_tools(&guard, &wrong).is_err());
    }
    journal
        .connection
        .execute(
            "UPDATE local_cleanup_obligations SET confirmation=NULL WHERE run_id=?1",
            [run.id.to_string()],
        )
        .unwrap();
    assert!(journal.reconcile_local_tools(&guard, &correct).is_err());
    journal
        .attest_local_cleanup(
            &guard,
            run.id,
            correct.installation_id,
            correct.principal_id,
        )
        .unwrap();
    let other = Session::new(session.workspace.clone(), "fixture".into());
    journal.create_session(&other).unwrap();
    let wrong_guard = journal.acquire_execution(other.id).unwrap();
    assert!(
        journal
            .reconcile_local_tools(&wrong_guard, &correct)
            .is_err()
    );
    journal.reconcile_local_tools(&guard, &correct).unwrap();
    let mut changed = correct;
    changed.expected_revision += 1;
    assert!(journal.reconcile_local_tools(&guard, &changed).is_err());
}

#[test]
fn reconciliation_receipt_and_canonical_append_are_atomic_and_survive_reopen() {
    let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
    let before = serde_json::to_vec(&journal.load_session(session.id).unwrap().session).unwrap();
    journal.connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON local_tool_reconciliations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(journal.reconcile_local_tools(&guard, &request).is_err());
    assert_eq!(
        serde_json::to_vec(&journal.load_session(session.id).unwrap().session).unwrap(),
        before
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        request.expected_revision
    );
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_receipt;")
        .unwrap();
    let first = journal.reconcile_local_tools(&guard, &request).unwrap();
    let path = journal.directory.clone();
    drop(guard);
    drop(journal);
    let mut journal = Journal::open(path).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let duplicate = journal.reconcile_local_tools(&guard, &request).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.revision, first.revision);
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        first.revision
    );
}

#[test]
fn ambiguous_or_unbounded_tool_histories_fail_without_rewriting() {
    for variant in 0..6 {
        let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
        let mut current = journal.load_session(session.id).unwrap();
        let call = current.session.messages.last_mut().unwrap();
        match variant {
            0 => call.tool_calls[0].id = String::new(),
            1 => call.tool_calls.push(call.tool_calls[0].clone()),
            2 => call.tool_calls[0].id = "x".repeat(1025),
            3 => call.tool_calls[0].id = "bad\nID".into(),
            4 => current
                .session
                .messages
                .push(Message::new(Role::Assistant, "unsafe later assistant")),
            _ => {
                let call = call.tool_calls[0].clone();
                current.session.messages.last_mut().unwrap().tool_calls = (0..129)
                    .map(|i| ToolCall {
                        id: format!("call-{i}"),
                        ..call.clone()
                    })
                    .collect();
            }
        }
        journal
            .connection
            .execute(
                "UPDATE sessions SET state=?1 WHERE id=?2",
                params![
                    serde_json::to_string(&current.session).unwrap(),
                    session.id.to_string()
                ],
            )
            .unwrap();
        let before = serde_json::to_vec(&current.session.messages).unwrap();
        assert!(
            journal.reconcile_local_tools(&guard, &request).is_err(),
            "variant {variant}"
        );
        assert_eq!(
            serde_json::to_vec(&journal.load_session(session.id).unwrap().session.messages)
                .unwrap(),
            before
        );
        assert_eq!(
            journal.load_session(session.id).unwrap().revision,
            request.expected_revision
        );
    }
}

#[test]
fn schema_five_requires_explicit_quiescent_upgrade_and_preserves_cleanup_evidence() {
    let (_dir, journal, session, _admission, guard, run, request) = interrupted();
    journal
        .connection
        .execute_batch(
            "DROP TABLE local_tool_reconciliations; UPDATE attachment_schema SET version=5;",
        )
        .unwrap();
    let path = journal.directory.clone();
    drop(guard);
    drop(journal);
    let mut journal = Journal::open(path.clone()).unwrap();
    let mut stale = Journal::open(path).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    assert!(journal.reconcile_local_tools(&guard, &request).is_err());
    assert!(journal.upgrade_quiescent().is_err());
    assert!(journal.local_cancel_requested(session.id, run.id).is_ok());
    assert!(
        journal.list_session_summaries(None, 1).unwrap().sessions[0]
            .pending_cleanup_run
            .is_none()
    );
    drop(guard);
    journal.upgrade_quiescent().unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    assert!(stale.reconcile_local_tools(&guard, &request).is_err());
    journal.reconcile_local_tools(&guard, &request).unwrap();
}

#[test]
fn sqlite_contention_and_failed_canonical_write_leave_no_receipt() {
    let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
    let mut other = Journal::open(journal.directory.clone()).unwrap();
    let tx = other
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let error = journal.reconcile_local_tools(&guard, &request).unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    tx.rollback().unwrap();
    journal.connection.execute_batch("CREATE TRIGGER fail_canonical BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(journal.reconcile_local_tools(&guard, &request).is_err());
    assert_eq!(
        journal
            .connection
            .query_row("SELECT count(*) FROM local_tool_reconciliations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        request.expected_revision
    );
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_canonical;")
        .unwrap();
    journal.reconcile_local_tools(&guard, &request).unwrap();
    assert!(
        other
            .reconcile_local_tools(&guard, &request)
            .unwrap()
            .duplicate
    );
}

#[test]
fn partial_results_are_preserved_and_only_unresolved_calls_are_appended_in_call_order() {
    let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
    let mut current = journal.load_session(session.id).unwrap();
    let calls = &mut current.session.messages.last_mut().unwrap().tool_calls;
    calls.push(ToolCall {
        id: "call-2".into(),
        name: "read".into(),
        arguments: serde_json::json!({}),
    });
    calls.push(ToolCall {
        id: "call-3".into(),
        name: "write".into(),
        arguments: serde_json::json!({}),
    });
    current
        .session
        .messages
        .push(Message::tool("call-2", "actual observed result"));
    journal
        .connection
        .execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            params![
                serde_json::to_string(&current.session).unwrap(),
                session.id.to_string()
            ],
        )
        .unwrap();
    let prefix = serde_json::to_value(&current.session.messages).unwrap();
    let receipt = journal.reconcile_local_tools(&guard, &request).unwrap();
    assert_eq!(receipt.tool_call_ids, ["call-1", "call-3"]);
    let after = journal.load_session(session.id).unwrap();
    assert_eq!(
        serde_json::to_value(&after.session.messages[..current.session.messages.len()]).unwrap(),
        prefix
    );
    assert_eq!(
        after.session.messages.len(),
        current.session.messages.len() + 2
    );
}

#[test]
fn active_no_cleanup_no_pending_and_stale_run_selections_fail_closed() {
    for variant in 0..4 {
        let (_dir, mut journal, session, _admission, guard, run, request) = interrupted();
        match variant {
            0 => {
                let mut row = journal.run(run.id).unwrap();
                row.state = RunState::Running;
                journal
                    .connection
                    .execute(
                        "UPDATE runs SET record=?1,active=1 WHERE id=?2",
                        params![serde_json::to_string(&row).unwrap(), run.id.to_string()],
                    )
                    .unwrap();
            }
            1 => {
                journal
                    .connection
                    .execute("DELETE FROM local_cleanup_obligations", [])
                    .unwrap();
            }
            2 => {
                let mut current = journal.load_session(session.id).unwrap();
                current.session.messages.push(Message::tool_result(
                    "call-1",
                    "already failed",
                    false,
                ));
                journal
                    .connection
                    .execute(
                        "UPDATE sessions SET state=?1 WHERE id=?2",
                        params![
                            serde_json::to_string(&current.session).unwrap(),
                            session.id.to_string()
                        ],
                    )
                    .unwrap();
            }
            _ => {
                journal
                    .connection
                    .execute(
                        "UPDATE events SET event=json_set(event,'$.run_id',?1) WHERE session_id=?2",
                        params![Uuid::new_v4().to_string(), session.id.to_string()],
                    )
                    .unwrap();
            }
        }
        assert!(
            journal.reconcile_local_tools(&guard, &request).is_err(),
            "variant {variant}"
        );
        assert_eq!(
            journal.load_session(session.id).unwrap().revision,
            request.expected_revision
        );
    }
}

#[tokio::test]
async fn reconciled_history_continues_through_three_native_http_provider_adapters() {
    use crate::provider::{AnthropicProvider, OpenAiProvider, OpenAiResponsesProvider, Provider};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for kind in 0..3 {
        let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
        let mut current = journal.load_session(session.id).unwrap();
        current.session.messages.last_mut().unwrap().provider_state = Some(serde_json::json!({
            "kind":"openai_responses_replay","version":1,"items":[
                {"type":"reasoning","id":"rs_1","encrypted_content":"PRIVATE_PROVIDER_CANARY","summary":[]},
                {"type":"function_call","call_id":"call-1","name":"subagent.wait","arguments":"{\"id\":\"child\"}"}
            ]
        }));
        journal
            .connection
            .execute(
                "UPDATE sessions SET state=?1 WHERE id=?2",
                params![
                    serde_json::to_string(&current.session).unwrap(),
                    session.id.to_string()
                ],
            )
            .unwrap();
        journal.reconcile_local_tools(&guard, &request).unwrap();
        let messages = journal.load_session(session.id).unwrap().session.messages;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(5),async move {
                let (mut socket,_)=listener.accept().await.unwrap();
                let mut bytes=Vec::new();let mut chunk=[0u8;4096];
                let (offset,length)=loop {
                    let n=socket.read(&mut chunk).await.unwrap();assert!(n>0);bytes.extend_from_slice(&chunk[..n]);assert!(bytes.len()<65536);
                    if let Some(end)=bytes.windows(4).position(|window|window==b"\r\n\r\n") {
                        let headers=std::str::from_utf8(&bytes[..end]).unwrap();
                        let length=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|value|value.trim().parse::<usize>().unwrap())).unwrap();
                        break(end+4,length)
                    }
                };
                while bytes.len()<offset+length { let n=socket.read(&mut chunk).await.unwrap();assert!(n>0);bytes.extend_from_slice(&chunk[..n]);assert!(bytes.len()<65536); }
                let request:serde_json::Value=serde_json::from_slice(&bytes[offset..offset+length]).unwrap();
                let response=match kind {
                    0=>serde_json::json!({"choices":[{"message":{"role":"assistant","content":"continued"}}],"usage":{}}),
                    1=>serde_json::json!({"content":[{"type":"text","text":"continued"}],"usage":{}}),
                    _=>serde_json::json!({"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"continued"}]}],"usage":{}}),
                }.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
                request
            }).await.unwrap()
        });
        let provider: Box<dyn Provider> = match kind {
            0 => Box::new(OpenAiProvider::new("fixture".into(), Some(endpoint))),
            1 => Box::new(AnthropicProvider::new("fixture".into(), Some(endpoint))),
            _ => Box::new(OpenAiResponsesProvider::new(
                "fixture".into(),
                Some(endpoint),
            )),
        };
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            provider.complete(crate::model::ModelRequest {
                model: "fixture".into(),
                messages,
                tools: vec![],
                temperature: None,
                max_tokens: None,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.message.content, "continued");
        let body = server.await.unwrap();
        if kind == 2 {
            let input = body["input"].as_array().unwrap();
            assert!(
                input
                    .iter()
                    .any(|item| item["encrypted_content"] == "PRIVATE_PROVIDER_CANARY")
            );
            let outputs: Vec<_> = input
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .collect();
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0]["call_id"], "call-1");
            assert!(outputs[0]["output"].as_str().unwrap().contains("unknown"));
        } else if kind == 1 {
            let last = body["messages"].as_array().unwrap().last().unwrap();
            assert_eq!(last["content"][0]["type"], "tool_result");
            assert_eq!(last["content"][0]["tool_use_id"], "call-1");
            assert!(
                last["content"][0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("unknown")
            );
        } else {
            let last = body["messages"].as_array().unwrap().last().unwrap();
            assert_eq!(last["role"], "tool");
            assert_eq!(last["tool_call_id"], "call-1");
            assert!(last["content"].as_str().unwrap().contains("unknown"));
        }
    }
}

#[test]
fn exact_call_count_and_id_size_limits_succeed_with_bounded_receipt() {
    let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
    let mut current = journal.load_session(session.id).unwrap();
    current.session.messages.last_mut().unwrap().tool_calls = (0..128)
        .map(|index| ToolCall {
            id: format!("{index:03}{}", "x".repeat(1021)),
            name: "fixture".into(),
            arguments: serde_json::json!({}),
        })
        .collect();
    journal
        .connection
        .execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            params![
                serde_json::to_string(&current.session).unwrap(),
                session.id.to_string()
            ],
        )
        .unwrap();
    let outcome = journal.reconcile_local_tools(&guard, &request).unwrap();
    assert_eq!(outcome.tool_call_ids.len(), 128);
    assert!(outcome.tool_call_ids.iter().all(|id| id.len() == 1024));
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        current.session.messages.len() + 128
    );
    let size: i64 = journal
        .connection
        .query_row(
            "SELECT length(CAST(record AS BLOB)) FROM local_tool_reconciliations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(size <= 256 * 1024);
}

#[test]
fn malformed_or_oversized_existing_receipts_never_become_new_reconciliation() {
    for variant in 0..4 {
        let (_dir, mut journal, session, _admission, guard, _run, request) = interrupted();
        journal.reconcile_local_tools(&guard, &request).unwrap();
        let before =
            serde_json::to_vec(&journal.load_session(session.id).unwrap().session).unwrap();
        match variant {
            0 => {
                journal
                    .connection
                    .execute("UPDATE local_tool_reconciliations SET record='{}'", [])
                    .unwrap();
            }
            1 => {
                journal
                    .connection
                    .execute(
                        "UPDATE local_tool_reconciliations SET record=?1",
                        ["x".repeat(256 * 1024 + 1)],
                    )
                    .unwrap();
            }
            2 => {
                journal.connection.execute("UPDATE local_tool_reconciliations SET record=json_set(record,'$.tool_call_ids',json('[\"call-1\",\"call-1\"]'))",[]).unwrap();
            }
            _ => {
                journal.connection.execute("UPDATE local_tool_reconciliations SET record=json_set(record,'$.revision',0)",[]).unwrap();
            }
        }
        assert!(journal.reconcile_local_tools(&guard, &request).is_err());
        assert_eq!(
            serde_json::to_vec(&journal.load_session(session.id).unwrap().session).unwrap(),
            before
        );
    }
}

#[test]
fn an_older_terminal_run_cannot_reconcile_calls_created_by_a_later_run() {
    let (_dir, mut journal, session, mut admission, guard, _run, mut request) = interrupted();
    // Resolve first run using an actual known result, without a reconciliation
    // receipt, then create a later run whose unresolved call belongs only to it.
    let mut current = journal.load_session(session.id).unwrap();
    current.session.messages.push(Message::tool_result(
        "call-1",
        "actual failure result",
        false,
    ));
    journal
        .connection
        .execute(
            "UPDATE sessions SET state=?1 WHERE id=?2",
            params![
                serde_json::to_string(&current.session).unwrap(),
                session.id.to_string()
            ],
        )
        .unwrap();
    admission.command_id = Uuid::new_v4();
    admission.expected_revision = current.revision;
    let later = journal.admit_turn(&guard, &admission, 1).unwrap().run;
    journal.register_local_cleanup(&guard, later.id).unwrap();
    journal.mark_running(&guard, later.id).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let mut call = Message::new(Role::Assistant, "");
    call.tool_calls.push(ToolCall {
        id: "later-call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({}),
    });
    messages.push(call);
    journal
        .checkpoint_canonical(&guard, later.id, &messages, &Usage::default())
        .unwrap();
    journal
        .finish(
            &guard,
            later.id,
            RunState::Cancelled,
            Some("cancelled"),
            None,
        )
        .unwrap();
    journal
        .confirm_local_cleanup_observed(&guard, later.id)
        .unwrap();
    request.expected_revision = journal.load_session(session.id).unwrap().revision;
    assert!(
        journal
            .reconcile_local_tools(&guard, &request)
            .unwrap_err()
            .to_string()
            .contains("latest run")
    );
    request.run_id = later.id;
    assert_eq!(
        journal
            .reconcile_local_tools(&guard, &request)
            .unwrap()
            .tool_call_ids,
        ["later-call"]
    );
}

#[test]
fn duplicate_receipt_rejects_revision_outside_sqlite_range() {
    let (_dir, mut journal, session, _admission, guard, _run, mut request) = interrupted();
    journal.reconcile_local_tools(&guard, &request).unwrap();
    request.expected_revision = i64::MAX as u64;
    let corrupt = serde_json::json!({"request":request,"revision":(i64::MAX as u64)+1,"tool_call_ids":["call-1"]});
    journal
        .connection
        .execute(
            "UPDATE local_tool_reconciliations SET record=?1",
            [corrupt.to_string()],
        )
        .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    assert!(journal.reconcile_local_tools(&guard, &request).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
}
