//! Whole ordinary delegation with production SQLite UsageObserver; no provider/executor.
use super::*;
use crate::{
    attachment::{
        journal::{Journal, RunState, TurnAdmission},
        runtime::ManagedSessionOwner,
    },
    provider::goal_meter::AllocationRequest,
    tools::ToolRegistry,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
const WAIT: Duration = Duration::from_secs(5);
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
struct Parent {
    _root: tempfile::TempDir,
    directory: PathBuf,
    session: Uuid,
    command: Uuid,
    run: Uuid,
    incarnation: Uuid,
    budget: ExecutionBudget,
}
impl Parent {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let session = crate::session::Session::new(root.path().into(), "owned parent".into());
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_decisions(&guard).unwrap();
        journal.initialize_assignments(&guard).unwrap();
        journal.initialize_observations(&guard).unwrap();
        journal.initialize_cleanup_progress(&guard).unwrap();
        let budget = ExecutionBudget {
            command_id: Uuid::new_v4(),
            session_id: session.id,
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 1000,
            elapsed_ms: 30000,
            expires_at_ms: (now() + 30000) as u64,
        };
        let run = journal
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
                    expected_revision: 0,
                    expires_at_ms: budget.expires_at_ms as i64,
                    prompt: "Owned delegated parent input".into(),
                    parts: vec![],
                },
                now(),
            )
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        drop(guard);
        drop(journal);
        Self {
            _root: root,
            directory,
            session: session.id,
            command: budget.command_id,
            run: run.id,
            incarnation: Uuid::new_v4(),
            budget,
        }
    }
    async fn open(&self) -> ManagedSessionOwner {
        ManagedSessionOwner::open(self.directory.clone(), self.session)
            .await
            .unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open_with_flags(
            self.directory.join("journal.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }
    fn graph(&self) -> String {
        let db = self.db();
        let names = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let mut graph = Vec::new();
        for name in names {
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            let mut stmt = db.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let count = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..count)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            graph.push(format!("{name}:{rows:?}"));
        }
        graph.join("\n")
    }
    fn allocation(&self) -> Option<(ExecutionBudget, Option<Value>)> {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT budget,dispatch FROM process_goal_allocations")
            .ok()?;
        let mut rows = statement.query([]).unwrap();
        let row = rows.next().unwrap()?;
        let budget: String = row.get(0).unwrap();
        let dispatch: Option<String> = row.get(1).unwrap();
        Some((
            serde_json::from_str(&budget).unwrap(),
            dispatch.map(|v| serde_json::from_str(&v).unwrap()),
        ))
    }
    fn retained(&self, destination: Uuid, dispatch: bool) -> ExecutionBudget {
        let mut journal = Journal::open(self.directory.clone()).unwrap();
        let guard = journal.acquire_execution(self.session).unwrap();
        journal
            .begin_delegated_meter(&guard, &self.budget, self.incarnation, now())
            .unwrap();
        let child = journal
            .allocate_goal_child(
                &guard,
                self.command,
                self.incarnation,
                AllocationRequest {
                    command_id: Uuid::new_v4(),
                    destination,
                    session_id: Uuid::new_v4(),
                    tokens: 100,
                    elapsed_ms: 5000,
                    expires_at_ms: (now() + 5000) as u64,
                },
                now(),
            )
            .unwrap();
        if dispatch {
            journal
                .dispatch_goal_child(
                    &guard,
                    self.command,
                    self.incarnation,
                    crate::provider::goal_meter::AllocationDispatch::Voyage {
                        command: Box::new(VoyageCommand::Submit {
                            budget: Some(child.clone()),
                            coordination: None,
                            command_id: child.command_id,
                            expected_revision: 3,
                            expires_at_ms: child.expires_at_ms,
                            prompt: "Exact retained child input 世界".into(),
                        }),
                    },
                )
                .unwrap();
        }
        journal
            .finish(
                &guard,
                self.run,
                RunState::Interrupted,
                Some("synthetic owner checkpoint interruption"),
                None,
            )
            .unwrap();
        journal
            .settle_delegated_run(&guard, self.run, None, true, now())
            .unwrap();
        drop(guard);
        drop(journal);
        child
    }
    fn lease_free(&self) {
        let journal = Journal::open(self.directory.clone()).unwrap();
        let guard = journal.acquire_execution(self.session).unwrap();
        drop(guard);
    }
}
#[derive(Clone)]
struct Mode {
    supports: bool,
    wait_for_submit: bool,
    idle: Value,
    receipt: Option<Value>,
    lost_submit: bool,
    deny_receipt: bool,
    destination: Uuid,
}
impl Mode {
    fn new() -> Self {
        Self {
            supports: true,
            wait_for_submit: false,
            idle: json!({"revision":3,"run":null,"pending_cleanup_run":null}),
            receipt: None,
            lost_submit: false,
            deny_receipt: false,
            destination: Uuid::new_v4(),
        }
    }
}
struct Peer {
    root: tempfile::TempDir,
    address: std::net::SocketAddr,
    mode: Arc<Mutex<Mode>>,
    calls: Arc<Mutex<Vec<Value>>>,
    stop: Option<oneshot::Sender<()>>,
    job: Option<tokio::task::JoinHandle<()>>,
}
impl Peer {
    async fn new(parent: &Parent) -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(
            root.path().join("process-http.json"),
            json!({"endpoint":format!("http://{address}"),"token":"a".repeat(64)}).to_string(),
        )
        .unwrap();
        std::fs::set_permissions(
            root.path().join("process-http.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mode = Arc::new(Mutex::new(Mode::new()));
        let state = mode.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let parentdb = parent.directory.join("journal.sqlite3");
        let (stop, mut stopped) = oneshot::channel();
        let job = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=&mut stopped=>break,Some(result)=children.join_next(),if !children.is_empty()=>result.unwrap(),accepted=listener.accept()=>{
                        let(mut socket,_)=accepted.unwrap();let state=state.clone();let calls=captured.clone();let parentdb=parentdb.clone();children.spawn(async move{
                            let mut bytes=Vec::new();let wire=tokio::time::timeout(WAIT,async{loop{let mut block=[0u8;4096];let n=socket.read(&mut block).await.unwrap();assert!(n>0);bytes.extend_from_slice(&block[..n]);assert!(bytes.len()<1024*1024);if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){let header=String::from_utf8_lossy(&bytes[..end]);assert!(header.starts_with(&format!("POST {} ",voyage_protocol::vessel::COMMAND_PATH)));assert!(header.to_ascii_lowercase().contains(&format!("authorization: bearer {}","a".repeat(64))));let length=header.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap();if bytes.len()>=end+4+length{let request:Value=serde_json::from_slice(&bytes[end+4..end+4+length]).unwrap();assert_eq!(request["protocol"],1);break request["command"].clone();}}}}).await.unwrap();
                            calls.lock().unwrap().push(wire.clone());let mode=state.lock().unwrap().clone();let op=wire["op"].as_str().unwrap();
                            let budget=||->Option<(ExecutionBudget,Option<Value>)>{let db=rusqlite::Connection::open_with_flags(&parentdb,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();let mut stmt=db.prepare("SELECT budget,dispatch FROM process_goal_allocations").ok()?;let mut rows=stmt.query([]).unwrap();let row=rows.next().unwrap()?;let b:String=row.get(0).unwrap();let d:Option<String>=row.get(1).unwrap();Some((serde_json::from_str(&b).unwrap(),d.map(|v|serde_json::from_str(&v).unwrap())))};
                            let payload=match op{
                                "capabilities"=>json!({"vessel_id":mode.destination,"features":if mode.supports{vec!["start_settings","execution_budget"]}else{vec!["start_settings"]}}),
                                "snapshot"=>mode.idle.clone(),
                                "start_settings"=>{let (b,dispatch)=budget().expect("real allocation precedes creation");assert_eq!(b.session_id.to_string(),wire["session_id"]);assert!(dispatch.is_none());json!({"session_id":wire["session_id"],"incarnation":Uuid::new_v4()})},
                                "submit"=>{let(b,dispatch)=budget().expect("real allocation precedes submission");assert_eq!(wire["budget"],serde_json::to_value(&b).unwrap());let dispatch=dispatch.expect("real frozen dispatch precedes wire");let mut dispatched_wire = wire.clone();
                            dispatched_wire.as_object_mut().unwrap().remove("session_id");
                            dispatched_wire.as_object_mut().unwrap().remove("incarnation");
                            assert_eq!(dispatch["command"], dispatched_wire);
                            let exact_usage = usage(b,true,true);
                            state.lock().unwrap().receipt=Some(json!({"status":"accepted",
                                "command_id":exact_usage.budget.command_id,"run_id":exact_usage.run_id,
                                "execution_usage":exact_usage}));
                            if mode.lost_submit{return;}
                            json!({"status":"accepted","command_id":wire["command_id"],"run_id":exact_usage.run_id})},
                                "receipt"|"resolve"=>match mode.receipt.clone(){Some(value)=>value,
                            None if op=="receipt" && mode.wait_for_submit=>json!({"status":"unknown","command_id":wire["command_id"]}),
                            None=>if let Some((budget,_))=budget(){if op=="resolve"{json!({"status":"not_admitted","command_id":wire["command_id"]})}else{json!({"status":"accepted","command_id":wire["command_id"],"execution_usage":usage(budget,true,true)})}}else{json!({"status":"unknown","command_id":wire["command_id"]})}},
                                _=>panic!("only bounded ordinary delegation wire: {wire}"),
                            };
                            let denied=mode.deny_receipt&&matches!(op,"receipt"|"resolve");let payload=if matches!(op,"capabilities"|"start_settings"){payload}else{json!({"session_id":wire["session_id"],"incarnation":Uuid::new_v4(),"result":payload})};
                            let body=json!({"protocol":1,"result":payload,"error":if denied{Some("history grant denied")}else{None},"outcome_unknown":false}).to_string();let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;let _=socket.shutdown().await;
                        });
                    }
                }
            }
            drop(listener);
            while let Some(result) = children.join_next().await {
                result.unwrap();
            }
        });
        Self {
            root,
            address,
            mode,
            calls,
            stop: Some(stop),
            job: Some(job),
        }
    }
    fn config(&self) -> crate::Config {
        let mut config = crate::Config::default();
        config.vessel.local_directory = Some(self.root.path().into());
        config
    }
    fn target(&self) -> Uuid {
        self.mode.lock().unwrap().destination
    }
    fn count(&self, op: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["op"] == op)
            .count()
    }
    async fn finish(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        tokio::time::timeout(WAIT, self.job.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(self.address).await.is_err());
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}
fn usage(budget: ExecutionBudget, complete: bool, cleanup: bool) -> ExecutionUsage {
    ExecutionUsage {
        session_id: budget.session_id,
        budget,
        run_id: Uuid::new_v4(),
        input_tokens: 7,
        output_tokens: 3,
        elapsed_ms: 25,
        complete,
        cleanup_observed: cleanup,
    }
}
async fn reconcile(owner: &ManagedSessionOwner, peer: &Peer, fence: bool) -> Value {
    tokio::time::timeout(
        WAIT,
        reconcile_goal_allocations(owner, &peer.config(), 0, 16, fence),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn fresh(
    create: bool,
    idle: Value,
    supports: bool,
    lost: bool,
) -> (
    Parent,
    Peer,
    Arc<crate::provider::goal_meter::GoalMeter>,
    Value,
) {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    {
        let mut m = peer.mode.lock().unwrap();
        m.idle = idle;
        m.supports = supports;
        m.wait_for_submit = true;
        m.lost_submit = lost;
    }
    let owner = parent.open().await;
    let meter = owner
        .begin_execution_meter(
            parent.command,
            parent.incarnation,
            Some(parent.budget.clone()),
        )
        .await
        .unwrap()
        .unwrap();
    let mut config = peer.config();
    config.goal_meter = Some(meter.clone());
    let sessions = peer.root.path().join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    std::fs::set_permissions(&sessions, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ownerpath = sessions.join(parent.session.to_string());
    std::fs::create_dir(&ownerpath).unwrap();
    std::fs::set_permissions(&ownerpath, std::fs::Permissions::from_mode(0o700)).unwrap();
    let tool = super::super::VesselTool::new(
        config.vessel.clone(),
        Some(super::super::VesselContext {
            session_id: parent.session,
            directory: peer.root.path().into(),
        }),
    )
    .with_launch(&config);
    let mut registry = ToolRegistry::default();
    registry.register(tool);
    let context = crate::tools::reliability_tests::context(peer.root.path());
    let command = Uuid::new_v4();
    let session = Uuid::new_v4();
    let args = if create {
        json!({"action":"create","command_id":command,"session_id":session,"workspace":peer.root.path(),"task":"Exact owned child 世界"})
    } else {
        json!({"action":"submit","command_id":command,"session_id":session,"expected_revision":3,"prompt":"Exact owned child 世界"})
    };
    let report = tokio::time::timeout(
        WAIT,
        registry.execute_report_with_workflow_secrets("vessel", args, &context, None),
    )
    .await
    .unwrap()
    .unwrap();
    let result = serde_json::from_str(&report.output.text_fallback()).unwrap();
    drop(owner);
    (parent, peer, meter, result)
}
#[tokio::test]
async fn registry_submit_uses_production_durable_allocation_and_exact_dispatch_before_one_effect() {
    let (parent, peer, meter, result) = fresh(
        false,
        json!({"revision":3,"run":null,"pending_cleanup_run":null}),
        true,
        false,
    )
    .await;
    assert!(result.to_string().contains("accepted"));
    tokio::time::timeout(
        WAIT,
        meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap();
    assert_eq!(peer.count("submit"), 1);
    let (child, dispatch) = parent.allocation().unwrap();
    assert!(dispatch.is_some());
    assert_eq!(child.tokens, 500);
    assert_eq!(child.parent_session_id, parent.session);
    assert_eq!(child.parent_run_id, parent.run);
    assert!(meter.measurement().complete);
    drop(meter);
    peer.finish().await;
}
#[tokio::test]
async fn registry_create_reserves_budget_before_creation_then_dispatches_same_child_once() {
    let (parent, peer, meter, result) = fresh(
        true,
        json!({"revision":3,"run":null,"pending_cleanup_run":null}),
        true,
        false,
    )
    .await;
    assert!(result.to_string().contains("accepted"));
    tokio::time::timeout(
        WAIT,
        meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap();
    assert_eq!(peer.count("start_settings"), 1);
    assert_eq!(peer.count("submit"), 1);
    let (child, dispatch) = parent.allocation().unwrap();
    assert_eq!(
        dispatch.unwrap()["command"]["budget"],
        serde_json::to_value(child).unwrap()
    );
    assert!(meter.measurement().complete);
    drop(meter);
    peer.finish().await;
}
#[tokio::test]
async fn legacy_budget_capability_refusal_never_allocates_or_creates_child() {
    let (parent, peer, meter, result) = fresh(
        false,
        json!({"revision":3,"run":null,"pending_cleanup_run":null}),
        false,
        false,
    )
    .await;
    assert_eq!(result["status"], "outcome_unknown");
    assert!(parent.allocation().is_none());
    assert_eq!(peer.count("submit"), 0);
    assert_eq!(peer.count("start_settings"), 0);
    assert_eq!(peer.count("snapshot"), 0);
    drop(meter);
    peer.finish().await;
}
#[tokio::test]
async fn active_child_and_pending_cleanup_refuse_before_budget_reservation_or_submission() {
    for idle in [
        json!({"revision":3,"run":{"state":"running"},"pending_cleanup_run":null}),
        json!({"revision":3,"run":null,"pending_cleanup_run":Uuid::new_v4()}),
        json!({"revision":3,"run":null}),
    ] {
        let (parent, peer, meter, result) = fresh(false, idle, true, false).await;
        assert_eq!(result["status"], "outcome_unknown");
        assert!(parent.allocation().is_none());
        assert_eq!(peer.count("submit"), 0);
        assert_eq!(peer.count("snapshot"), 1);
        drop(meter);
        peer.finish().await;
    }
}
#[tokio::test]
async fn readonly_reconciliation_unknown_receipt_preserves_entire_private_graph_and_parent_uncertainty()
 {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    let child = parent.retained(peer.target(), true);
    peer.mode.lock().unwrap().receipt =
        Some(json!({"status":"unknown","command_id":child.command_id}));
    let before = parent.graph();
    let owner = parent.open().await;
    let result = reconcile(&owner, &peer, false).await;
    assert_eq!(result["allocations"][0]["observed"], false);
    assert_eq!(result["effects_replayed"], false);
    assert_eq!(result["continuation_restored"], false);
    assert!(parent.graph() == before);
    assert_eq!(peer.count("submit"), 0);
    assert_eq!(peer.count("start_settings"), 0);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn exact_complete_usage_is_imported_once_after_reopen_without_replaying_dispatch() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    let child = parent.retained(peer.target(), true);
    let exact = usage(child.clone(), true, true);
    peer.mode.lock().unwrap().receipt = Some(json!({"execution_usage":exact}));
    let owner = parent.open().await;
    let before = owner
        .process_receipt(parent.command)
        .await
        .unwrap()
        .unwrap();
    let imported = reconcile(&owner, &peer, false).await;
    assert_eq!(imported["allocations"][0]["observed"], true);
    assert_eq!(imported["allocations"][0]["usage_updated"], true);
    let after = owner
        .process_receipt(parent.command)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after["execution_usage"], before["execution_usage"]);
    assert_eq!(after["execution_usage_observed"]["input_tokens"], 7);
    assert_eq!(after["execution_usage_observed"]["complete"], false);
    drop(owner);
    parent.lease_free();
    let owner = parent.open().await;
    let graph = parent.graph();
    let replay = reconcile(&owner, &peer, false).await;
    assert_eq!(replay["allocations"][0]["usage_updated"], false);
    assert!(parent.graph() == graph);
    assert_eq!(peer.count("submit"), 0);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn wrong_destination_never_queries_another_vessel_for_a_retained_child_receipt() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    parent.retained(Uuid::new_v4(), true);
    let owner = parent.open().await;
    let before = parent.graph();
    let result = reconcile(&owner, &peer, false).await;
    assert_eq!(result["allocations"][0]["observed"], false);
    assert_eq!(peer.count("receipt"), 0);
    assert!(parent.graph() == before);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn changed_budget_session_run_and_usage_identity_cannot_settle_reserved_uncertainty() {
    for field in 0..7 {
        let parent = Parent::new();
        let peer = Peer::new(&parent).await;
        let child = parent.retained(peer.target(), true);
        let mut wrong = usage(child, true, true);
        match field {
            0 => wrong.budget.command_id = Uuid::new_v4(),
            1 => wrong.budget.session_id = Uuid::new_v4(),
            2 => wrong.budget.tokens += 1,
            3 => wrong.budget.parent_run_id = Uuid::new_v4(),
            4 => wrong.session_id = Uuid::new_v4(),
            5 => wrong.run_id = Uuid::nil(),
            _ => wrong.budget.expires_at_ms += 1,
        };
        peer.mode.lock().unwrap().receipt = Some(json!({"execution_usage":wrong}));
        let owner = parent.open().await;
        let before = parent.graph();
        assert_eq!(
            reconcile(&owner, &peer, false).await["allocations"][0]["observed"],
            false
        );
        assert!(parent.graph() == before);
        drop(owner);
        parent.lease_free();
        peer.finish().await;
    }
}
#[tokio::test]
async fn malformed_supplemental_usage_cannot_override_exact_original_child_receipt() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    let child = parent.retained(peer.target(), true);
    peer.mode.lock().unwrap().receipt = Some(
        json!({"execution_usage":usage(child,true,true),"execution_usage_observed":{"malformed":"not usage"}}),
    );
    let owner = parent.open().await;
    let before = parent.graph();
    assert_eq!(
        reconcile(&owner, &peer, false).await["allocations"][0]["observed"],
        false
    );
    assert!(parent.graph() == before);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn refused_history_authority_remains_pending_without_cancelling_or_resubmitting_child() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    parent.retained(peer.target(), true);
    peer.mode.lock().unwrap().deny_receipt = true;
    let owner = parent.open().await;
    let before = parent.graph();
    assert_eq!(
        reconcile(&owner, &peer, false).await["allocations"][0]["observed"],
        false
    );
    assert!(parent.graph() == before);
    assert_eq!(peer.count("submit"), 0);
    assert_eq!(peer.count("resolve"), 0);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn exact_nonadmission_receipt_closes_allocation_without_fabricating_a_zero_usage_run() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    let child = parent.retained(peer.target(), true);
    peer.mode.lock().unwrap().receipt =
        Some(json!({"status":"not_admitted","command_id":child.command_id}));
    let owner = parent.open().await;
    let result = reconcile(&owner, &peer, false).await;
    assert_eq!(result["allocations"][0]["non_admission"], Value::Null);
    assert_eq!(result["allocations"][0]["observed"], true);
    let allocations = owner.goal_allocations(0, 16).await.unwrap();
    assert!(allocations[0].closed);
    let receipt: Option<String> = parent
        .db()
        .query_row(
            "SELECT receipt FROM process_goal_allocations WHERE request_id=?1",
            [child.command_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(receipt.is_none());
    assert_eq!(peer.count("submit"), 0);
    let second = reconcile(&owner, &peer, false).await;
    assert_eq!(second["allocations"][0]["non_admission"], true);
    assert_eq!(peer.count("receipt"), 1);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn wrong_negative_receipt_id_does_not_refund_unknown_work() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    parent.retained(peer.target(), true);
    peer.mode.lock().unwrap().receipt =
        Some(json!({"status":"not_admitted","command_id":Uuid::new_v4()}));
    let owner = parent.open().await;
    let before = parent.graph();
    assert_eq!(
        reconcile(&owner, &peer, false).await["allocations"][0]["observed"],
        false
    );
    assert!(parent.graph() == before);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn fenced_reconciliation_sends_original_resolve_not_a_second_submit() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    let child = parent.retained(peer.target(), true);
    let original = parent.allocation().unwrap().1.unwrap()["command"].clone();
    let owner = parent.open().await;
    let result = reconcile(&owner, &peer, true).await;
    assert_eq!(result["allocations"][0]["observed"], true);
    let calls = peer.calls.lock().unwrap().clone();
    let resolve = calls.iter().find(|v| v["op"] == "resolve").unwrap();
    assert_eq!(resolve["original"], original);
    assert_eq!(resolve["command_id"], child.command_id.to_string());
    assert_eq!(peer.count("submit"), 0);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn terminal_gated_undispatched_allocation_closes_without_any_child_receipt_rpc() {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    parent.retained(peer.target(), false);
    let owner = parent.open().await;
    let result = reconcile(&owner, &peer, false).await;
    assert_eq!(result["allocations"][0]["non_admission"], true);
    assert_eq!(peer.count("receipt"), 0);
    assert_eq!(peer.count("resolve"), 0);
    let count = peer.count("capabilities");
    assert_eq!(
        reconcile(&owner, &peer, true).await["allocations"][0]["non_admission"],
        true
    );
    assert_eq!(peer.count("capabilities"), count + 1);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn disabled_or_missing_routes_leave_retained_allocation_unknown_and_never_recreate_credential()
 {
    let parent = Parent::new();
    let peer = Peer::new(&parent).await;
    parent.retained(peer.target(), true);
    let owner = parent.open().await;
    let before = parent.graph();
    let mut config = peer.config();
    config.vessel.enabled = false;
    let result = reconcile_goal_allocations(&owner, &config, 0, 16, false)
        .await
        .unwrap();
    assert_eq!(result["allocations"][0]["observed"], false);
    assert_eq!(peer.count("capabilities"), 0);
    config.vessel.enabled = true;
    let missing = peer.root.path().join("absent-route");
    config.vessel.local_directory = Some(missing.clone());
    let result = reconcile_goal_allocations(&owner, &config, 0, 16, false)
        .await
        .unwrap();
    assert_eq!(result["allocations"][0]["observed"], false);
    assert!(!missing.exists());
    assert!(parent.graph() == before);
    drop(owner);
    parent.lease_free();
    peer.finish().await;
}
#[tokio::test]
async fn incomplete_child_usage_or_unobserved_cleanup_imports_only_observation_without_restoring_parent()
 {
    for cleanup in [false, true] {
        let parent = Parent::new();
        let peer = Peer::new(&parent).await;
        let child = parent.retained(peer.target(), true);
        peer.mode.lock().unwrap().receipt =
            Some(json!({"execution_usage":usage(child,false,cleanup)}));
        let owner = parent.open().await;
        let before = owner
            .process_receipt(parent.command)
            .await
            .unwrap()
            .unwrap();
        let result = reconcile(&owner, &peer, false).await;
        assert_eq!(result["allocations"][0]["observed"], true);
        let after = owner
            .process_receipt(parent.command)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after["execution_usage"], before["execution_usage"]);
        assert_eq!(after["execution_usage_observed"]["complete"], false);
        assert!(owner.goal_continuation().await.unwrap().is_none());
        assert_eq!(peer.count("submit"), 0);
        drop(owner);
        parent.lease_free();
        peer.finish().await;
    }
}

#[tokio::test]
async fn lost_submit_reply_remains_unknown_even_when_exact_usage_is_later_accounted_without_replay()
{
    let (parent, peer, meter, result) = fresh(
        false,
        json!({"revision":3,"run":null,"pending_cleanup_run":null}),
        true,
        true,
    )
    .await;
    assert_eq!(result["status"], "outcome_unknown");
    let (child, dispatch) = parent.allocation().unwrap();
    assert!(dispatch.is_some());
    assert_eq!(peer.count("submit"), 1);
    tokio::time::timeout(
        WAIT,
        meter.wait_allocations(tokio_util::sync::CancellationToken::new()),
    )
    .await
    .unwrap();
    let measurement = meter.measurement();
    assert_eq!(
        (measurement.input_tokens, measurement.output_tokens),
        (7, 3)
    );
    assert!(measurement.complete);
    assert_eq!(peer.count("submit"), 1);
    let journal = peer
        .root
        .path()
        .join("sessions")
        .join(parent.session.to_string())
        .join("resources/vessel-coordination");
    let retained: Value = serde_json::from_slice(
        &std::fs::read(journal.join(format!("{}.result.json", child.command_id))).unwrap(),
    )
    .unwrap();
    assert_eq!(retained["status"], "outcome_unknown");
    assert_eq!(retained["replayed"], false);
    // Usage settlement is not a confirmed tool reply or background-task join.
    drop(meter);
    peer.finish().await;
}
