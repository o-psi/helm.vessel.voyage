use super::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Isolate persistent stores and credentials from other binary tests; the outer
// watchdog also catches accidental blocking waits on an inactive UI receiver.
#[test]
fn inactive_workspace_children_do_not_wait_for_root_questions_or_approvals() {
    const CHILD: &str = "HELM_INACTIVE_WORKSPACE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tui_runtime::tests::inactive_workspace_children_do_not_wait_for_root_questions_or_approvals", "--nocapture"])
            .env(CHILD, "1")
            .env("HOME", directory.path())
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("INACTIVE_FIXTURE_KEY", "offline-fixture")
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "inactive workspace child failed: {status}"
                );
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("inactive workspace child waited on an unavailable frontend");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            provider: helm::config::ProviderKind::OpenaiChat,
            base_url: Some(format!("http://{}/v1", listener.local_addr().unwrap())),
            api_key_env: "INACTIVE_FIXTURE_KEY".into(),
            access: Some(AccessMode::Approval),
            provider_retry_attempts: 1,
            ..Config::default()
        };
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let (header_end, length) = loop {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = headers.lines().find_map(|line|line.strip_prefix("content-length: ")).unwrap().parse::<usize>().unwrap();
                        break (end + 4, length);
                    }
                };
                while request.len() < header_end + length {
                    let mut buffer = [0; 4096];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                let body: Value = serde_json::from_slice(&request[header_end..header_end+length]).unwrap();
                if step > 0 {
                    let result = body["messages"].as_array().unwrap().iter().rev().find(|message|message["role"]=="tool").unwrap()["content"].as_str().unwrap();
                    if step == 1 { assert!(result.contains("unavailable"), "{result}"); }
                    else { assert!(result.contains("denied") || result.contains("approval"), "{result}"); }
                }
                let delta = match step {
                    0 => json!({"tool_calls":[{"index":0,"id":"question","type":"function","function":{"name":"questions","arguments":json!({"question":"Continue?","options":["One","Two"]}).to_string()}}]}),
                    1 => json!({"tool_calls":[{"index":0,"id":"shell","type":"function","function":{"name":"shell","arguments":json!({"command":"printf forbidden > child-effect"}).to_string()}}]}),
                    _ => json!({"content":"child completed without a frontend"}),
                };
                let body = format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"delta":delta}]}));
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let inactive = WorkspaceRuntime::build(&config, &Session::new(first.path().into(), config.model.clone())).await.unwrap();
        let active = WorkspaceRuntime::build(&config, &Session::new(second.path().into(), config.model.clone())).await.unwrap();
        let budget = AgentBudget { max_tokens: 4096, max_terminals: 1 };
        let id = inactive.subagents.spawn(helm::subagent::SpawnRequest {
            parent_id: None, name: "inactive-child".into(), task: "ask then request shell approval".into(),
            policy: AgentPolicy {
                readable_roots: vec![first.path().into()], writable_roots: vec![first.path().into()],
                allowed_tools: ["questions".into(), "shell".into()].into_iter().collect(),
                approval: ApprovalPolicy::Deny, budget: budget.clone(),
            }, budget, worktree: None, branch: None,
        }).await.unwrap();
        // Both cached root receivers remain unpolled. The production child
        // executor must resolve its own unattended question/approval boundary.
        let result = tokio::time::timeout(Duration::from_secs(5), inactive.subagents.wait(id)).await.unwrap().unwrap().unwrap();
        assert!(result.summary.contains("child completed without a frontend"));
        assert!(!first.path().join("child-effect").exists());
        server.await.unwrap();
        inactive.subagents.shutdown().await;
        active.subagents.shutdown().await;
    });
}
