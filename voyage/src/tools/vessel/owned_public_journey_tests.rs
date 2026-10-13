//! Whole live tool registry through owned public HTTP and private durable journals.
use super::*;
use crate::tools::{ToolRegistry, ToolReport};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
};
const WAIT: Duration = Duration::from_secs(4);
struct Packet {
    wire: Value,
    reply: oneshot::Sender<Option<Value>>,
}
impl Packet {
    fn ok(self, value: Value) {
        self.reply
            .send(Some(
                json!({"protocol":1,"result":value,"error":null,"outcome_unknown":false}),
            ))
            .unwrap();
    }
    fn refused(self, unknown: bool) {
        self.reply.send(Some(json!({"protocol":1,"result":null,"error":"grant denied SYNTHETIC-PRIVATE","outcome_unknown":unknown}))).unwrap();
    }
    fn lost(self) {
        self.reply.send(None).unwrap();
    }
}
struct Peer {
    address: std::net::SocketAddr,
    packets: mpsc::Receiver<Packet>,
    calls: Arc<Mutex<Vec<Value>>>,
    stop: Option<oneshot::Sender<()>>,
    job: Option<tokio::task::JoinHandle<()>>,
}
impl Peer {
    async fn new(root: &Path) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(
            root.join("process-http.json"),
            json!({"endpoint":format!("http://{address}"),"token":"a".repeat(64)}).to_string(),
        )
        .unwrap();
        std::fs::set_permissions(
            root.join("process-http.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let (send, packets) = mpsc::channel(16);
        let (stop, mut ended) = oneshot::channel();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let capture = calls.clone();
        let job = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=&mut ended=>break,
                    Some(result)=tasks.join_next(),if !tasks.is_empty()=>result.unwrap(),
                    accepted=listener.accept()=>{
                        let(mut socket,_)=accepted.unwrap();let send=send.clone();let capture=capture.clone();
                        tasks.spawn(async move {
                            let mut bytes=Vec::new();let body=tokio::time::timeout(WAIT,async {
                                loop {let mut block=[0u8;4096];let n=socket.read(&mut block).await.unwrap();assert!(n>0);bytes.extend_from_slice(&block[..n]);assert!(bytes.len()<=1024*1024);
                                    if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){
                                        let header=String::from_utf8_lossy(&bytes[..end]);assert!(header.starts_with(&format!("POST {} ", voyage_protocol::vessel::COMMAND_PATH)));
                                        assert!(header.to_ascii_lowercase().contains(&format!("authorization: bearer {}","a".repeat(64))));
                                        let length=header.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").map(|v|v.trim().parse::<usize>().unwrap())).unwrap();
                                        if bytes.len()>=end+4+length {break serde_json::from_slice::<Value>(&bytes[end+4..end+4+length]).unwrap();}
                                    }
                                }
                            }).await.unwrap();
                            assert_eq!(body["protocol"],1);let wire=body["command"].clone();capture.lock().unwrap().push(wire.clone());
                            let(tx,rx)=oneshot::channel();send.send(Packet{wire,reply:tx}).await.unwrap();
                            if let Some(reply)=tokio::time::timeout(WAIT,rx).await.unwrap().unwrap(){let reply=reply.to_string();let header=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",reply.len());let _=socket.write_all(header.as_bytes()).await;}
                            let _=socket.shutdown().await;
                        });
                    }
                }
            }
            drop(listener);
            while let Some(result) = tasks.join_next().await {
                result.unwrap();
            }
        });
        Self {
            address,
            packets,
            calls,
            stop: Some(stop),
            job: Some(job),
        }
    }
    async fn next(&mut self) -> Packet {
        tokio::time::timeout(WAIT, self.packets.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    async fn finish(&mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        tokio::time::timeout(WAIT, self.job.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(self.address).await.is_err());
        assert!(self.packets.try_recv().is_err());
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
struct Fixture {
    root: tempfile::TempDir,
    peer: Peer,
    registry: Arc<ToolRegistry>,
    context: ToolContext,
    owner: Uuid,
    session: Uuid,
    incarnation: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let owner = Uuid::new_v4();
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.path().join("sessions"))
            .unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.path().join("sessions").join(owner.to_string()))
            .unwrap();
        let peer = Peer::new(root.path()).await;
        let tool = VesselTool::new(
            VesselSettings::default(),
            Some(VesselContext {
                session_id: owner,
                directory: root.path().into(),
            }),
        );
        let mut registry = ToolRegistry::default();
        registry.register(tool.clone());
        let mut context = crate::tools::reliability_tests::context(root.path());
        context.max_output_bytes = 8192;
        context.redactor = Arc::new(crate::tools::Redactor::new(["SYNTHETIC-PRIVATE".into()]));
        Self {
            root,
            peer,
            registry: Arc::new(registry),
            context,
            owner,
            session,
            incarnation,
        }
    }
    fn start(&self, args: Value) -> tokio::task::JoinHandle<Result<ToolReport, ToolError>> {
        self.with_context(args, self.context.clone())
    }
    fn with_context(
        &self,
        args: Value,
        ctx: ToolContext,
    ) -> tokio::task::JoinHandle<Result<ToolReport, ToolError>> {
        let registry = self.registry.clone();
        tokio::spawn(async move {
            registry
                .execute_report_with_workflow_secrets("vessel", args, &ctx, None)
                .await
        })
    }
    fn envelope(&self, value: Value) -> Value {
        json!({"session_id":self.session,"incarnation":self.incarnation,"result":value})
    }
    fn reopen_registry(&mut self) {
        let mut registry = ToolRegistry::default();
        registry.register(VesselTool::new(
            VesselSettings::default(),
            Some(VesselContext {
                session_id: self.owner,
                directory: self.root.path().into(),
            }),
        ));
        self.registry = Arc::new(registry);
    }
    fn journal_root(&self) -> PathBuf {
        self.root
            .path()
            .join("sessions")
            .join(self.owner.to_string())
            .join("resources/vessel-coordination")
    }
    fn saved(&self, id: Uuid, suffix: &str) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.journal_root().join(format!("{id}.{suffix}.json"))).unwrap(),
        )
        .unwrap()
    }
    fn rename(&self, id: Uuid) -> Value {
        json!({"action":"rename","session_id":self.session,"command_id":id,"expected_revision":7,"name":"Exact owned Unicode 世界"})
    }
    async fn finish(mut self) {
        self.peer.finish().await;
    }
}
async fn done(
    task: tokio::task::JoinHandle<Result<ToolReport, ToolError>>,
) -> Result<Value, ToolError> {
    let report = tokio::time::timeout(WAIT, task).await.unwrap().unwrap()?;
    Ok(serde_json::from_str(&report.output.text_fallback()).unwrap())
}
fn assert_no_secret(value: &Value) {
    assert!(!value.to_string().contains("SYNTHETIC-PRIVATE"));
    assert!(!value.to_string().contains(&"a".repeat(64)));
}
async fn mutation_unknown(f: &mut Fixture, kind: usize, id: Uuid) -> Value {
    let request = f.rename(id);
    let mut ctx = f.context.clone();
    // Cancel this invocation, not the later manual readonly reopen fixture.
    ctx.cancellation = tokio_util::sync::CancellationToken::new();
    if kind == 2 {
        ctx.timeout = Duration::from_millis(75);
    }
    let task = f.with_context(request.clone(), ctx.clone());
    let packet = f.peer.next().await;
    assert_eq!(packet.wire["op"], "rename");
    assert_eq!(packet.wire["command_id"], id.to_string());
    assert_eq!(f.saved(id, "intent")["request"], request);
    assert_eq!(f.saved(id, "rename.command"), packet.wire);
    let value = match kind {
        0 => {
            packet.lost();
            done(task).await.unwrap()
        }
        1 => {
            ctx.cancellation.cancel();
            let value = done(task).await.unwrap();
            packet.ok(f.envelope(json!({"status":"renamed"})));
            value
        }
        _ => {
            let value = done(task).await.unwrap();
            packet.ok(f.envelope(json!({"status":"renamed"})));
            value
        }
    };
    assert_eq!(value["status"], "outcome_unknown");
    assert_eq!(value["command_id"], id.to_string());
    assert_eq!(value["replayed"], false);
    value
}
#[tokio::test]
async fn whole_registry_success_retains_exact_wire_receipt_and_reopens_without_http_replay() {
    let mut f = Fixture::new().await;
    let id = Uuid::new_v4();
    let request = f.rename(id);
    let task = f.start(request.clone());
    let packet = f.peer.next().await;
    assert_eq!(f.saved(id, "intent")["request"], request);
    assert_eq!(f.saved(id, "rename.command"), packet.wire);
    packet.ok(f.envelope(json!({"command_id":id,"status":"renamed","name":"SYNTHETIC-PRIVATE"})));
    let first = done(task).await.unwrap();
    assert_no_secret(&first);
    let bytes = std::fs::read(f.journal_root().join(format!("{id}.result.json"))).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("SYNTHETIC-PRIVATE"));
    f.reopen_registry();
    assert_eq!(done(f.start(request.clone())).await.unwrap(), first);
    assert_eq!(f.peer.count(), 1);
    let mut changed = request;
    changed["name"] = json!("different");
    assert!(done(f.start(changed)).await.is_err());
    assert_eq!(f.peer.count(), 1);
    f.finish().await;
}
#[tokio::test]
async fn lost_mutation_reply_is_durable_unknown_and_exact_reopen_never_resends() {
    let mut f = Fixture::new().await;
    let id = Uuid::new_v4();
    let first = mutation_unknown(&mut f, 0, id).await;
    f.reopen_registry();
    assert_eq!(done(f.start(f.rename(id))).await.unwrap(), first);
    assert_eq!(f.peer.count(), 1);
    f.finish().await;
}
#[tokio::test]
async fn cancelled_mutation_is_unknown_with_retained_wire_and_no_remote_cancel_or_retry() {
    let mut f = Fixture::new().await;
    let id = Uuid::new_v4();
    let first = mutation_unknown(&mut f, 1, id).await;
    f.reopen_registry();
    assert_eq!(done(f.start(f.rename(id))).await.unwrap(), first);
    assert_eq!(f.peer.count(), 1);
    f.finish().await;
}
#[tokio::test]
async fn timed_out_mutation_is_unknown_not_nonadmission_and_does_not_generate_another_id() {
    let mut f = Fixture::new().await;
    let id = Uuid::new_v4();
    let first = mutation_unknown(&mut f, 2, id).await;
    f.reopen_registry();
    assert_eq!(done(f.start(f.rename(id))).await.unwrap(), first);
    assert_eq!(f.peer.count(), 1);
    f.finish().await;
}
#[tokio::test]
async fn retained_unknown_and_operations_reads_do_not_create_a_remote_executor_or_new_intent() {
    let mut f = Fixture::new().await;
    let id = Uuid::new_v4();
    mutation_unknown(&mut f, 0, id).await;
    let count = f.peer.count();
    let receipt = done(f.start(json!({"action":"receipt","command_id":id})))
        .await
        .unwrap();
    assert_eq!(receipt["result"]["status"], "outcome_unknown");
    let list = done(f.start(json!({"action":"operations","offset":0,"limit":1})))
        .await
        .unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["operations"][0]["command_id"], id.to_string());
    assert_eq!(f.peer.count(), count);
    f.finish().await;
}
#[tokio::test]
async fn readonly_and_cancelled_current_policy_refuse_before_any_private_journal_or_http() {
    let f = Fixture::new().await;
    let id = Uuid::new_v4();
    let mut ctx = f.context.clone();
    ctx.policy = Arc::new(
        crate::policy::Policy::new(
            &crate::Config {
                access: Some(AccessMode::ReadOnly),
                ..Default::default()
            },
            f.root.path().into(),
        )
        .unwrap(),
    );
    assert!(done(f.with_context(f.rename(id), ctx)).await.is_err());
    let ctx = f.context.clone();
    ctx.cancellation.cancel();
    assert!(done(f.with_context(f.rename(id), ctx)).await.is_err());
    assert_eq!(f.peer.count(), 0);
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.0.load(Ordering::SeqCst), "owned authority revoked");
        Ok(())
    }
}
#[tokio::test]
async fn current_authority_withdrawal_never_serves_cached_page_or_new_mutation() {
    let mut f = Fixture::new().await;
    f.context.max_output_bytes = 1600;
    let authority = Arc::new(Authority(AtomicBool::new(false)));
    f.context.policy = Arc::new(
        f.context
            .policy
            .as_ref()
            .clone()
            .with_execution_authority(authority.clone()),
    );
    let request = json!({"action":"run_output","session_id":f.session,"run_id":Uuid::new_v4()});
    let first = page(&mut f, request, &"owned live output ".repeat(1000)).await;
    let count = f.peer.count();
    authority.0.store(true, Ordering::SeqCst);
    assert!(done(f.start(first["next_read"].clone())).await.is_err());
    assert!(done(f.start(f.rename(Uuid::new_v4()))).await.is_err());
    assert_eq!(f.peer.count(), count);
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[tokio::test]
async fn missing_owner_context_refuses_mutation_but_advertises_routes_without_service_start() {
    let mut f = Fixture::new().await;
    let mut registry = ToolRegistry::default();
    registry.register(VesselTool::new(
        VesselSettings {
            local_directory: Some(f.root.path().into()),
            ..Default::default()
        },
        None,
    ));
    f.registry = Arc::new(registry);
    assert!(done(f.start(f.rename(Uuid::new_v4()))).await.is_err());
    let routes = done(f.start(json!({"action":"routes"}))).await.unwrap();
    assert_eq!(routes["automatic_start"], false);
    assert_eq!(f.peer.count(), 0);
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[tokio::test]
async fn malformed_action_unknown_route_and_relative_create_refuse_before_receipt() {
    let f = Fixture::new().await;
    for args in [
        json!({"action":"capabilities","target":"unknown"}),
        json!({"action":"follow","session_id":f.session,"wait_ms":30001}),
        json!({"action":"rename","session_id":f.session,"command_id":Uuid::new_v4(),"expected_revision":0,"name":"   "}),
        json!({"action":"create","command_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"workspace":"relative","task":"owned"}),
        json!({"action":"capabilities","untrusted_url":"http://invalid"}),
    ] {
        assert!(done(f.start(args)).await.is_err());
    }
    assert_eq!(f.peer.count(), 0);
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[tokio::test]
async fn server_scope_refusals_redact_diagnostics_and_never_count_as_live_success() {
    let mut f = Fixture::new().await;
    for unknown in [false, true] {
        let task = f.start(json!({"action":"capabilities"}));
        let packet = f.peer.next().await;
        packet.refused(unknown);
        let value = done(task).await.unwrap();
        assert_eq!(
            value["status"],
            if unknown {
                "outcome_unknown"
            } else {
                "refused"
            }
        );
        assert_eq!(value["code"], "permission_denied");
        assert_no_secret(&value);
    }
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[tokio::test]
async fn wrong_public_session_reply_never_discloses_remote_private_result() {
    let mut f = Fixture::new().await;
    let task = f.start(json!({"action":"inspect","session_id":f.session}));
    let packet = f.peer.next().await;
    assert_eq!(packet.wire["op"], "inspect");
    packet.ok(json!({"session_id":f.session,"incarnation":f.incarnation}));
    let packet = f.peer.next().await;
    assert_eq!(packet.wire["op"], "snapshot");
    packet.ok(json!({"session_id":Uuid::new_v4(),"incarnation":f.incarnation,"result":{"private":"SYNTHETIC-PRIVATE"}}));
    assert!(done(task).await.is_err());
    assert!(!f.journal_root().exists());
    f.finish().await;
}
#[tokio::test]
async fn inspect_retains_registration_and_snapshot_unavailability_without_inventing_cleanup() {
    let mut f = Fixture::new().await;
    let task = f.start(json!({"action":"inspect","session_id":f.session}));
    f.peer
        .next()
        .await
        .ok(json!({"session_id":f.session,"state":"suspended"}));
    f.peer.next().await.refused(false);
    let value = done(task).await.unwrap();
    assert_eq!(value["status"], "snapshot_unavailable");
    assert_eq!(value["registration"]["state"], "suspended");
    assert!(value.get("cleanup_observed").is_none());
    f.finish().await;
}
#[tokio::test]
async fn history_events_and_catalogue_are_whole_registry_readonly_without_persistence() {
    let mut f = Fixture::new().await;
    for action in ["history", "follow", "list"] {
        let args = if action == "list" {
            json!({"action":"list","offset":0,"limit":2})
        } else {
            json!({"action":action,"session_id":f.session,"limit":2})
        };
        let task = f.start(args);
        let packet = f.peer.next().await;
        let value=match action{"history"=>f.envelope(json!({"session_id":f.session,"revision":3,"messages":[{"role":"user","content":"owned café"}],"message_offset":0,"total_messages":1,"next_offset":1,"has_more":false})),"follow"=>f.envelope(json!({"events":[],"cursor":8,"replay_gap":true})),_=>json!([{"session_id":f.session,"name":"owned"}])};
        packet.ok(value);
        let result = done(task).await.unwrap();
        assert_no_secret(&result);
        if action == "follow" {
            assert_eq!(result["next_read"]["action"], "inspect");
        }
    }
    assert!(!f.journal_root().exists());
    f.finish().await;
}
fn chunk(f: &Fixture, wire: &Value, source: &str) -> Value {
    let offset = wire["offset"].as_u64().unwrap() as usize;
    let limit = wire["limit"].as_u64().unwrap() as usize;
    let mut end = (offset + limit).min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let mut value = json!({"offset":offset,"next_offset":end,"total_bytes":source.len(),"data":&source[offset..end],"run_id":wire["run_id"],"state":"running","index":wire["index"],"revision":wire["expected_revision"]});
    if wire["op"] == "message_chunk" {
        value["encoding"] = json!("public_message_json_utf8");
    }
    f.envelope(value)
}
async fn page(f: &mut Fixture, request: Value, source: &str) -> Value {
    let mut task = f.start(request);
    tokio::time::timeout(WAIT, async {
        loop {
            tokio::select! {
                result = &mut task => {
                    let report = result.unwrap().unwrap();
                    return serde_json::from_str(&report.output.text_fallback()).unwrap();
                }
                packet = f.peer.next() => {
                    let wire = packet.wire.clone();
                    assert!(matches!(wire["op"].as_str(), Some("run_output" | "message_chunk")));
                    packet.ok(chunk(f, &wire, source));
                }
            }
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn whole_run_output_pages_preserve_utf8_split_secrets_exact_offsets_and_live_state() {
    let mut f = Fixture::new().await;
    f.context.max_output_bytes = 4096;
    let run = Uuid::new_v4();
    let source = format!(
        "{}SYNTHETIC-PRIVATE{}",
        "é".repeat(32765),
        "世界".repeat(1000)
    );
    let expected = source.replace("SYNTHETIC-PRIVATE", "[REDACTED]");
    let mut request = json!({"action":"run_output","session_id":f.session,"run_id":run,"offset":0});
    let mut output = String::new();
    for _ in 0..100 {
        let value = page(&mut f, request, &source).await;
        assert_no_secret(&value);
        assert_eq!(value["offset"], output.len());
        assert_eq!(value["state"], "running");
        output.push_str(value["data"].as_str().unwrap());
        if value["has_more"] == false {
            assert_eq!(output, expected);
            assert!(
                value["detail"]
                    .as_str()
                    .unwrap()
                    .contains("not proof of task completion")
            );
            f.finish().await;
            return;
        }
        request = value["next_read"].clone();
    }
    panic!("bounded page count exceeded");
}
#[tokio::test]
async fn complete_public_message_json_pages_omit_provider_state_and_preserve_all_canonical_text() {
    let mut f = Fixture::new().await;
    f.context.max_output_bytes = 4096;
    let source=json!({"role":"assistant","content":format!("{}SYNTHETIC-PRIVATE", "café 世界 ".repeat(6000)),"tool_calls":[]}).to_string();
    let mut request = json!({"action":"message","session_id":f.session,"index":4,"expected_revision":3,"offset":0});
    let mut output = String::new();
    for _ in 0..100 {
        let value = page(&mut f, request, &source).await;
        assert_no_secret(&value);
        output.push_str(value["data"].as_str().unwrap());
        if value["has_more"] == false {
            let message: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(message["role"], "assistant");
            let mut expected: Value = serde_json::from_str(&source).unwrap();
            expected["content"] = json!(
                expected["content"]
                    .as_str()
                    .unwrap()
                    .replace("SYNTHETIC-PRIVATE", "[REDACTED]")
            );
            assert_eq!(message, expected);
            assert!(message.get("provider_state").is_none());
            f.finish().await;
            return;
        }
        request = value["next_read"].clone();
    }
    panic!("bounded message page count exceeded");
}
#[tokio::test]
async fn cached_continuation_requires_exact_route_context_and_redactor_without_rpc_on_mismatch() {
    let mut f = Fixture::new().await;
    f.context.max_output_bytes = 1600;
    let run = Uuid::new_v4();
    let source = "owned 世界".repeat(2000);
    let request = json!({"action":"run_output","session_id":f.session,"run_id":run});
    let first = page(&mut f, request, &source).await;
    let next = first["next_read"].clone();
    let count = f.peer.count();
    for field in 0..3 {
        let mut changed = next.clone();
        match field {
            0 => changed["offset"] = json!(999999),
            1 => changed["run_id"] = json!(Uuid::new_v4()),
            _ => changed["session_id"] = json!(Uuid::new_v4()),
        };
        let value = done(f.start(changed)).await.unwrap();
        assert_eq!(value["status"], "cursor_mismatch");
        assert_eq!(f.peer.count(), count);
    }
    let mut ctx = f.context.clone();
    ctx.redactor = Arc::new(crate::tools::Redactor::new(["new-private".into()]));
    let value = done(f.with_context(next, ctx)).await.unwrap();
    assert_eq!(value["status"], "cursor_mismatch");
    assert_eq!(f.peer.count(), count);
    f.finish().await;
}
#[tokio::test]
async fn cached_bytes_recheck_current_remote_refusal_before_any_next_page_is_released() {
    let mut f = Fixture::new().await;
    f.context.max_output_bytes = 1600;
    let source = "private-at-revocation 世界".repeat(1000);
    let request = json!({"action":"run_output","session_id":f.session,"run_id":Uuid::new_v4()});
    let first = page(&mut f, request, &source).await;
    let task = f.start(first["next_read"].clone());
    let packet = f.peer.next().await;
    assert_eq!(packet.wire["offset"], 0);
    assert_eq!(packet.wire["limit"], 4);
    packet.refused(false);
    let value = done(task).await.unwrap();
    assert_eq!(value["status"], "refused");
    assert!(value.get("data").is_none());
    assert!(!value.to_string().contains("private-at-revocation"));
    f.finish().await;
}
#[tokio::test]
async fn wrong_chunk_bounds_revision_or_run_identity_refuse_without_partial_public_page() {
    for field in 0..7 {
        let mut f = Fixture::new().await;
        let run = Uuid::new_v4();
        let task = f.start(json!({"action":"run_output","session_id":f.session,"run_id":run}));
        let packet = f.peer.next().await;
        let mut result = json!({"run_id":run,"offset":0,"next_offset":3,"total_bytes":3,"data":"abc","state":"running"});
        match field {
            0 => result["run_id"] = json!(Uuid::new_v4()),
            1 => result["offset"] = json!(1),
            2 => result["next_offset"] = json!(2),
            3 => result["total_bytes"] = json!(2),
            4 => result["data"] = Value::Null,
            5 => result["data"] = json!(""),
            _ => result["next_offset"] = json!(1.5),
        };
        packet.ok(f.envelope(result));
        assert!(done(task).await.is_err());
        assert!(!f.journal_root().exists());
        f.finish().await;
    }
}

#[tokio::test]
async fn setup_discovery_uses_exact_scoped_commands_without_launch_or_journal() {
    let mut f = Fixture::new().await;
    let binding = json!({"account_id":Uuid::new_v4(),"connection_id":Uuid::new_v4(),
        "identity_generation":4,"connection_revision":8,"transport":"xai_oauth"});
    for action in ["accounts", "profiles", "account_defaults", "account_models"] {
        let mut request = json!({"action":action,"workspace":"/destination/work"});
        if action == "accounts" {
            request["transport"] = json!("xai_oauth");
        }
        if action == "account_models" {
            request["account"] = binding.clone();
        }
        let task = f.start(request.clone());
        let packet = f.peer.next().await;
        assert_eq!(packet.wire["op"], action);
        assert_eq!(packet.wire["workspace"], request["workspace"]);
        if action == "account_models" {
            assert_eq!(packet.wire["account"], binding);
        }
        assert!(packet.wire.get("configuration").is_none());
        assert!(packet.wire.get("command_id").is_none());
        packet.ok(json!({"items":["SYNTHETIC-PRIVATE"],"revision":8}));
        let value = done(task).await.unwrap();
        assert_eq!(value["revision"], 8);
        assert_no_secret(&value);
    }
    let task = f.start(json!({"action":"accounts","workspace":"/destination/work"}));
    f.peer.next().await.refused(false);
    let value = done(task).await.unwrap();
    assert_eq!(value["status"], "refused");
    assert_no_secret(&value);
    let calls = f.peer.count();
    assert!(
        done(f.start(json!({"action":"profiles","workspace":"relative"})))
            .await
            .is_err()
    );
    assert_eq!(f.peer.count(), calls);
    assert!(!f.journal_root().exists());
    f.finish().await;
}

#[tokio::test]
async fn preparation_is_exact_read_and_not_creation_or_resolution() {
    let mut f = Fixture::new().await;
    let account = json!({"account_id":Uuid::new_v4(),"connection_id":Uuid::new_v4(),
        "identity_generation":2,"connection_revision":3,"transport":"anthropic"});
    let request = json!({"action":"prepare","workspace":"/destination/work",
        "account":account,"settings":{"max_output_tokens":0,"reasoning_effort":null}});
    let task = f.start(request.clone());
    let packet = f.peer.next().await;
    assert_eq!(packet.wire["op"], "prepare_start_settings");
    assert_eq!(packet.wire["binding"], account);
    assert_eq!(packet.wire["settings"]["max_output_tokens"], 0);
    assert_eq!(packet.wire["settings"]["reasoning_effort"], Value::Null);
    assert!(packet.wire.get("command_id").is_none());
    packet.ok(json!({"execution_authorized":false,"account":account,
        "workspace":"/destination/work","settings":request["settings"]}));
    let value = done(task).await.unwrap();
    assert_eq!(value["execution_authorized"], false);
    assert!(!f.journal_root().exists());
    f.finish().await;
}
