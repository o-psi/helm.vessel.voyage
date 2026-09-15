use super::*;
#[test]
fn names_and_json_rpc_envelopes_are_bounded_and_validated() {
    assert_eq!(sanitize(""), "unnamed");
    assert_eq!(sanitize("Hello-世界!"), "hello____");
    assert_eq!(tool_name("My Server", "READ"), "mcp_my_server_read");
    let a = tool_name(&"a".repeat(100), "one");
    let b = tool_name(&"a".repeat(100), "two");
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
    for value in [
        json!({"jsonrpc":"2.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","method":"ping"}),
        json!({"jsonrpc":"2.0","id":"x","error":{"code":-1,"message":"error"}}),
    ] {
        validate_envelope(&value).unwrap();
    }
    for value in [
        json!(null),
        json!({"jsonrpc":"1.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":true,"result":{}}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{}}),
        json!({"jsonrpc":"2.0","id":1,"error":{"code":"bad","message":"error"}}),
    ] {
        assert!(validate_envelope(&value).is_err());
    }
    let value = json!({"hello":"world"});
    let frame = encode_frame(&value).unwrap();
    assert_eq!(frame.last(), Some(&b'\n'));
    assert_eq!(serde_json::from_slice::<Value>(&frame).unwrap(), value);
    assert!(encode_frame(&json!("x".repeat(MAX_FRAME_BYTES))).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_handshake_paginated_discovery_execution_and_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let mut ctx = crate::tools::reliability_tests::context(root.path());
    ctx.artifact_scope = Some(crate::artifacts::Scope {
        directory: root.path().join("artifacts"),
        session: uuid::Uuid::new_v4(),
    });
    let script = r#"
import sys,json
for line in sys.stdin:
 r=json.loads(line); m=r.get('method'); i=r.get('id')
 if i is None: continue
 if m=='initialize': result={'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'fixture','version':'1'}}
 elif m=='tools/list':
  if r.get('params',{}).get('cursor')=='next': result={'tools':[{'name':'fail','description':'fixture failure','inputSchema':{'type':'object'}}]}
  else: result={'tools':[{'name':'echo','description':'fixture echo','inputSchema':{'type':'object'},'annotations':{'readOnlyHint':True}}],'nextCursor':'next'}
 elif m=='tools/call':
  fail=r['params']['name']=='fail'; result={'content':[{'type':'text','text':'fixture failure' if fail else 'hello '+r['params']['arguments']['text']}],'isError':fail}
 else: result={}
 print(json.dumps({'jsonrpc':'2.0','id':i,'result':result}),flush=True)
"#;
    let server = McpServer::start_scoped(
        "Fixture",
        "/usr/bin/python3",
        &["-u".into(), "-c".into(), script.into()],
        &BTreeMap::new(),
        &ctx.policy,
    )
    .unwrap();
    server.initialize().await.unwrap();
    let tools = server.discover().await.unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].definition().name, "mcp_fixture_echo");
    assert_eq!(
        tools[0]
            .execute(json!({"text":"world"}), &ctx)
            .await
            .unwrap(),
        "hello world"
    );
    assert!(
        tools[1]
            .execute(json!({}), &ctx)
            .await
            .unwrap_err()
            .to_string()
            .contains("fixture failure")
    );
    let mut cancelled = ctx.clone();
    cancelled.cancellation = Default::default();
    cancelled.cancellation.cancel();
    assert!(matches!(
        tools[0].execute(json!({}), &cancelled).await,
        Err(ToolError::Cancelled)
    ));
    server.shutdown().await.unwrap();
    assert!(server.observed());
    assert!(server.can_retire());
    server.shutdown().await.unwrap();
    assert!(tools[0].execute(json!({}), &ctx).await.is_err());
}
