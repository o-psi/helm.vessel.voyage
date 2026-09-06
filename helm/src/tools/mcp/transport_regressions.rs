//! Real stdio regression peers. Tests precede the production correction.
use super::*;
use std::time::Duration;

const PEER: &str = r#"
import json, os, pathlib, sys, time
mode, directory = sys.argv[1:]
root = pathlib.Path(directory)
def mark(name, value='ready'):
    temporary = root / (name + '.tmp')
    temporary.write_text(value)
    temporary.replace(root / name)
def reply(value):
    print(json.dumps(value, separators=(',',':')), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method')
    if method == 'notifications/cancelled':
        mark('cancelled', json.dumps(request))
        continue
    if method == 'notifications/initialized':
        mark('initialized')
        if mode == 'block_read': time.sleep(30)
        continue
    identifier = request['id']
    if method == 'initialize':
        if mode == 'init_hold':
            mark('init_dispatched', str(identifier))
            continue
        if mode == 'init_error':
            reply({'jsonrpc':'2.0','id':identifier,'error':{'code':-32602,'message':'unsupported'}})
        elif mode == 'init_version':
            reply({'jsonrpc':'2.0','id':identifier,'result':{'protocolVersion':'2099-01-01','capabilities':{},'serverInfo':{'name':'fixture','version':'1'}}})
        elif mode == 'init_missing':
            reply({'jsonrpc':'2.0','id':identifier,'result':{}})
        else:
            reply({'jsonrpc':'2.0','id':identifier,'result':{'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}})
    elif method == 'tools/list':
        reply({'jsonrpc':'2.0','id':identifier,'result':{'tools':[{'name':'echo','inputSchema':{'type':'object'}}]}})
    else:
        mark('dispatched', str(identifier))
        if mode == 'oversized':
            sys.stdout.write('x' * (1024 * 1024 + 1)); sys.stdout.flush()
            time.sleep(30)
        elif mode in ('flood', 'flood_exact'):
            for i in range(129 if mode == 'flood' else 128):
                reply({'jsonrpc':'2.0','method':'notifications/progress','params':{}})
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[]}})
        elif mode in ('frame_exact', 'frame_plus_one'):
            raw = json.dumps({'jsonrpc':'2.0','id':identifier,'result':{'content':[]}},separators=(',',':'))
            size = 1024 * 1024 + (mode == 'frame_plus_one')
            sys.stdout.write(raw + ' ' * (size - len(raw)) + '\n'); sys.stdout.flush()
        elif mode == 'unicode':
            raw = json.dumps({'jsonrpc':'2.0','id':identifier,'result':{'content':[{'type':'text','text':'雪é'}]}},ensure_ascii=False).encode() + b'\n'
            for byte in raw:
                os.write(sys.stdout.fileno(), bytes([byte]))
        elif mode == 'late':
            reply({'jsonrpc':'2.0','id':identifier-1,'result':{'content':[{'type':'text','text':'WRONG'}]}})
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[{'type':'text','text':'ok'}]}})
        elif mode == 'gate':
            while not (root / 'release').exists(): time.sleep(.002)
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[{'type':'text','text':'ok'}]}})
        elif mode == 'peer_ping':
            reply({'jsonrpc':'2.0','id':'p' * (1024 * 1024 - 100),'method':'ping'})
            mark('peer_ping')
            while not (root / 'release').exists(): time.sleep(.002)
            mark('peer_input', sys.stdin.read())
            break
        elif mode == 'hold':
            pass
        elif mode == 'partial':
            sys.stdout.write('{"jsonrpc":"2.0",'); sys.stdout.flush()
            mark('partial')
        else:
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[{'type':'text','text':'ok'}]}})
"#;

fn peer(mode: &str, directory: &std::path::Path) -> Arc<McpServer> {
    Arc::new(
        McpServer::start(
            "fixture",
            "python3",
            &[
                "-u".into(),
                "-c".into(),
                PEER.into(),
                mode.into(),
                directory.to_string_lossy().into_owned(),
            ],
            &BTreeMap::new(),
        )
        .unwrap(),
    )
}

async fn marker(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("fixture did not reach observed barrier");
}

#[tokio::test]
async fn invalid_negotiation_stops_before_initialized() {
    for mode in ["init_error", "init_version", "init_missing"] {
        let directory = tempfile::tempdir().unwrap();
        let server = peer(mode, directory.path());
        let result = server.initialize().await;
        server.shutdown().await.unwrap();
        assert!(result.is_err(), "accepted {mode}");
        assert!(!directory.path().join("initialized").exists());
    }
}

#[tokio::test]
async fn newline_free_frame_is_rejected_before_peer_closes() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("oversized", directory.path());
    server.initialize().await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        server.transport.request("tools/call", json!({})),
    )
    .await;
    server.shutdown().await.unwrap();
    assert!(
        matches!(result, Ok(Err(_))),
        "unbounded frame waited for newline: {result:?}"
    );
}

#[tokio::test]
async fn unrelated_frame_flood_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("flood", directory.path());
    server.initialize().await.unwrap();
    let result = server.transport.request("tools/call", json!({})).await;
    server.shutdown().await.unwrap();
    assert!(result.is_err(), "accepted excess unrelated frames");
}

#[tokio::test]
async fn dropped_dispatched_and_partial_read_call_retire_transport() {
    for mode in ["hold", "partial"] {
        let directory = tempfile::tempdir().unwrap();
        let server = peer(mode, directory.path());
        server.initialize().await.unwrap();
        let transport = server.transport.clone();
        let task = tokio::spawn(async move { transport.request("tools/call", json!({})).await });
        marker(&directory.path().join(if mode == "partial" {
            "partial"
        } else {
            "dispatched"
        }))
        .await;
        task.abort();
        let _ = task.await;
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            server.transport.request("tools/call", json!({})),
        )
        .await;
        server.shutdown().await.unwrap();
        assert!(
            matches!(result, Ok(Err(_))),
            "abandoned call transport remained usable or hung: {result:?}"
        );
    }
}

#[tokio::test]
async fn exact_frame_and_unrelated_limits_unicode_and_late_ids() {
    for mode in [
        "frame_exact",
        "frame_plus_one",
        "flood_exact",
        "unicode",
        "late",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let server = peer(mode, directory.path());
        server.initialize().await.unwrap();
        let result = server.transport.request("tools/call", json!({})).await;
        server.shutdown().await.unwrap();
        if mode == "frame_plus_one" {
            assert!(result.is_err());
        } else {
            let result = result.unwrap();
            if mode == "unicode" {
                assert_eq!(extract_content(&result["result"]), "雪é");
            }
            if mode == "late" {
                assert_eq!(extract_content(&result["result"]), "ok");
            }
        }
    }
}

#[test]
fn outgoing_limit_counts_encoded_utf8_without_delimiter() {
    let exact = json!("x".repeat(MAX_FRAME_BYTES - 2));
    assert_eq!(encode_frame(&exact).unwrap().len(), MAX_FRAME_BYTES + 1);
    assert!(encode_frame(&json!("x".repeat(MAX_FRAME_BYTES - 1))).is_err());
    assert!(encode_frame(&json!("\n".repeat(MAX_FRAME_BYTES / 2))).is_err());
    assert_eq!(encode_frame(&json!("雪")).unwrap(), "\"雪\"\n".as_bytes());
}

#[test]
fn response_envelopes_reject_malformed_and_ambiguous_values() {
    for value in [
        json!({"jsonrpc":"1.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":null,"result":{}}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":"x"}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":"bad","message":"x"}}),
        json!({"jsonrpc":"2.0","id":1,"method":7}),
        json!({"jsonrpc":"2.0","method":"ping","result":{}}),
        json!([]),
    ] {
        assert!(validate_envelope(&value).is_err(), "accepted {value}");
    }
    for value in [
        json!({"jsonrpc":"2.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":"peer","method":"ping"}),
        json!({"jsonrpc":"2.0","method":"notifications/progress","params":{}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"x"}}),
    ] {
        validate_envelope(&value).unwrap();
    }
}

#[tokio::test]
async fn oversized_outgoing_request_has_no_effect_and_preserves_connection() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("echo", directory.path());
    server.initialize().await.unwrap();
    assert!(
        server
            .transport
            .request("tools/call", json!({"value":"x".repeat(MAX_FRAME_BYTES)}))
            .await
            .is_err()
    );
    assert!(!directory.path().join("dispatched").exists());
    assert!(
        server
            .transport
            .request("tools/call", json!({}))
            .await
            .is_ok()
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropped_queued_request_sends_nothing_and_does_not_retire_active_call() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("gate", directory.path());
    server.initialize().await.unwrap();
    let transport = server.transport.clone();
    let task = tokio::spawn(async move { transport.request("tools/call", json!({})).await });
    marker(&directory.path().join("dispatched")).await;
    let before = server.transport.next_id.load(Ordering::Acquire);
    {
        let queued = server.transport.request("tools/call", json!({}));
        tokio::pin!(queued);
        assert!(futures_util::poll!(&mut queued).is_pending());
    }
    assert_eq!(server.transport.next_id.load(Ordering::Acquire), before);
    assert!(!server.transport.closed.load(Ordering::Acquire));
    std::fs::write(directory.path().join("release"), "release").unwrap();
    assert!(task.await.unwrap().is_ok());
    assert!(
        server
            .transport
            .request("tools/call", json!({}))
            .await
            .is_ok()
    );
    server.shutdown().await.unwrap();
    assert!(!directory.path().join("cancelled").exists());
}

#[tokio::test]
async fn dropped_request_cancellation_is_exact_but_initialize_is_never_cancelled() {
    for mode in ["hold", "init_hold"] {
        let directory = tempfile::tempdir().unwrap();
        let server = peer(mode, directory.path());
        if mode == "hold" {
            server.initialize().await.unwrap();
        }
        let transport = server.transport.clone();
        let task = tokio::spawn(async move {
            transport
                .request(
                    if mode == "hold" {
                        "tools/call"
                    } else {
                        "initialize"
                    },
                    json!({}),
                )
                .await
        });
        let path = directory.path().join(if mode == "hold" {
            "dispatched"
        } else {
            "init_dispatched"
        });
        marker(&path).await;
        let id: u64 = std::fs::read_to_string(path).unwrap().parse().unwrap();
        task.abort();
        let _ = task.await;
        if mode == "hold" {
            marker(&directory.path().join("cancelled")).await;
            let notification: Value =
                serde_json::from_slice(&std::fs::read(directory.path().join("cancelled")).unwrap())
                    .unwrap();
            assert_eq!(notification["params"]["requestId"], id);
        }
        server.shutdown().await.unwrap();
        assert_eq!(directory.path().join("cancelled").exists(), mode == "hold");
    }
}

#[tokio::test]
async fn cancelled_partial_write_retires_without_appending_notification() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("block_read", directory.path());
    server.initialize().await.unwrap();
    marker(&directory.path().join("initialized")).await;
    server.transport.written_bytes.store(0, Ordering::Release);
    let transport = server.transport.clone();
    let task = tokio::spawn(async move {
        transport
            .request(
                "tools/call",
                json!({"padding":"x".repeat(MAX_FRAME_BYTES - 1024)}),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.transport.written_bytes.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(server.transport.written_bytes.load(Ordering::Acquire) < MAX_FRAME_BYTES - 1024);
    task.abort();
    let _ = task.await;
    assert!(server.transport.closed.load(Ordering::Acquire));
    assert!(
        server
            .transport
            .request("tools/call", json!({}))
            .await
            .is_err()
    );
    server.shutdown().await.unwrap();
    assert!(!directory.path().join("dispatched").exists());
    assert!(!directory.path().join("cancelled").exists());
}

#[tokio::test]
async fn interrupted_reply_to_peer_request_is_not_followed_by_cancellation_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("peer_ping", directory.path());
    server.initialize().await.unwrap();
    let transport = server.transport.clone();
    let task = tokio::spawn(async move { transport.request("tools/call", json!({})).await });
    marker(&directory.path().join("peer_ping")).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.transport.written_bytes.load(Ordering::Acquire) <= 1024 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(server.transport.outbound_partial.load(Ordering::Acquire));
    assert!(server.transport.written_bytes.load(Ordering::Acquire) < MAX_FRAME_BYTES - 100);
    task.abort();
    let _ = task.await;
    std::fs::write(directory.path().join("release"), "release").unwrap();
    marker(&directory.path().join("peer_input")).await;
    let bytes = std::fs::read(directory.path().join("peer_input")).unwrap();
    assert!(
        !bytes
            .windows(b"notifications/cancelled".len())
            .any(|part| part == b"notifications/cancelled")
    );
    assert!(
        !bytes.ends_with(b"\n"),
        "an incomplete reply must never have another frame appended"
    );
    server.shutdown().await.unwrap();
    assert!(server.transport.closed.load(Ordering::Acquire));
}

fn context(directory: &std::path::Path, maximum: usize) -> ToolContext {
    ToolContext {
        github: None,
        completion: None,
        policy: Arc::new(
            crate::policy::Policy::new(&crate::config::Config::default(), directory.to_owned())
                .unwrap(),
        ),
        approver: Arc::new(crate::tools::UnattendedApprover { allow: false }),
        timeout: Duration::from_millis(200),
        max_output_bytes: maximum,
        environment: BTreeMap::new(),
        cancellation: tokio_util::sync::CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: crate::tools::InteractionMode::Unattended,
        redactor: Arc::new(crate::tools::Redactor::default()),
    }
}

#[tokio::test]
async fn owner_failure_after_dispatch_fences_connection_and_retains_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("hold", directory.path());
    server.initialize().await.unwrap();
    server
        .transport
        .abort_after_dispatch
        .store(true, Ordering::Release);
    let result = server.transport.request("tools/call", json!({})).await;
    assert!(result.unwrap_err().to_string().contains("owner stopped"));
    assert!(server.transport.closed.load(Ordering::Acquire));
    assert!(
        server
            .transport
            .request("tools/call", json!({}))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !server.can_retire() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn tool_result_limit_is_exact_and_timeout_retires_only_its_server() {
    let directory = tempfile::tempdir().unwrap();
    let server = peer("echo", directory.path());
    server.initialize().await.unwrap();
    let tools = server.discover().await.unwrap();
    assert_eq!(
        tools[0]
            .execute(json!({}), &context(directory.path(), 2))
            .await
            .unwrap(),
        "ok"
    );
    assert!(
        tools[0]
            .execute(json!({}), &context(directory.path(), 1))
            .await
            .is_err()
    );

    let held_directory = tempfile::tempdir().unwrap();
    let held = peer("hold", held_directory.path());
    held.initialize().await.unwrap();
    let held_tools = held.discover().await.unwrap();
    let result = held_tools[0]
        .execute(json!({}), &context(held_directory.path(), 2))
        .await;
    assert!(matches!(result, Err(ToolError::Timeout(_))));
    marker(&held_directory.path().join("cancelled")).await;
    assert!(held.transport.closed.load(Ordering::Acquire));
    assert_eq!(
        tools[0]
            .execute(json!({}), &context(directory.path(), 2))
            .await
            .unwrap(),
        "ok"
    );
    held.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}
