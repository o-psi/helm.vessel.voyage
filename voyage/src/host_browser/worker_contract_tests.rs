use super::*;
struct Fixture {
    root: tempfile::TempDir,
    launch: Launch,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let guardian = root.path().join("guardian.py");
        std::fs::write(&guardian,r#"
import sys,json,os,shutil
home=os.environ['HOME']; temporary=sys.argv[3]
status={'browser':'11111111-1111-4111-8111-111111111111','tab':'22222222-2222-4222-8222-222222222222','open':True,'mode':'agent','epochs':{'tab':1,'document':1,'viewport':1,'control':1,'capture':1},'tabs':[]}
for line in sys.stdin:
 q=json.loads(line); op=q.get('op'); result={'echo':q.get('payload'),'op':op}
 with open(os.path.join(home,'requests.jsonl'),'a') as f:f.write(json.dumps(q)+'\n')
 if op=='refuse' or (op=='agent' and q['action'].get('kind')=='refuse'): out={'id':q['id'],'ok':False,'error':{'state':'refused','code':'element_hidden'}}
 elif op=='unknown' or (op=='agent' and q['action'].get('kind')=='unknown'): out={'id':q['id'],'ok':False,'error':{'state':'uncertain','code':'private error'}}
 elif op=='die': break
 elif op=='malformed': print('bad json',flush=True);break
 elif op=='duplicate':
  out={'id':q['id'],'ok':True,'result':result};print(json.dumps(out),flush=True)
 else:
  if op in ['status','join','disconnect','input','control','mirror','agent']:
   if op=='control':status['mode']=q['mode'];status['controller']=q['viewer'];status['epochs']['control']+=1;status['epochs']['capture']+=1
   result={'status':status,'value':{'accepted':True}}
   if op=='mirror':result['value']={'cursor':q['since']+1,'events':[{'type':'fixture'}]}
   if op=='input' and q['action']['kind']=='download':result['value']={'download_id':q['action']['download_id'],'data_base64':'Zml4dHVyZQ=='}
  out={'id':q['id'],'ok':True,'result':result}
 print(json.dumps(out),flush=True)
 if op=='shutdown':break
shutil.rmtree(temporary,ignore_errors=True)
with open(os.path.join(home,'guardian-cleanup.json'),'w') as f:json.dump({'observed':True},f)
"#).unwrap();
        let worker = root.path().join("worker.mjs");
        std::fs::write(&worker, b"fixture only").unwrap();
        Self {
            launch: Launch {
                node: "/usr/bin/python3".into(),
                worker,
                chromium: "/bin/true".into(),
                config: default_config(),
            },
            root,
        }
    }
    fn start(&self) -> Arc<Worker> {
        Worker::spawn(&self.launch, self.root.path()).unwrap()
    }
}
#[tokio::test]
async fn framed_exchange_roundtrips_and_shutdown_observes_guardian_cleanup() {
    let fixture = Fixture::new();
    let worker = fixture.start();
    for payload in [json!(null), json!({"unicode":"界"}), json!([1, true, "x"])] {
        let result = worker
            .exchange(json!({"id":Uuid::new_v4(),"op":"echo","payload":payload}))
            .await
            .unwrap();
        assert_eq!(result["echo"], payload);
    }
    let failure = worker
        .exchange(json!({"id":Uuid::new_v4(),"op":"refuse"}))
        .await
        .unwrap_err();
    assert!(failure.downcast_ref::<BeforeEffectRefusal>().is_some());
    assert!(!worker.failed.load(Ordering::Acquire));
    worker.stop().await.unwrap();
    assert!(!worker.temporary.exists());
    assert!(
        worker
            .exchange(json!({"id":Uuid::new_v4(),"op":"echo"}))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_or_lost_effect_reply_poison_transport_and_never_replay() {
    for op in ["die", "malformed"] {
        let fixture = Fixture::new();
        let worker = fixture.start();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            worker.exchange(json!({"id":Uuid::new_v4(),"op":op})),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(worker.failed.load(Ordering::Acquire));
        assert!(
            worker
                .exchange(json!({"id":Uuid::new_v4(),"op":"echo"}))
                .await
                .is_err()
        );
        worker.stop().await.unwrap();
    }
}
#[tokio::test]
async fn invalid_identity_oversize_and_pending_capacity_refuse_before_dispatch() {
    let fixture = Fixture::new();
    let worker = fixture.start();
    assert!(worker.exchange(json!({"op":"echo"})).await.is_err());
    assert!(
        worker
            .exchange(json!({"id":Uuid::new_v4(),"payload":"x".repeat(MAX_FRAME)}))
            .await
            .is_err()
    );
    let id = Uuid::new_v4();
    let (tx, _rx) = oneshot::channel();
    worker.pending.lock().unwrap().insert(id, tx);
    assert!(worker.exchange(json!({"id":id,"op":"echo"})).await.is_err());
    worker.pending.lock().unwrap().clear();
    worker.stop().await.unwrap();
}
#[test]
fn all_human_inputs_keep_typed_frame_and_action_identity() {
    for action in [
        json!({"type":"history","direction":"reload"}),
        json!({"type":"click","node_id":7,"button":"left"}),
        json!({"type":"surface_click","node_id":7,"x":1,"y":2,"button":"right"}),
        json!({"type":"fill","node_id":7,"text":"private"}),
        json!({"type":"select","node_id":7,"value":"choice"}),
        json!({"type":"wheel","node_id":7,"delta_x":1,"delta_y":-2}),
        json!({"type":"key","key":"Enter","pressed":false}),
        json!({"type":"text","text":"fixture"}),
        json!({"type":"scroll","delta_x":3,"delta_y":4}),
        json!({"type":"resize","width":800,"height":600}),
        json!({"type":"dialog","accept":true,"text":null}),
        json!({"type":"navigate","url":"https://example.invalid"}),
    ] {
        let typed: HostBrowserInput = serde_json::from_value(action.clone()).unwrap();
        let translated = input_action(typed).unwrap();
        assert!(translated["kind"].is_string());
        if let Some(id) = action.get("node_id") {
            assert_eq!(&translated["node_id"], id);
        }
    }
}

#[tokio::test]
async fn synthetic_viewer_lifecycle_fences_sequence_identity_and_disconnected_socket() {
    let fixture = Fixture::new();
    let worker = fixture.start();
    let host = HostBrowser::new(
        fixture.root.path().into(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        None,
    );
    let status = worker
        .exchange(json!({"id":Uuid::new_v4(),"op":"status"}))
        .await
        .unwrap();
    host.update_status(&status).await;
    host.inner.lock().await.worker = Some(worker.clone());
    let principal = Uuid::new_v4();
    let socket = Uuid::new_v4();
    let binding = host.binding(&status["status"], Uuid::new_v4()).unwrap();
    let attach = HostBrowserOperation::Attach {
        command_id: Uuid::new_v4(),
        binding: binding.clone(),
    };
    let first = host
        .human(attach.clone(), socket, principal, None)
        .await
        .unwrap();
    assert!(first["status"].is_object());
    let retry = host.human(attach, socket, principal, None).await.unwrap();
    assert!(retry["receipt"].is_object());
    let input = HostBrowserOperation::Input {
        command_id: Uuid::new_v4(),
        binding: binding.clone(),
        sequence: 1,
        claim: false,
        input: HostBrowserInput::Text {
            text: "private fixture".into(),
        },
    };
    let result = host
        .human(input.clone(), socket, principal, None)
        .await
        .unwrap();
    assert!(result["value"].is_object());
    let replay = host.human(input, socket, principal, None).await.unwrap();
    assert_eq!(replay["receipt"]["state"], "completed");
    assert!(
        host.human(
            HostBrowserOperation::Input {
                command_id: Uuid::new_v4(),
                binding: binding.clone(),
                sequence: 1,
                claim: false,
                input: HostBrowserInput::Text {
                    text: "not replayed".into()
                }
            },
            principal,
            socket,
            None
        )
        .await
        .is_err()
    );
    assert!(
        host.human(
            HostBrowserOperation::Detach {
                command_id: Uuid::new_v4(),
                binding: binding.clone()
            },
            Uuid::new_v4(),
            socket,
            None
        )
        .await
        .is_err()
    );
    host.human(
        HostBrowserOperation::Detach {
            command_id: Uuid::new_v4(),
            binding,
        },
        socket,
        principal,
        None,
    )
    .await
    .unwrap();
    assert!(host.inner.lock().await.viewers.is_empty());
    host.disconnect(socket).await.unwrap();
    assert!(
        host.human(HostBrowserOperation::Status {}, socket, principal, None)
            .await
            .is_err()
    );
    host.close().await.unwrap();
    assert!(!host.blocks_suspension().await);
}

#[tokio::test]
async fn stale_binding_and_revoked_viewer_never_dispatch_private_input() {
    let fixture = Fixture::new();
    let worker = fixture.start();
    let host = HostBrowser::new(
        fixture.root.path().into(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        None,
    );
    let status = worker
        .exchange(json!({"id":Uuid::new_v4(),"op":"status"}))
        .await
        .unwrap();
    host.update_status(&status).await;
    host.inner.lock().await.worker = Some(worker);
    let principal = Uuid::new_v4();
    let socket = Uuid::new_v4();
    let binding = host.binding(&status["status"], Uuid::new_v4()).unwrap();
    host.human(
        HostBrowserOperation::Attach {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
        },
        socket,
        principal,
        None,
    )
    .await
    .unwrap();
    for stale in ["document", "viewport", "capture", "controller"] {
        let mut b = binding.clone();
        match stale {
            "document" => b.document_epoch += 1,
            "viewport" => b.viewport_epoch += 1,
            "capture" => b.capture_epoch += 1,
            _ => b.controller_epoch += 1,
        };
        let id = Uuid::new_v4();
        assert!(
            host.human(
                HostBrowserOperation::Input {
                    command_id: id,
                    binding: b,
                    sequence: 1,
                    claim: false,
                    input: HostBrowserInput::Text {
                        text: "never dispatched".into()
                    }
                },
                socket,
                principal,
                None
            )
            .await
            .is_err()
        );
        assert_eq!(
            host.inner.lock().await.viewers[&binding.attachment_id].input_sequence,
            0
        );
    }
    host.inner
        .lock()
        .await
        .viewers
        .get_mut(&binding.attachment_id)
        .unwrap()
        .seen = std::time::Instant::now() - Duration::from_secs(30);
    host.maintain().await.unwrap();
    assert!(host.inner.lock().await.viewers.is_empty());
    assert!(host.inner.lock().await.disconnected.contains(&socket));
    host.close().await.unwrap();
}
#[tokio::test]
async fn durable_receipt_owner_and_hash_conflicts_remain_fenced_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let session = Uuid::new_v4();
    let host = HostBrowser::new(root.path().into(), session, Uuid::new_v4(), None);
    let id = Uuid::new_v4();
    let principal = Uuid::new_v4();
    assert!(host.receipt(id, principal, None, None).unwrap().is_none());
    assert!(
        host.receipt(id, principal, Some("exact"), None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        host.receipt(id, principal, None, None).unwrap().unwrap()["state"],
        "unknown"
    );
    host.receipt(id, principal, Some("exact"), Some("completed"))
        .unwrap();
    let reopened = HostBrowser::new(root.path().into(), session, Uuid::new_v4(), None);
    assert_eq!(
        reopened
            .receipt(id, principal, None, None)
            .unwrap()
            .unwrap()["state"],
        "completed"
    );
    assert!(
        reopened
            .receipt(id, principal, Some("changed"), None)
            .is_err()
    );
    assert!(reopened.receipt(id, Uuid::new_v4(), None, None).is_err());
}

impl Fixture {
    // The worker is a real guardian-owned subprocess; only its browser endpoint
    // is synthetic. Seeding ownership avoids host-global capacity reservations.
    async fn host(&self) -> Arc<HostBrowser> {
        let host = HostBrowser::new(
            self.root.path().into(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            None,
        );
        let worker = self.start();
        host.refresh(&worker).await.unwrap();
        host.inner.lock().await.worker = Some(worker);
        host
    }
    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.path().join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    async fn attach(
        &self,
        host: &HostBrowser,
        socket: Uuid,
        principal: Uuid,
    ) -> HostBrowserBinding {
        let binding = host
            .binding(&host.inner.lock().await.status, Uuid::new_v4())
            .unwrap();
        host.human(
            HostBrowserOperation::Attach {
                command_id: Uuid::new_v4(),
                binding: binding.clone(),
            },
            socket,
            principal,
            None,
        )
        .await
        .unwrap();
        binding
    }
}

#[tokio::test]
async fn start_reuses_owned_transport_and_refuses_unresolved_or_stale_start() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let worker = host.inner.lock().await.worker.clone().unwrap();
    assert!(Arc::ptr_eq(&host.start().await.unwrap(), &worker));
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let start = HostBrowserOperation::Start {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        incarnation: host.incarnation,
    };
    let before = fixture.requests().len();
    let reply = host
        .human(start.clone(), socket, principal, None)
        .await
        .unwrap();
    assert_eq!(reply["status"]["running"], true);
    assert_eq!(fixture.requests().len(), before);
    assert_eq!(
        host.human(start, socket, principal, None).await.unwrap()["receipt"]["state"],
        "completed"
    );
    let error = host
        .human(
            HostBrowserOperation::Start {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                incarnation: Uuid::new_v4(),
            },
            socket,
            principal,
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "stale browser incarnation");
    host.inner.lock().await.status = Value::Null;
    assert_eq!(
        host.start().await.err().unwrap().to_string(),
        "browser startup unresolved"
    );
    worker.failed.store(true, Ordering::Release);
    assert_eq!(
        host.start().await.err().unwrap().to_string(),
        "browser transport unresolved; close required"
    );
    assert_eq!(
        host.human(HostBrowserOperation::Status {}, socket, principal, None)
            .await
            .unwrap_err()
            .to_string(),
        "browser transport unresolved"
    );
    host.finish_run().await.unwrap();
    assert!(!host.blocks_suspension().await);
    assert!(!worker.temporary.exists());
}

#[tokio::test]
async fn mirror_cursor_reset_and_attachment_authority_are_exact() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let binding = fixture.attach(&host, socket, principal).await;
    let mirror = HostBrowserOperation::Mirror {
        binding: binding.clone(),
        since: 41,
    };
    let reply = host
        .human(mirror.clone(), socket, principal, None)
        .await
        .unwrap();
    assert_eq!(
        reply["value"],
        json!({"cursor":42,"events":[{"type":"fixture"}]})
    );
    assert_eq!(reply["status"]["binding"], json!(binding));
    let last = fixture.requests().pop().unwrap();
    assert_eq!(last["op"], "mirror");
    assert_eq!(last["viewer"], json!(binding.attachment_id));
    assert_eq!(last["since"], 41);
    let count = fixture.requests().len();
    for (s, p) in [(Uuid::new_v4(), principal), (socket, Uuid::new_v4())] {
        assert_eq!(
            host.human(mirror.clone(), s, p, None)
                .await
                .unwrap_err()
                .to_string(),
            "browser attachment authority mismatch"
        );
    }
    let mut stale = binding.clone();
    stale.document_epoch += 1;
    let reply = host
        .human(
            HostBrowserOperation::Mirror {
                binding: stale,
                since: 41,
            },
            socket,
            principal,
            None,
        )
        .await
        .unwrap();
    assert_eq!(reply["value"], json!({"reset":true,"cursor":0,"events":[]}));
    assert_eq!(reply["status"]["binding"], json!(binding));
    assert_eq!(fixture.requests().len(), count);
    host.close().await.unwrap();
}

#[tokio::test]
async fn download_receipts_are_ephemeral_and_detach_permanently_retires_identity() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let binding = fixture.attach(&host, socket, principal).await;
    let id = Uuid::new_v4();
    let download_id = Uuid::new_v4();
    let input = HostBrowserOperation::Input {
        command_id: id,
        binding: binding.clone(),
        sequence: 1,
        claim: false,
        input: HostBrowserInput::Download { download_id },
    };
    let fence = host.fence.load(Ordering::Acquire);
    let reply = host
        .human(input.clone(), socket, principal, None)
        .await
        .unwrap();
    assert_eq!(
        reply["value"],
        json!({"download_id":download_id,"data_base64":"Zml4dHVyZQ=="})
    );
    assert_eq!(reply["status"]["input_sequence"], 1);
    assert_eq!(host.fence.load(Ordering::Acquire), fence);
    let sent = fixture.requests().pop().unwrap();
    assert_eq!(
        sent["action"],
        json!({"kind":"download","download_id":download_id})
    );
    assert_eq!(sent["seq"], 1);
    assert_eq!(sent["claim"], false);
    let count = fixture.requests().len();
    assert_eq!(
        host.human(input.clone(), socket, principal, None)
            .await
            .unwrap(),
        json!({"receipt":{"command_id":id,"state":"completed","content_withheld":true}})
    );
    assert_eq!(
        host.human(
            HostBrowserOperation::Receipt { command_id: id },
            socket,
            principal,
            None
        )
        .await
        .unwrap()["receipt"]["state"],
        "completed"
    );
    assert_eq!(
        host.human(
            HostBrowserOperation::Receipt { command_id: id },
            socket,
            Uuid::new_v4(),
            None
        )
        .await
        .unwrap_err()
        .to_string(),
        "browser receipt principal mismatch"
    );
    assert_eq!(
        host.human(input.clone(), Uuid::new_v4(), principal, None)
            .await
            .unwrap_err()
            .to_string(),
        "browser live receipt identity conflict"
    );
    assert_eq!(fixture.requests().len(), count);
    assert!(host.receipt(id, principal, None, None).unwrap().is_none());
    host.human(
        HostBrowserOperation::Detach {
            command_id: Uuid::new_v4(),
            binding,
        },
        socket,
        principal,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        host.human(input, socket, principal, None)
            .await
            .unwrap_err()
            .to_string(),
        "browser input identity retired; never replay"
    );
    assert!(host.inner.lock().await.input_receipts.is_empty());
    host.close().await.unwrap();
}

#[tokio::test]
async fn private_control_disconnect_and_same_principal_rejoin_preserve_owner() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let binding = fixture.attach(&host, socket, principal).await;
    let operation: HostBrowserOperation = serde_json::from_value(
        json!({"action":"control","command_id":Uuid::new_v4(),"binding":binding,"mode":"private"}),
    )
    .unwrap();
    let reply = host
        .human(operation, socket, principal, None)
        .await
        .unwrap();
    assert_eq!(reply["status"]["mode"], "private");
    assert_eq!(reply["status"]["controller"], json!(binding.attachment_id));
    assert_eq!(reply["status"]["binding"]["controller_epoch"], 2);
    assert_eq!(reply["status"]["binding"]["capture_epoch"], 2);
    let current: HostBrowserBinding =
        serde_json::from_value(reply["status"]["binding"].clone()).unwrap();
    assert_eq!(
        host.human(
            HostBrowserOperation::Input {
                command_id: Uuid::new_v4(),
                binding: current,
                sequence: 1,
                claim: true,
                input: HostBrowserInput::Click {
                    node_id: 1,
                    button: HostBrowserButton::Left
                }
            },
            socket,
            principal,
            None
        )
        .await
        .unwrap_err()
        .to_string(),
        "browser already controlled"
    );
    host.disconnect(socket).await.unwrap();
    assert_eq!(
        host.inner.lock().await.private_owner,
        Some((binding.attachment_id, principal))
    );
    assert!(host.inner.lock().await.viewers.is_empty());
    let new_socket = Uuid::new_v4();
    let fresh = host
        .binding(&host.inner.lock().await.status, Uuid::new_v4())
        .unwrap();
    let count = fixture.requests().len();
    assert_eq!(
        host.human(
            HostBrowserOperation::Attach {
                command_id: Uuid::new_v4(),
                binding: fresh.clone()
            },
            new_socket,
            Uuid::new_v4(),
            None
        )
        .await
        .unwrap_err()
        .to_string(),
        "private browser belongs to another principal"
    );
    assert_eq!(fixture.requests().len(), count);
    let joined = host
        .human(
            HostBrowserOperation::Attach {
                command_id: Uuid::new_v4(),
                binding: fresh,
            },
            new_socket,
            principal,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        joined["status"]["binding"]["attachment_id"],
        json!(binding.attachment_id)
    );
    assert_eq!(
        fixture.requests().pop().unwrap()["viewer"],
        json!(binding.attachment_id)
    );
    let current = serde_json::from_value(joined["status"]["binding"].clone()).unwrap();
    host.human(
        HostBrowserOperation::Close {
            command_id: Uuid::new_v4(),
            binding: current,
        },
        new_socket,
        principal,
        None,
    )
    .await
    .unwrap();
    assert!(host.inner.lock().await.private_owner.is_none());
    assert!(!host.blocks_suspension().await);
}

#[tokio::test]
async fn agent_success_refusal_and_uncertainty_have_distinct_durable_outcomes() {
    for (kind, expected) in [
        ("observe", "completed"),
        ("refuse", "refused"),
        ("unknown", "unknown"),
    ] {
        let fixture = Fixture::new();
        let host = fixture.host().await;
        let context = crate::tools::reliability_tests::context(fixture.root.path());
        let id = Uuid::new_v4();
        let action = json!({"kind":kind});
        let result = host.agent(id, action.clone(), &context).await;
        match kind {
            "observe" => assert_eq!(result.unwrap(), json!({"accepted":true})),
            "refuse" => assert!(result.unwrap_err().is::<BeforeEffectRefusal>()),
            _ => {
                assert!(result.is_err());
                assert!(!host.blocks_suspension().await);
            }
        }
        assert_eq!(
            host.receipt(id, host.session, None, None).unwrap().unwrap()["state"],
            expected
        );
        let sent: Vec<_> = fixture
            .requests()
            .into_iter()
            .filter(|q| q["op"] == "agent")
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["id"], json!(id));
        assert_eq!(sent[0]["action"], action);
        if kind != "unknown" {
            assert_eq!(
                host.agent(id, action, &context).await.unwrap(),
                json!({"receipt":{"command_id":id,"state":expected,"content_withheld":true},"content_withheld":true})
            );
            assert_eq!(
                fixture
                    .requests()
                    .iter()
                    .filter(|q| q["op"] == "agent")
                    .count(),
                1
            );
            host.finish_run().await.unwrap();
            assert!(host.blocks_suspension().await);
        }
        host.close().await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_and_human_controlled_agent_calls_never_dispatch() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let context = crate::tools::reliability_tests::context(fixture.root.path());
    context.cancellation.cancel();
    let count = fixture.requests().len();
    assert_eq!(
        host.agent(Uuid::new_v4(), json!({"kind":"observe"}), &context)
            .await
            .unwrap_err()
            .to_string(),
        "browser call cancelled"
    );
    assert_eq!(fixture.requests().len(), count);
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let binding = fixture.attach(&host, socket, principal).await;
    let operation = serde_json::from_value(
        json!({"action":"control","command_id":Uuid::new_v4(),"binding":binding,"mode":"human"}),
    )
    .unwrap();
    host.human(operation, socket, principal, None)
        .await
        .unwrap();
    let context = crate::tools::reliability_tests::context(fixture.root.path());
    assert_eq!(
        host.agent(Uuid::new_v4(), json!({"kind":"observe"}), &context)
            .await
            .unwrap_err()
            .to_string(),
        "browser controlled by human or private operator"
    );
    assert_eq!(
        fixture
            .requests()
            .iter()
            .filter(|q| q["op"] == "agent")
            .count(),
        0
    );
    host.close().await.unwrap();
}

#[test]
fn frame_upload_tab_and_key_translations_preserve_exact_payloads() {
    let frame = Uuid::new_v4();
    let cases = [
        (
            json!({"type":"click","node_id":7,"button":"left"}),
            json!({"kind":"element","action":"click","node_id":7,"button":"left"}),
        ),
        (
            json!({"type":"surface_click","node_id":7,"x":1,"y":2,"button":"right"}),
            json!({"kind":"element","action":"surface_click","node_id":7,"x":1,"y":2,"button":"right"}),
        ),
        (
            json!({"type":"fill","node_id":7,"text":"secret"}),
            json!({"kind":"element","action":"fill","node_id":7,"text":"secret"}),
        ),
        (
            json!({"type":"select","node_id":7,"value":"choice"}),
            json!({"kind":"element","action":"select","node_id":7,"value":"choice"}),
        ),
        (
            json!({"type":"wheel","node_id":7,"delta_x":1,"delta_y":-2}),
            json!({"kind":"element","action":"wheel","node_id":7,"x":1,"y":-2}),
        ),
        (
            json!({"type":"upload","node_id":7,"name":"fixture.txt","mime_type":"text/plain","data_base64":"eA=="}),
            json!({"kind":"element","action":"upload","node_id":7,"name":"fixture.txt","mime_type":"text/plain","data_base64":"eA=="}),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            input_action(serde_json::from_value(input.clone()).unwrap()).unwrap(),
            expected
        );
        let mut expected = expected;
        expected["frame_id"] = json!(frame);
        let framed = json!({"type":"frame_element","frame_id":frame,"input":input});
        assert_eq!(
            input_action(serde_json::from_value(framed).unwrap()).unwrap(),
            expected
        );
    }
    let tab = Uuid::new_v4();
    for operation in ["new", "select", "close"] {
        let input = json!({"type":"tab","operation":operation,"tab_id":tab});
        assert_eq!(
            input_action(serde_json::from_value(input).unwrap()).unwrap(),
            json!({"kind":"tabs","operation":operation,"tab":tab})
        );
    }
    for pressed in [true, false] {
        assert_eq!(
            input_action(HostBrowserInput::Key {
                key: "Enter".into(),
                pressed
            })
            .unwrap(),
            json!({"kind":"key","type":if pressed {"down"} else {"up"},"key":"Enter"})
        );
    }
}

#[tokio::test]
async fn start_with_prior_worker_lock_refuses_before_launch_or_reservation() {
    let fixture = Fixture::new();
    let host = HostBrowser::new(
        fixture.root.path().into(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Some(fixture.launch.clone()),
    );
    let root = host.root().unwrap();
    std::fs::write(root.join("worker.lock"), b"unresolved owner").unwrap();
    assert_eq!(
        host.start().await.err().unwrap().to_string(),
        "prior host browser worker lock remains; inspect cleanup before recovery"
    );
    let inner = host.inner.lock().await;
    assert!(inner.worker.is_none());
    assert!(inner.capacity.is_none());
    assert!(inner.reservation.is_none());
    assert!(!inner.starting);
    assert_eq!(
        std::fs::read(root.join("worker.lock")).unwrap(),
        b"unresolved owner"
    );
    assert!(!fixture.root.path().join("requests.jsonl").exists());
}

#[derive(Debug)]
struct TestAuthority(AtomicBool);
impl crate::policy::ExecutionAuthority for TestAuthority {
    fn check(&self) -> Result<()> {
        ensure!(self.0.load(Ordering::Acquire), "fixture authority revoked");
        Ok(())
    }
}

#[tokio::test]
async fn revoked_attached_authority_blocks_mirror_then_maintenance_disconnects() {
    let fixture = Fixture::new();
    let host = fixture.host().await;
    let socket = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let binding = host
        .binding(&host.inner.lock().await.status, Uuid::new_v4())
        .unwrap();
    let authority = Arc::new(TestAuthority(AtomicBool::new(true)));
    host.human(
        HostBrowserOperation::Attach {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
        },
        socket,
        principal,
        Some(authority.clone()),
    )
    .await
    .unwrap();
    authority.0.store(false, Ordering::Release);
    let count = fixture.requests().len();
    assert_eq!(
        host.human(
            HostBrowserOperation::Mirror {
                binding: binding.clone(),
                since: 0
            },
            socket,
            principal,
            None
        )
        .await
        .unwrap_err()
        .to_string(),
        "fixture authority revoked"
    );
    assert_eq!(
        host.human(
            HostBrowserOperation::Status {},
            socket,
            principal,
            Some(authority)
        )
        .await
        .unwrap_err()
        .to_string(),
        "fixture authority revoked"
    );
    assert_eq!(fixture.requests().len(), count);
    host.maintain().await.unwrap();
    assert!(host.inner.lock().await.viewers.is_empty());
    assert!(host.inner.lock().await.disconnected.contains(&socket));
    let sent = fixture.requests().pop().unwrap();
    assert_eq!(sent["op"], "disconnect");
    assert_eq!(sent["viewer"], json!(binding.attachment_id));
    host.close().await.unwrap();
}
