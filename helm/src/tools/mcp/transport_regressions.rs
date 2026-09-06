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
        continue
    identifier = request['id']
    if method == 'initialize':
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
        elif mode == 'flood':
            for i in range(129):
                reply({'jsonrpc':'2.0','method':'notifications/progress','params':{}})
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[]}})
        elif mode == 'hold':
            pass
        elif mode == 'partial':
            sys.stdout.write('{"jsonrpc":"2.0",'); sys.stdout.flush()
            mark('partial')
        else:
            reply({'jsonrpc':'2.0','id':identifier,'result':{'content':[{'type':'text','text':'ok'}]}})
"#;

fn peer(mode: &str, directory: &std::path::Path) -> Arc<McpServer> {
    Arc::new(McpServer::start("fixture", "python3", &[
        "-u".into(), "-c".into(), PEER.into(), mode.into(),
        directory.to_string_lossy().into_owned(),
    ], &BTreeMap::new()).unwrap())
}

async fn marker(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("fixture did not reach observed barrier");
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
    let result = tokio::time::timeout(Duration::from_secs(2), server.transport.request("tools/call", json!({}))).await;
    server.shutdown().await.unwrap();
    assert!(matches!(result, Ok(Err(_))), "unbounded frame waited for newline: {result:?}");
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
        marker(&directory.path().join(if mode == "partial" { "partial" } else { "dispatched" })).await;
        task.abort();
        let _ = task.await;
        let result = tokio::time::timeout(Duration::from_millis(500), server.transport.request("tools/call", json!({}))).await;
        server.shutdown().await.unwrap();
        assert!(matches!(result, Ok(Err(_))), "abandoned call transport remained usable or hung: {result:?}");
    }
}
