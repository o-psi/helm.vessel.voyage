use super::*;
use crate::provider::goal_meter::{AllocationRequest, GoalMeter, Observer, RequestObservation};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
struct Accounting {
    parent: Uuid,
    run: Uuid,
    allocated: Mutex<Vec<ExecutionBudget>>,
    settled: Mutex<Vec<ExecutionUsage>>,
    late: Mutex<Option<ExecutionUsage>>,
}
#[async_trait]
impl Observer for Accounting {
    async fn record(&self, _: RequestObservation) -> anyhow::Result<()> {
        Ok(())
    }
    async fn allocate(&self, r: AllocationRequest) -> anyhow::Result<ExecutionBudget> {
        let b = ExecutionBudget {
            command_id: r.command_id,
            session_id: r.session_id,
            parent_session_id: self.parent,
            parent_run_id: self.run,
            tokens: r.tokens,
            elapsed_ms: r.elapsed_ms,
            expires_at_ms: r.expires_at_ms,
        };
        self.allocated.lock().unwrap().push(b.clone());
        Ok(b)
    }
    async fn settle_allocation(&self, _: Uuid, u: ExecutionUsage) -> anyhow::Result<()> {
        self.settled.lock().unwrap().push(u);
        Ok(())
    }
}
struct Peer {
    root: tempfile::TempDir,
    transport: transport::Transport,
    commands: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn peer(supports_budget: bool, history: bool, accounting: Arc<Accounting>) -> Peer {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let destination = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let child_run = Uuid::new_v4();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let saved = commands.clone();
    let task = tokio::spawn(async move {
        let mut budgets = std::collections::HashMap::<Uuid, ExecutionBudget>::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (start, length) = loop {
                let mut chunk = [0u8; 4096];
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    return;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break (end + 4, length);
                    }
                }
            };
            let envelope: Value = serde_json::from_slice(&bytes[start..start + length]).unwrap();
            let wire = &envelope["command"];
            let payload = match wire["op"].as_str().unwrap() {
                "capabilities" => {
                    json!({"vessel_id":destination,"features":if supports_budget {vec!["start_settings","execution_budget"]} else {vec!["start_settings"]}})
                }
                "start_settings" => {
                    assert_eq!(
                        accounting.allocated.lock().unwrap().len(),
                        1,
                        "intent must precede process creation"
                    );
                    json!({"session_id":wire["session_id"],"incarnation":incarnation})
                }
                "snapshot" => json!({"revision":0,"run":null,"pending_cleanup_run":null}),
                "submit" => {
                    let budget: ExecutionBudget =
                        serde_json::from_value(wire["budget"].clone()).unwrap();
                    assert_eq!(
                        accounting.allocated.lock().unwrap().as_slice(),
                        &[budget.clone()],
                        "intent must precede child submission"
                    );
                    assert_eq!(
                        budget.session_id.to_string(),
                        wire["session_id"].as_str().unwrap()
                    );
                    budgets.insert(budget.command_id, budget);
                    json!({"command_id":wire["command_id"],"run_id":child_run,"status":"accepted"})
                }
                "receipt" => {
                    let command: Uuid = serde_json::from_value(wire["command_id"].clone()).unwrap();
                    let late = accounting.late.lock().unwrap().clone();
                    if let Some(usage) = late {
                        json!({"command_id":command,"run_id":usage.run_id,"status":"accepted","state":"completed","execution_usage":usage})
                    } else {
                        match budgets.get(&command) {
                            Some(budget) => {
                                json!({"command_id":command,"run_id":child_run,"status":"accepted","state":"completed","execution_usage":ExecutionUsage {budget:budget.clone(),session_id:budget.session_id,run_id:child_run,input_tokens:9,output_tokens:3,elapsed_ms:10,complete:true,cleanup_observed:true}})
                            }
                            None => json!({"command_id":command,"status":"unknown"}),
                        }
                    }
                }
                op => panic!("unexpected fixture operation: {op}"),
            };
            let result = if matches!(wire["op"].as_str(), Some("capabilities" | "start_settings")) {
                payload
            } else {
                json!({"session_id":wire["session_id"],"incarnation":incarnation,"result":payload})
            };
            saved.lock().unwrap().push(wire.clone());
            let denied = wire["op"] == "snapshot" && !history;
            let body = json!({"protocol":1,"result":result,"error":if denied {Some("history scope denied")} else {None},"outcome_unknown":false})
                .to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    std::fs::write(
        root.path().join("process-http.json"),
        json!({"endpoint":endpoint,"token":"a".repeat(64)}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(
        root.path().join("process-http.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut transport = transport::Transport::open(root.path(), None).unwrap();
    let journal = root.path().join("wire");
    std::fs::create_dir(&journal).unwrap();
    transport.journal(journal);
    Peer {
        root,
        transport,
        commands,
        task,
    }
}
fn accounting() -> Arc<Accounting> {
    Arc::new(Accounting {
        late: Mutex::new(None),
        parent: Uuid::new_v4(),
        run: Uuid::new_v4(),
        allocated: Mutex::new(vec![]),
        settled: Mutex::new(vec![]),
    })
}
#[tokio::test]
async fn goal_vessel_create_and_submit_publish_budget_before_effects_and_import_exact_receipt() {
    for create in [false, true] {
        let accounting = accounting();
        let peer = peer(true, true, accounting.clone()).await;
        let meter =
            GoalMeter::with_observer(100, Duration::from_secs(10), Some(accounting.clone()));
        let config = crate::Config {
            goal_meter: Some(meter.clone()),
            ..Default::default()
        };
        let ctx = crate::tools::reliability_tests::context(peer.root.path());
        let session = Uuid::new_v4();
        let command = Uuid::new_v4();
        let value = if create {
            json!({"action":"create","session_id":session,"command_id":command,"workspace":peer.root.path(),"task":"bounded child"})
        } else {
            json!({"action":"submit","session_id":session,"command_id":command,"expected_revision":0,"prompt":"bounded child"})
        };
        let action: Action = serde_json::from_value(value.clone()).unwrap();
        let result = perform(action, &peer.transport, &ctx, None, Some(&config))
            .await
            .unwrap();
        assert!(result.to_string().contains("accepted"), "{result}");
        tokio::time::timeout(
            Duration::from_secs(4),
            meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
        )
        .await
        .unwrap();
        let measured = meter.measurement();
        assert!(measured.complete);
        assert_eq!((measured.input_tokens, measured.output_tokens), (9, 3));
        assert_eq!(accounting.allocated.lock().unwrap()[0].tokens, 50);
        assert_eq!(accounting.settled.lock().unwrap().len(), 1);
        assert_eq!(
            peer.commands
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c["op"] == "submit")
                .count(),
            1
        );
        let mut injected = value;
        injected["budget"] = json!({"tokens":999999});
        assert!(
            crate::tools::schema::CompiledSchema::compile(&input_schema())
                .unwrap()
                .validate(&injected)
                .is_err()
        );
    }
}
#[tokio::test]
async fn goal_vessel_refuses_legacy_peer_and_unbudgeted_steering_before_effects() {
    let accounting = accounting();
    let peer = peer(false, true, accounting.clone()).await;
    let config = crate::Config {
        goal_meter: Some(GoalMeter::with_observer(
            100,
            Duration::from_secs(10),
            Some(accounting.clone()),
        )),
        ..Default::default()
    };
    let ctx = crate::tools::reliability_tests::context(peer.root.path());
    let action:Action=serde_json::from_value(json!({"action":"submit","session_id":Uuid::new_v4(),"command_id":Uuid::new_v4(),"expected_revision":0,"prompt":"bounded child"})).unwrap();
    assert!(
        perform(action, &peer.transport, &ctx, None, Some(&config))
            .await
            .is_err()
    );
    assert!(accounting.allocated.lock().unwrap().is_empty());
    assert!(
        peer.commands
            .lock()
            .unwrap()
            .iter()
            .all(|c| c["op"] == "capabilities")
    );
    let action:Action=serde_json::from_value(json!({"action":"steer","session_id":Uuid::new_v4(),"command_id":Uuid::new_v4(),"incarnation":Uuid::new_v4(),"run_id":Uuid::new_v4(),"expected_revision":0,"prompt":"unbounded"})).unwrap();
    assert!(
        perform(action, &peer.transport, &ctx, None, Some(&config))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn goal_child_submission_requires_history_before_allocating_or_dispatching() {
    let accounting = accounting();
    let peer = peer(true, false, accounting.clone()).await;
    let config = crate::Config {
        goal_meter: Some(GoalMeter::with_observer(
            100,
            Duration::from_secs(10),
            Some(accounting.clone()),
        )),
        ..Default::default()
    };
    let ctx = crate::tools::reliability_tests::context(peer.root.path());
    let action:Action=serde_json::from_value(json!({"action":"submit","session_id":Uuid::new_v4(),"command_id":Uuid::new_v4(),"expected_revision":0,"prompt":"requires accounting"})).unwrap();
    assert!(
        perform(action, &peer.transport, &ctx, None, Some(&config))
            .await
            .is_err()
    );
    assert!(accounting.allocated.lock().unwrap().is_empty());
    assert!(
        peer.commands
            .lock()
            .unwrap()
            .iter()
            .all(|c| c["op"] == "capabilities" || c["op"] == "snapshot")
    );
}

#[tokio::test]
async fn goal_reconciliation_reopens_owner_and_reads_exact_receipt_without_replay() {
    use crate::attachment::{
        journal::{Journal, RunState, TurnAdmission},
        runtime::ManagedSessionOwner,
    };
    let accounting = accounting();
    let peer = peer(true, true, accounting.clone()).await;
    let caps = peer
        .transport
        .exchange(VesselCommand::Capabilities)
        .await
        .unwrap();
    let target: Uuid = serde_json::from_value(caps["vessel_id"].clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let mut j = Journal::open(directory.clone()).unwrap();
    let session =
        crate::session::Session::new(root.path().canonicalize().unwrap(), "fixture".into());
    j.create_session(&session).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    j.initialize_process_commands(&guard).unwrap();
    j.initialize_lifecycle(&guard).unwrap();
    let budget = ExecutionBudget {
        command_id: Uuid::new_v4(),
        session_id: session.id,
        parent_session_id: Uuid::new_v4(),
        parent_run_id: Uuid::new_v4(),
        tokens: 100,
        elapsed_ms: 10_000,
        expires_at_ms: 11_000,
    };
    let run = j
        .admit_turn(
            &guard,
            &TurnAdmission {
                budget: Some(budget.clone()),
                coordination: None,
                operator_name: None,
                command_id: budget.command_id,
                machine_id: Uuid::new_v4(),
                principal_id: Uuid::new_v4(),
                session_id: session.id,
                expected_revision: j.load_session(session.id).unwrap().revision,
                expires_at_ms: 11_000,
                prompt: "fixture".into(),
                parts: vec![],
            },
            1000,
        )
        .unwrap()
        .run;
    let inc = Uuid::new_v4();
    j.begin_delegated_meter(&guard, &budget, inc, 1000).unwrap();
    let child = j
        .allocate_goal_child(
            &guard,
            run.command_id,
            inc,
            AllocationRequest {
                command_id: Uuid::new_v4(),
                destination: target,
                session_id: Uuid::new_v4(),
                tokens: 20,
                elapsed_ms: 1000,
                expires_at_ms: 2000,
            },
            1000,
        )
        .unwrap();
    j.finish(
        &guard,
        run.id,
        RunState::Interrupted,
        Some("fixture death"),
        None,
    )
    .unwrap();
    j.settle_delegated_run(&guard, run.id, None, true, 2000)
        .unwrap();
    drop(guard);
    drop(j);
    let owner = ManagedSessionOwner::open(directory, session.id)
        .await
        .unwrap();
    let mut config = crate::Config::default();
    config.vessel.local_directory = Some(peer.root.path().to_path_buf());
    let original = owner
        .process_receipt(run.command_id)
        .await
        .unwrap()
        .unwrap();
    let pending = reconcile_goal_allocations(&owner, &config, 0, 16)
        .await
        .unwrap();
    assert_eq!(pending["allocations"][0]["observed"], false);
    let usage = ExecutionUsage {
        budget: child.clone(),
        session_id: child.session_id,
        run_id: Uuid::new_v4(),
        input_tokens: 9,
        output_tokens: 3,
        elapsed_ms: 100,
        complete: true,
        cleanup_observed: true,
    };
    let mut wrong = usage.clone();
    wrong.session_id = Uuid::new_v4();
    *accounting.late.lock().unwrap() = Some(wrong);
    let pending = reconcile_goal_allocations(&owner, &config, 0, 16)
        .await
        .unwrap();
    assert_eq!(pending["allocations"][0]["observed"], false);
    *accounting.late.lock().unwrap() = Some(usage);
    let observed = reconcile_goal_allocations(&owner, &config, 0, 16)
        .await
        .unwrap();
    assert_eq!(observed["allocations"][0]["usage_updated"], true);
    assert_eq!(observed["effects_replayed"], false);
    assert_eq!(observed["continuation_restored"], false);
    let replay = reconcile_goal_allocations(&owner, &config, 0, 16)
        .await
        .unwrap();
    assert_eq!(replay["allocations"][0]["observed"], true);
    assert_eq!(replay["allocations"][0]["usage_updated"], false);
    let receipt = owner
        .process_receipt(run.command_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt["execution_usage"], original["execution_usage"]);
    assert_eq!(receipt["execution_usage_observed"]["input_tokens"], 9);
    assert_eq!(receipt["execution_usage_observed"]["output_tokens"], 3);
    assert_eq!(receipt["execution_usage_observed"]["complete"], false);
    assert!(
        peer.commands
            .lock()
            .unwrap()
            .iter()
            .all(|c| c["op"] == "capabilities" || c["op"] == "receipt")
    );
    assert!(
        reconcile_goal_allocations(&owner, &config, 0, 129)
            .await
            .is_err()
    );
}
