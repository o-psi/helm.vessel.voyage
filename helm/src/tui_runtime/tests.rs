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
        let waiting = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (server_waiting, server_release) = (waiting.clone(), release.clone());
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
                if step == 2 {
                    server_waiting.notify_one();
                    server_release.notified().await;
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
        let parent_session = Session::new(first.path().into(), config.model.clone());
        let inactive = WorkspaceRuntime::build(&config, &parent_session).await.unwrap();
        let active = WorkspaceRuntime::build(&config, &Session::new(second.path().into(), config.model.clone())).await.unwrap();
        let parent_run = inactive.agent.prepare_run(&parent_session).await.unwrap().unwrap();
        let budget = AgentBudget { max_tokens: 4096, max_terminals: 1 };
        let id = inactive.subagents.spawn_for_run(helm::subagent::SpawnRequest {
            parent_id: None, name: "inactive-child".into(), task: "ask then request shell approval".into(),
            policy: AgentPolicy {
                readable_roots: vec![first.path().into()], writable_roots: vec![first.path().into()],
                allowed_tools: ["questions".into(), "shell".into()].into_iter().collect(),
                approval: ApprovalPolicy::Deny, budget: budget.clone(),
            }, budget, worktree: None, branch: None,
        }, Some(parent_run)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), waiting.notified()).await.unwrap();
        assert!(inactive.check_idle().await.is_err(), "live child must refuse policy handoff");
        active.check_idle().await.unwrap();
        release.notify_one();
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

#[test]
fn policy_handoff_revalidates_before_ownership_and_preserves_other_workspaces() {
    const CHILD: &str = "HELM_POLICY_HANDOFF_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tui_runtime::tests::policy_handoff_revalidates_before_ownership_and_preserves_other_workspaces", "--nocapture"])
            .env(CHILD, "1").env("HOME", directory.path())
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .env("POLICY_HANDOFF_FIXTURE_KEY", "offline-fixture").spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "policy fixture failed: {status}");
                return;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("policy fixture timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let root = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            let session = Session::new(root.path().into(), "fixture".into());
            let other_session = Session::new(other.path().into(), "fixture".into());
            let config = Config {
                provider: helm::config::ProviderKind::OpenaiChat,
                base_url: Some("http://127.0.0.1:1/v1".into()),
                api_key_env: "POLICY_HANDOFF_FIXTURE_KEY".into(),
                access: Some(AccessMode::Unrestricted),
                provider_retry_attempts: 1,
                ..Config::default()
            };
            let closed = Arc::new(ManagedResources::local());
            closed.close().unwrap();
            assert!(
                WorkspaceRuntime::build_with_resources(&config, &session, None, closed)
                    .await
                    .is_err()
            );
            // Failed post-build registration released its persistent writer; no stale
            // registry, bridge or native executor holds a hidden runtime lease.
            let runtime = WorkspaceRuntime::build(&config, &session).await.unwrap();
            let other_runtime = WorkspaceRuntime::build(&config, &other_session)
                .await
                .unwrap();
            runtime.check_idle().await.unwrap();
            let pending = runtime.agent.clone();
            assert!(runtime.check_idle().await.is_err());
            drop(pending);
            let observer = runtime.supervisor.clone();
            assert!(runtime.check_idle().await.is_err());
            drop(observer);
            let directory = root.path().join("profiles");
            let profiles = runtime.policy.profiles(&directory).unwrap();
            let target = runtime
                .policy
                .target(
                    directory,
                    profiles
                        .iter()
                        .find(|profile| profile.name == "restricted")
                        .unwrap(),
                )
                .unwrap();
            let preview = runtime.policy.preview(&target).unwrap();
            assert!(!preview.requires_confirmation);
            let request = helm::policy_profile::switching::SwitchRequest {
                target,
                preview_digest: preview.digest,
                confirmation: None,
            };
            let candidate = runtime.policy.prepare(&request).unwrap();
            let next_digest = preview.proposed.digest().to_owned();
            assert!(
                WorkspaceRuntime::build_checked(&candidate, &session, Some("wrong digest"))
                    .await
                    .is_err()
            );
            runtime.check_idle().await.unwrap();
            runtime.stop_observed().await.unwrap();
            assert!(
                runtime
                    .agent
                    .run(vec![], "must never request provider".into())
                    .await
                    .is_err()
            );
            drop(runtime);
            let selected =
                WorkspaceRuntime::build_checked(&candidate, &session, Some(&next_digest))
                    .await
                    .unwrap();
            assert_eq!(selected.access, AccessMode::ReadOnly);
            assert_eq!(other_runtime.access, AccessMode::Unrestricted);
            assert!(other_runtime.config.policy_profile.is_none());
            other_runtime.check_idle().await.unwrap();
            assert_eq!(config.access_mode(), AccessMode::Unrestricted);
            selected.stop_observed().await.unwrap();
            drop(selected);
            // Selection was runtime-only: ordinary restart uses fresh launch policy.
            let reopened = WorkspaceRuntime::build(&config, &session).await.unwrap();
            assert_eq!(reopened.access, AccessMode::Unrestricted);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _guard = reopened.resources.0.lock().unwrap();
                panic!("injected resource observation failure");
            }));
            assert!(reopened.stop_observed().await.is_err());
            assert!(
                reopened
                    .agent
                    .run(vec![], "must remain blocked".into())
                    .await
                    .is_err()
            );
            assert!(WorkspaceRuntime::build(&config, &session).await.is_err());
            let failure = WorkspaceRuntime::construction_error(
                anyhow::anyhow!("injected build failure"),
                &reopened.subagents,
                &reopened.resources,
                true,
            )
            .await;
            assert!(
                failure
                    .downcast_ref::<UnconfirmedRuntimeCleanup>()
                    .is_some()
            );
            drop(reopened);
            let recovered = WorkspaceRuntime::build(&config, &session).await.unwrap();
            recovered.stop_observed().await.unwrap();
            other_runtime.stop_observed().await.unwrap();
        });
}
