//! Real parent obligations and scoped loopback transport. No remote service,
//! paid provider, native root or unowned worker is contacted.
use super::*;
use crate::attachment::{
    journal::{Journal, TurnAdmission},
    runtime::{Admission, RunOwner},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Clone, Copy)]
enum Mode {
    Complete,
    CompleteNested,
    Pending,
    Refused,
    Unknown,
    Malformed,
    Misattributed,
}
struct Peer {
    url: String,
    vessel: Uuid,
    budget_feature: Arc<AtomicBool>,
    mode: Arc<Mutex<Mode>>,
    assigned: Arc<Mutex<BTreeMap<Uuid, AssignmentRequest>>>,
    operations: Arc<Mutex<Vec<String>>>,
    task: Option<tokio::task::JoinHandle<()>>,
    stop: tokio_util::sync::CancellationToken,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Peer {
    async fn new(mode: Mode, owner: ManagedSessionOwner, run_id: Uuid) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let vessel = Uuid::new_v4();
        let mode = Arc::new(Mutex::new(mode));
        let assigned = Arc::new(Mutex::new(BTreeMap::<Uuid, AssignmentRequest>::new()));
        let operations = Arc::new(Mutex::new(Vec::new()));
        let budget_feature = Arc::new(AtomicBool::new(true));
        let advertised = budget_feature.clone();
        let behavior = mode.clone();
        let records = assigned.clone();
        let calls = operations.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let shutdown = stop.clone();
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=shutdown.cancelled()=>break,
                    Some(done)=clients.join_next(),if !clients.is_empty()=>{done.unwrap();},
                    socket=listener.accept()=>{let(mut socket,_)=socket.unwrap();let behavior=behavior.clone();let records=records.clone();let calls=calls.clone();let owner=owner.clone();let advertised=advertised.clone();
                        clients.spawn(async move {
                            let mut bytes=Vec::new();let end=loop {let mut block=[0;4096];let n=socket.read(&mut block).await.unwrap();if n==0{return;}bytes.extend_from_slice(&block[..n]);assert!(bytes.len()<512*1024);if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){break end+4;}};
                            let headers=String::from_utf8_lossy(&bytes[..end]);assert!(headers.starts_with("POST /v1/vessel/command "));let length=headers.lines().find_map(|line|{let(name,value)=line.split_once(':')?;name.eq_ignore_ascii_case("content-length").then(||value.trim().parse::<usize>().unwrap())}).unwrap();
                            while bytes.len()<end+length {let mut block=[0;4096];let n=socket.read(&mut block).await.unwrap();if n==0{return;}bytes.extend_from_slice(&block[..n]);}
                            let request:VesselRequest=serde_json::from_slice(&bytes[end..end+length]).unwrap();assert_eq!(request.protocol,1);
                            let mut error=None;let mut unknown=false;
                            let op=match &request.command {VesselCommand::Capabilities=>"capabilities",VesselCommand::Assign {..}=>"assign",VesselCommand::ObserveAssignment {..}=>"observe",VesselCommand::FenceAssignment {..}=>"fence",_=>panic!("unexpected participant request")};calls.lock().await.push(op.into());
                            let mode=*behavior.lock().await;
                            let result=match request.command {
                                VesselCommand::Capabilities=>json!({"vessel_id":vessel,"features":if advertised.load(Ordering::SeqCst) {vec!["execution_budget"]} else {vec![]}}),
                                VesselCommand::Assign {request}=>{let canonical=owner.assignment_request(run_id,request.assignment_id).await.unwrap().unwrap();assert_eq!(serde_json::to_value(&canonical).unwrap(),serde_json::to_value(&request).unwrap(),"parent obligation must precede transport effect");records.lock().await.insert(request.assignment_id,request.clone());match mode {Mode::Refused=>{error=Some("owned receiver refusal".to_owned());Value::Null},Mode::Unknown=>{error=Some("owned unknown delivery".to_owned());unknown=true;Value::Null},Mode::Malformed=>json!({"invalid":"observation"}),_=>observation(vessel,&request,mode,false)}},
                                VesselCommand::ObserveAssignment {assignment_id}=>{let request=records.lock().await.get(&assignment_id).cloned();if let Some(request)=request {match mode {Mode::Unknown=>{error=Some("owned unknown delivery".into());unknown=true;Value::Null},Mode::Malformed=>json!({"invalid":"observation"}),_=>observation(vessel,&request,mode,false)}} else {error=Some("owned assignment not yet observed".into());unknown=true;Value::Null}},
                                VesselCommand::FenceAssignment {request}=>{records.lock().await.insert(request.assignment_id,request.clone());cancelled_observation(vessel,&request,mode)},
                                _=>unreachable!(),
                            };
                            let body=json!({"protocol":1,"result":result,"error":error,"outcome_unknown":unknown}).to_string();let response=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=socket.write_all(response.as_bytes()).await;let _=socket.shutdown().await;
                        });
                    }
                }
            }
            clients.abort_all();
            while let Some(done) = clients.join_next().await {
                assert!(done.is_ok() || done.unwrap_err().is_cancelled());
            }
        });
        Self {
            url,
            vessel,
            budget_feature,
            mode,
            assigned,
            operations,
            task: Some(task),
            stop,
        }
    }
    async fn shutdown(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap()
                .unwrap();
        }
    }
    async fn count(&self, op: &str) -> usize {
        self.operations
            .lock()
            .await
            .iter()
            .filter(|value| value.as_str() == op)
            .count()
    }
}
fn observation(vessel: Uuid, request: &AssignmentRequest, mode: Mode, closed: bool) -> Value {
    if matches!(mode, Mode::Refused) {
        return json!({"assignment_id":request.assignment_id,"participant_vessel_id":vessel,"parent_session_id":request.parent_session_id,"parent_run_id":request.parent_run_id,"child_session_id":request.assignment_id,"child_incarnation":null,"run_id":null,"state":"rejected","cleanup_observed":true,"result":{"reason":"owned receiver refusal"}});
    }
    let terminal = matches!(mode, Mode::Complete | Mode::CompleteNested) || closed;
    let child = if matches!(mode, Mode::Misattributed) {
        Uuid::new_v4()
    } else {
        request.assignment_id
    };
    let mut value = json!({"admission_closed":closed,"execution_usage":null,"execution_usage_observed":null,"assignment_id":request.assignment_id,"participant_vessel_id":vessel,"parent_session_id":request.parent_session_id,"parent_run_id":request.parent_run_id,"child_session_id":child,"child_incarnation":if closed {None}else{Some(request.assignment_id)},"run_id":if closed {None}else{Some(request.assignment_id)},"state":if closed {"cancelled"}else if terminal {"completed"}else{"running"},"cleanup_observed":terminal,"result":if closed {Value::Null}else if terminal {json!({"text":"Owned participant result"})}else{Value::Null}});
    if terminal
        && !closed
        && let Some(budget) = &request.budget
    {
        let usage = json!({"budget":budget,"session_id":request.assignment_id,"run_id":request.assignment_id,"input_tokens":3,"output_tokens":2,"elapsed_ms":10,"complete":true,"cleanup_observed":true});
        if matches!(mode, Mode::CompleteNested) {
            value["result"]["execution_usage"] = usage.clone();
            value["result"]["execution_usage_observed"] = usage;
        } else {
            value["execution_usage"] = usage.clone();
            value["execution_usage_observed"] = usage;
        }
    }
    value
}
fn cancelled_observation(vessel: Uuid, request: &AssignmentRequest, mode: Mode) -> Value {
    // A known admitted child retains its run and incarnation. Only the fixture's
    // deliberately unadmitted unknown/malformed cases produce a permanent fence.
    let admitted = matches!(mode, Mode::Pending | Mode::Complete | Mode::CompleteNested);
    let mut value = observation(vessel, request, Mode::Pending, !admitted);
    value["state"] = json!("cancelled");
    value["cleanup_observed"] = json!(true);
    value["result"] = Value::Null;
    value
}
struct Fixture {
    root: tempfile::TempDir,
    tool: ParticipantTool,
    run: RunOwner,
    run_id: Uuid,
    context: ToolContext,
    peer: Peer,
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().unwrap();
        let session = crate::session::Session::new(root.path().into(), "fixture".into());
        let directory = root.path().join("journal");
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        drop(journal);
        let owner = ManagedSessionOwner::open(directory, session.id)
            .await
            .unwrap();
        owner.initialize_process_commands().await.unwrap();
        owner.initialize_session_resources().await.unwrap();
        let principal = Uuid::new_v4();
        owner.initialize_command_bindings(principal).await.unwrap();
        let admission = TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: principal,
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 120_000,
            prompt: "Owned parent participant fixture".into(),
            parts: vec![],
        };
        let Admission::New(mut run) = owner.admit(admission.clone()).await.unwrap() else {
            panic!("fresh parent")
        };
        // Retain the exact admitted canonical record rather than accessing the
        // execution owner's private fields or guessing a run ID from a command.
        let admitted = owner.lookup_turn(admission).await.unwrap().unwrap();
        assert_eq!(admitted.session_id, session.id);
        let run_id = admitted.id;
        run.register_local_cleanup().await.unwrap();
        run.start_operator().await.unwrap();
        let peer = Peer::new(mode, owner.clone(), run_id).await;
        let credential = root.path().join("participant.json");
        std::fs::write(
            &credential,
            serde_json::to_vec(&AccessCredential {
                endpoint: peer.url.clone(),
                token: "a".repeat(64),
                grant_id: Uuid::new_v4(),
                session_id: session.id,
            })
            .unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let endpoint = ParticipantEndpoint {
            name: "owned".into(),
            credential_file: credential,
            participant_vessel_id: peer.vessel,
            binding_id: Uuid::new_v4(),
            binding_revision: 1,
        };
        let tool = ParticipantTool {
            parent: Arc::new(Parent {
                meter: None,
                owner,
                run_id,
                principal_id: principal,
                vessel_id: Uuid::new_v4(),
                endpoints: vec![endpoint],
            }),
        };
        Self {
            context: crate::tools::reliability_tests::context(root.path()),
            root,
            tool,
            run,
            run_id,
            peer,
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        serde_json::from_str(&self.tool.execute(args, &self.context).await?)
            .map_err(|error| ToolError::Failed(error.to_string()))
    }
    fn submit(&self, id: Uuid) -> Value {
        json!({"action":"submit","participant":"owned","task":"Explicit owned task","assignment_id":id,"context":[{"role":"user","content":"Only explicitly selected public text"}]})
    }
    async fn finish(&mut self) {
        self.run
            .finish_operator(Ok("owned parent terminal".into()), false)
            .await
            .unwrap();
        self.run.confirm_local_cleanup_observed().await.unwrap();
        self.peer.shutdown().await;
    }
}

#[tokio::test]
async fn real_participant_submission_records_before_delivery_and_exact_terminal_retry_does_not_assign_again()
 {
    let mut f = Fixture::new(Mode::Complete).await;
    let id = Uuid::new_v4();
    let first = f.call(f.submit(id)).await.unwrap();
    assert_eq!(first["state"], "completed");
    assert_eq!(first["cleanup_observed"], true);
    assert_eq!(f.peer.count("assign").await, 1);
    let recorded = f
        .tool
        .parent
        .owner
        .assignment_request(f.run_id, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recorded.context[0].content,
        "Only explicitly selected public text"
    );
    assert_eq!(recorded.policy.access, "unrestricted");
    let retry = f.call(f.submit(id)).await.unwrap();
    assert_eq!(retry, first);
    assert_eq!(f.peer.count("assign").await, 1);
    assert_eq!(
        f.call(json!({"action":"list"}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.finish().await;
    assert!(f.tool.parent.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn unknown_delivery_keeps_original_obligation_and_rejects_a_replacement_assignment() {
    let mut f = Fixture::new(Mode::Unknown).await;
    let id = Uuid::new_v4();
    let result = f.call(f.submit(id)).await.unwrap();
    assert_eq!(result["state"], "acceptance_unknown");
    assert_eq!(result["cleanup_observed"], false);
    assert!(f.call(f.submit(Uuid::new_v4())).await.is_err());
    assert_eq!(f.peer.count("assign").await, 1);
    let closed = f
        .call(json!({"action":"cancel","participant":"owned","assignment_id":id}))
        .await
        .unwrap();
    assert_eq!(closed["admission_closed"], true);
    assert_eq!(closed["cleanup_observed"], true);
    assert_eq!(closed["state"], "cancelled");
    f.finish().await;
    assert!(f.tool.parent.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn definite_first_refusal_is_retained_as_rejected_without_fabricating_a_run() {
    let mut f = Fixture::new(Mode::Refused).await;
    let id = Uuid::new_v4();
    let result = f.call(f.submit(id)).await.unwrap();
    assert_eq!(result["state"], "rejected");
    assert!(result["run_id"].is_null());
    assert_eq!(result["cleanup_observed"], true);
    f.finish().await;
}

#[tokio::test]
async fn malformed_or_forged_acceptance_never_marks_the_parent_obligation_clean() {
    for mode in [Mode::Malformed, Mode::Misattributed] {
        let mut f = Fixture::new(mode).await;
        let id = Uuid::new_v4();
        assert!(f.call(f.submit(id)).await.is_err());
        let list = f.call(json!({"action":"list"})).await.unwrap();
        assert_eq!(list[0]["cleanup_observed"], false);
        assert_ne!(list[0]["state"], "completed");
        f.call(json!({"action":"cancel","participant":"owned","assignment_id":id}))
            .await
            .unwrap();
        f.finish().await;
    }
}

#[tokio::test]
async fn cancellation_monitor_survives_tool_return_and_requires_observed_fence_receipt() {
    let mut f = Fixture::new(Mode::Pending).await;
    let id = Uuid::new_v4();
    let result = f.call(f.submit(id)).await.unwrap();
    assert_eq!(result["cleanup_observed"], false);
    f.context.cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let prior = f
                .tool
                .parent
                .owner
                .assignment_result(f.run_id, id)
                .await
                .unwrap();
            if prior["cleanup_observed"] == true {
                // False is deliberately omitted on the wire. Decode the strict
                // observation rather than interpreting a missing JSON key as a
                // different admission proof.
                let observed: AssignmentObservation = serde_json::from_value(prior).unwrap();
                assert!(!observed.admission_closed);
                assert_eq!(observed.run_id, Some(id));
                assert_eq!(observed.child_incarnation, Some(id));
                assert_eq!(observed.state, "cancelled");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(f.peer.count("fence").await >= 1);
    f.finish().await;
    assert!(f.tool.parent.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn observation_and_retained_reconciliation_are_exact_binding_and_child_scoped() {
    let mut f = Fixture::new(Mode::Pending).await;
    let id = Uuid::new_v4();
    f.call(f.submit(id)).await.unwrap();
    assert!(
        f.call(json!({"action":"observe","participant":"wrong","assignment_id":id}))
            .await
            .is_err()
    );
    let mut config = crate::Config {
        participants: f.tool.parent.endpoints.clone(),
        ..Default::default()
    };
    config.participants[0].binding_id = Uuid::new_v4();
    assert!(
        crate::participant::reconcile(
            f.tool.parent.owner.clone(),
            f.run_id,
            id,
            "owned",
            false,
            &config
        )
        .await
        .is_err()
    );
    config.participants = f.tool.parent.endpoints.clone();
    *f.peer.mode.lock().await = Mode::Complete;
    let observed = crate::participant::reconcile(
        f.tool.parent.owner.clone(),
        f.run_id,
        id,
        "owned",
        false,
        &config,
    )
    .await
    .unwrap();
    assert_eq!(observed["cleanup_observed"], true);
    f.peer.shutdown().await;
    let calls = f.peer.count("observe").await;
    assert_eq!(
        crate::participant::reconcile(
            f.tool.parent.owner.clone(),
            f.run_id,
            id,
            "owned",
            false,
            &config
        )
        .await
        .unwrap(),
        observed
    );
    assert_eq!(f.peer.count("observe").await, calls);
    f.finish().await;
}

#[tokio::test]
async fn payload_collision_and_lost_credential_scope_refuse_before_new_assignment_effect() {
    let mut f = Fixture::new(Mode::Complete).await;
    let id = Uuid::new_v4();
    f.call(f.submit(id)).await.unwrap();
    let mut changed = f.submit(id);
    changed["task"] = json!("different task");
    assert!(f.call(changed).await.is_err());
    assert_eq!(f.peer.count("assign").await, 1);
    let path = &f.tool.parent.endpoints[0].credential_file;
    let mut credential: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    credential["session_id"] = json!(Uuid::new_v4());
    std::fs::write(path, serde_json::to_vec(&credential).unwrap()).unwrap();
    assert!(f.call(f.submit(Uuid::new_v4())).await.is_err());
    assert_eq!(f.peer.count("assign").await, 1);
    f.finish().await;
}

#[tokio::test]
async fn tool_schema_does_not_disclose_runtime_instructions_or_invent_context_and_cancellation_prevents_delivery()
 {
    let mut f = Fixture::new(Mode::Complete).await;
    let definition = f.tool.definition();
    assert_eq!(definition.name, "participant");
    assert!(definition.description.contains("unknown admission"));
    let schema = crate::tools::schema::CompiledSchema::compile(&definition.input_schema).unwrap();
    schema.validate(&f.submit(Uuid::new_v4())).unwrap();
    assert!(schema.validate(&json!({"action":"submit","context":[{"role":"system","content":"injected runtime instructions"}]})).is_err());
    let id = Uuid::new_v4();
    f.context.cancellation.cancel();
    assert!(f.call(f.submit(id)).await.is_err());
    assert_eq!(f.peer.count("assign").await, 0);
    assert!(
        f.tool
            .parent
            .owner
            .assignment_request(f.run_id, id)
            .await
            .unwrap()
            .is_none()
    );
    f.finish().await;
}

#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.0.load(Ordering::SeqCst),
            "owned participant authority revoked"
        );
        Ok(())
    }
}
struct RevokeAfterApproval(Arc<Authority>);
#[async_trait]
impl crate::tools::Approver for RevokeAfterApproval {
    async fn approve(
        &self,
        request: &crate::tools::ApprovalRequest,
    ) -> crate::tools::ApprovalOutcome {
        assert_eq!(request.action, "participant_assignment");
        assert_eq!(request.target, "owned");
        assert!(request.reason.contains("1 selected context messages"));
        self.0.0.store(false, Ordering::SeqCst);
        crate::tools::ApprovalOutcome::Approved
    }
}

#[tokio::test]
async fn read_only_and_unattended_approval_refuse_without_contacting_or_recording_a_participant() {
    for mode in [AccessMode::ReadOnly, AccessMode::Approval] {
        let mut f = Fixture::new(Mode::Complete).await;
        f.context.policy = Arc::new(
            crate::policy::Policy::new(
                &crate::Config {
                    access: Some(mode),
                    ..Default::default()
                },
                f.root.path().into(),
            )
            .unwrap(),
        );
        let id = Uuid::new_v4();
        assert!(f.call(f.submit(id)).await.is_err());
        assert_eq!(f.peer.count("capabilities").await, 0);
        assert_eq!(f.peer.count("assign").await, 0);
        assert!(
            f.tool
                .parent
                .owner
                .assignment_request(f.run_id, id)
                .await
                .unwrap()
                .is_none()
        );
        f.finish().await;
    }
}

#[tokio::test]
async fn foreground_authority_is_rechecked_after_approval_before_any_remote_dispatch() {
    let mut f = Fixture::new(Mode::Complete).await;
    let authority = Arc::new(Authority(AtomicBool::new(true)));
    f.context.policy = Arc::new(
        crate::policy::Policy::new(
            &crate::Config {
                access: Some(AccessMode::Approval),
                ..Default::default()
            },
            f.root.path().into(),
        )
        .unwrap()
        .with_execution_authority(authority.clone()),
    );
    f.context.approver = Arc::new(RevokeAfterApproval(authority));
    let id = Uuid::new_v4();
    assert!(f.call(f.submit(id)).await.is_err());
    assert_eq!(f.peer.count("capabilities").await, 0);
    assert!(
        f.tool
            .parent
            .owner
            .assignment_request(f.run_id, id)
            .await
            .unwrap()
            .is_none()
    );
    f.finish().await;
}

#[tokio::test]
async fn receiver_identity_and_budget_capability_are_required_before_parent_allocation() {
    let mut f = Fixture::new(Mode::Complete).await;
    Arc::get_mut(&mut f.tool.parent).unwrap().endpoints[0].participant_vessel_id = Uuid::new_v4();
    let id = Uuid::new_v4();
    assert!(f.call(f.submit(id)).await.is_err());
    assert_eq!(f.peer.count("assign").await, 0);
    Arc::get_mut(&mut f.tool.parent).unwrap().endpoints[0].participant_vessel_id = f.peer.vessel;
    Arc::get_mut(&mut f.tool.parent).unwrap().meter = Some(
        crate::provider::goal_meter::GoalMeter::new(100, Duration::from_secs(10)),
    );
    f.peer.budget_feature.store(false, Ordering::SeqCst);
    assert!(f.call(f.submit(id)).await.is_err());
    assert!(
        f.tool
            .parent
            .owner
            .assignment_request(f.run_id, id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.peer.count("assign").await, 0);
    f.finish().await;
}

#[tokio::test]
async fn configured_participant_limits_and_invalid_bindings_fail_before_global_runtime_identity_lookup()
 {
    let mut f = Fixture::new(Mode::Complete).await;
    let parent = &f.tool.parent;
    assert!(
        ParticipantTool::configured(
            parent.owner.clone(),
            parent.run_id,
            parent.principal_id,
            &crate::Config::default()
        )
        .await
        .unwrap()
        .is_none()
    );
    let endpoint = parent.endpoints[0].clone();
    let mut bad = Vec::new();
    let mut changed = endpoint.clone();
    changed.name.clear();
    bad.push(vec![changed]);
    let mut changed = endpoint.clone();
    changed.name = "x".repeat(65);
    bad.push(vec![changed]);
    let mut changed = endpoint.clone();
    changed.credential_file = "relative.json".into();
    bad.push(vec![changed]);
    let mut changed = endpoint.clone();
    changed.participant_vessel_id = Uuid::nil();
    bad.push(vec![changed]);
    let mut changed = endpoint.clone();
    changed.binding_id = Uuid::nil();
    bad.push(vec![changed]);
    let mut changed = endpoint.clone();
    changed.binding_revision = 0;
    bad.push(vec![changed]);
    bad.push(vec![endpoint.clone(), endpoint.clone()]);
    bad.push(vec![endpoint; 17]);
    for endpoints in bad {
        let error = ParticipantTool::configured(
            parent.owner.clone(),
            parent.run_id,
            parent.principal_id,
            &crate::Config {
                participants: endpoints,
                ..Default::default()
            },
        )
        .await
        .err()
        .unwrap();
        assert!(
            error.to_string().contains("configuration")
                || error.to_string().contains("endpoint limit")
        );
    }
    assert_eq!(f.peer.count("capabilities").await, 0);
    f.finish().await;
}

#[tokio::test]
async fn configured_supervised_identity_is_isolated_to_an_owned_child_process() {
    const CHILD: &str = "VOYAGE_PARTICIPANT_CONFIGURED_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let mut f = Fixture::new(Mode::Complete).await;
        let root = f.root.path().join("supervised");
        std::fs::create_dir_all(root.join("identity")).unwrap();
        let vessel = Uuid::new_v4();
        std::fs::write(
            root.join("identity/public.json"),
            serde_json::to_vec(&VesselIdentity {
                vessel_id: vessel,
                public_key: "owned synthetic public identity".into(),
            })
            .unwrap(),
        )
        .unwrap();
        crate::build::set_resource_root(root.join("runtime/resources/session")).unwrap();
        let parent = &f.tool.parent;
        let configured = ParticipantTool::configured(
            parent.owner.clone(),
            parent.run_id,
            parent.principal_id,
            &crate::Config {
                participants: parent.endpoints.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
        let id = Uuid::new_v4();
        let output = configured.execute(f.submit(id), &f.context).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&output).unwrap()["cleanup_observed"],
            true
        );
        assert_eq!(f.peer.assigned.lock().await[&id].parent_vessel_id, vessel);
        f.finish().await;
        return;
    }
    // OnceLock resource roots belong to one runtime. A separate libtest child
    // exercises configured() without changing the concurrent parent workspace.
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","participant::tool::family_tests::configured_supervised_identity_is_isolated_to_an_owned_child_process","--nocapture"]).env(CHILD,"1").kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "configured participant child failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}

#[tokio::test]
async fn malformed_usage_and_non_admission_proof_do_not_change_the_last_exact_observation() {
    let mut f = Fixture::new(Mode::Complete).await;
    let id = Uuid::new_v4();
    let prior = f.call(f.submit(id)).await.unwrap();
    let parsed: AssignmentObservation = serde_json::from_value(prior.clone()).unwrap();
    for invalid in [
        json!({"admission_closed":true}),
        json!({"result":{"execution_usage":{"invalid":"usage"}}}),
        json!({"execution_usage":{"budget":{"command_id":Uuid::new_v4(),"session_id":id,"parent_session_id":f.tool.parent.owner.session_id(),"parent_run_id":f.run_id,"tokens":10,"elapsed_ms":100,"expires_at_ms":u64::MAX},"session_id":id,"run_id":id,"input_tokens":1,"output_tokens":1,"elapsed_ms":1,"complete":true,"cleanup_observed":true}}),
    ] {
        let mut value = serde_json::to_value(&parsed).unwrap();
        for (key, replacement) in invalid.as_object().unwrap() {
            value[key] = replacement.clone();
        }
        let candidate: AssignmentObservation = serde_json::from_value(value).unwrap();
        assert!(f.tool.parent.record_observation(candidate).await.is_err());
        assert_eq!(
            f.tool
                .parent
                .owner
                .assignment_result(f.run_id, id)
                .await
                .unwrap(),
            prior
        );
    }
    f.finish().await;
}

#[tokio::test]
async fn retained_completed_reconciliation_is_offline_and_unknown_obligations_block_local_cleanup()
{
    let mut f = Fixture::new(Mode::Unknown).await;
    let id = Uuid::new_v4();
    f.call(f.submit(id)).await.unwrap();
    f.run
        .finish_operator(Ok("parent finished while delivery unknown".into()), false)
        .await
        .unwrap();
    // This error retains the local observation without certifying the unknown
    // remote obligation as clean. Reconciliation below must close that exact ID.
    assert!(f.run.confirm_local_cleanup_observed().await.is_err());
    assert_eq!(
        f.tool.parent.owner.process_snapshot().await.unwrap()["pending_cleanup_run"],
        json!(f.run_id)
    );
    let config = crate::Config {
        participants: f.tool.parent.endpoints.clone(),
        ..Default::default()
    };
    assert!(
        crate::participant::reconcile(
            f.tool.parent.owner.clone(),
            f.run_id,
            Uuid::new_v4(),
            "owned",
            false,
            &config
        )
        .await
        .is_err()
    );
    let closed = crate::participant::reconcile(
        f.tool.parent.owner.clone(),
        f.run_id,
        id,
        "owned",
        true,
        &config,
    )
    .await
    .unwrap();
    assert_eq!(closed["admission_closed"], true);
    assert!(f.tool.parent.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
    f.peer.shutdown().await;
    assert_eq!(
        crate::participant::reconcile(
            f.tool.parent.owner.clone(),
            f.run_id,
            id,
            "owned",
            false,
            &config
        )
        .await
        .unwrap(),
        closed
    );
}

// A private accounting observer for transport handoff contracts. Actual journal
// Goal reserve/settlement durability is covered by the attachment Goal cohort.
#[derive(Debug, Default)]
struct Accounting {
    parent_session: Uuid,
    parent_run: Uuid,
    allocations: std::sync::Mutex<Vec<crate::provider::goal_meter::AllocationRequest>>,
    dispatches: std::sync::Mutex<Vec<AssignmentRequest>>,
    receipts: std::sync::Mutex<Vec<voyage_protocol::execution_budget::ExecutionUsage>>,
    closures: std::sync::Mutex<Vec<(Uuid, Uuid, Value)>>,
}
#[async_trait]
impl crate::provider::goal_meter::Observer for Accounting {
    async fn record(
        &self,
        _: crate::provider::goal_meter::RequestObservation,
    ) -> anyhow::Result<()> {
        anyhow::bail!("fixture has no provider requests")
    }
    async fn allocate(
        &self,
        request: crate::provider::goal_meter::AllocationRequest,
    ) -> anyhow::Result<voyage_protocol::execution_budget::ExecutionBudget> {
        let budget = voyage_protocol::execution_budget::ExecutionBudget {
            command_id: request.command_id,
            session_id: request.session_id,
            parent_session_id: self.parent_session,
            parent_run_id: self.parent_run,
            tokens: request.tokens,
            elapsed_ms: request.elapsed_ms,
            expires_at_ms: request.expires_at_ms,
        };
        self.allocations.lock().unwrap().push(request);
        Ok(budget)
    }
    async fn dispatch(
        &self,
        dispatch: crate::provider::goal_meter::AllocationDispatch,
    ) -> anyhow::Result<()> {
        let crate::provider::goal_meter::AllocationDispatch::Participant { request } = dispatch
        else {
            anyhow::bail!("participant fixture cannot dispatch a Voyage command")
        };
        assert!(
            request
                .budget
                .as_ref()
                .unwrap()
                .valid_for(request.assignment_id)
        );
        self.dispatches.lock().unwrap().push(*request);
        Ok(())
    }
    async fn settle_allocation(
        &self,
        destination: Uuid,
        usage: voyage_protocol::execution_budget::ExecutionUsage,
    ) -> anyhow::Result<()> {
        assert_eq!(
            self.allocations
                .lock()
                .unwrap()
                .iter()
                .find(|item| item.command_id == usage.budget.command_id)
                .unwrap()
                .destination,
            destination
        );
        assert!(
            self.dispatches
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.assignment_id == usage.budget.command_id)
        );
        self.receipts.lock().unwrap().push(usage);
        Ok(())
    }
    async fn close_allocation(
        &self,
        destination: Uuid,
        id: Uuid,
        proof: Value,
    ) -> anyhow::Result<()> {
        self.closures.lock().unwrap().push((destination, id, proof));
        Ok(())
    }
}
fn accounting(f: &mut Fixture) -> Arc<Accounting> {
    let observer = Arc::new(Accounting {
        parent_session: f.tool.parent.owner.session_id(),
        parent_run: f.run_id,
        ..Default::default()
    });
    Arc::get_mut(&mut f.tool.parent).unwrap().meter =
        Some(crate::provider::goal_meter::GoalMeter::with_observer(
            100,
            Duration::from_secs(30),
            Some(observer.clone()),
        ));
    observer
}
#[tokio::test]
async fn bounded_participant_receipts_settle_once_for_top_level_and_nested_usage_without_redelegation()
 {
    for mode in [Mode::Complete, Mode::CompleteNested] {
        let mut f = Fixture::new(mode).await;
        let observer = accounting(&mut f);
        let id = Uuid::new_v4();
        let result = f.call(f.submit(id)).await.unwrap();
        assert_eq!(result["cleanup_observed"], true);
        let request = f.peer.assigned.lock().await[&id].clone();
        let budget = request.budget.clone().unwrap();
        assert_eq!(budget.command_id, id);
        assert_eq!(budget.session_id, id);
        assert_eq!(budget.parent_run_id, f.run_id);
        assert_eq!(budget.parent_session_id, f.tool.parent.owner.session_id());
        assert_eq!(budget.tokens, 50);
        assert!(budget.elapsed_ms <= 30_000);
        assert_eq!(observer.allocations.lock().unwrap().len(), 1);
        assert_eq!(observer.dispatches.lock().unwrap().len(), 1);
        assert_eq!(observer.receipts.lock().unwrap().len(), 1);
        assert!(observer.closures.lock().unwrap().is_empty());
        let meter = f.tool.parent.meter.as_ref().unwrap();
        let measurement = meter.measurement();
        assert_eq!(
            (
                measurement.input_tokens,
                measurement.output_tokens,
                measurement.complete
            ),
            (3, 2, true)
        );
        assert_eq!(f.call(f.submit(id)).await.unwrap(), result);
        assert_eq!(f.peer.count("assign").await, 1);
        assert_eq!(observer.receipts.lock().unwrap().len(), 1);
        f.finish().await;
    }
}
#[tokio::test]
async fn uncertain_bounded_delivery_keeps_reservation_until_exact_permanent_non_admission_proof() {
    let mut f = Fixture::new(Mode::Unknown).await;
    let observer = accounting(&mut f);
    let id = Uuid::new_v4();
    assert_eq!(
        f.call(f.submit(id)).await.unwrap()["state"],
        "acceptance_unknown"
    );
    let meter = f.tool.parent.meter.as_ref().unwrap();
    assert!(!meter.measurement().complete);
    assert!(observer.receipts.lock().unwrap().is_empty());
    assert!(observer.closures.lock().unwrap().is_empty());
    let closed = f
        .call(json!({"action":"cancel","participant":"owned","assignment_id":id}))
        .await
        .unwrap();
    assert_eq!(closed["admission_closed"], true);
    assert!(closed["run_id"].is_null());
    assert_eq!(
        *observer.closures.lock().unwrap(),
        vec![(
            f.peer.vessel,
            id,
            json!({"status":"not_admitted","command_id":id})
        )]
    );
    assert!(meter.measurement().complete);
    assert_eq!(meter.measurement().input_tokens, 0);
    // A duplicate terminal read cannot spend, submit again or close a new ID.
    assert_eq!(f.call(f.submit(id)).await.unwrap(), closed);
    assert_eq!(f.peer.count("assign").await, 1);
    assert_eq!(observer.closures.lock().unwrap().len(), 1);
    f.finish().await;
}
