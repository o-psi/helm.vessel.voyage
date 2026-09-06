//! Explicit synthetic native driver; default return is not behavioral evidence.
use super::*;
use crate::tools::{Approver, ApprovalRequest, ApprovalOutcome, Tool, ToolContext, ProcessTool};
use serde_json::json;
struct No;
#[async_trait::async_trait]
impl Approver for No {
    async fn approve(&self,_:&ApprovalRequest)->ApprovalOutcome {ApprovalOutcome::Unavailable}
}
#[test]
fn native_driver() {
    let Ok(mode)=std::env::var("HELM_PLAIN_TERMINAL_DRIVER") else{return};
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let directory=tempfile::tempdir().unwrap();
        let config=crate::Config{approval:crate::config::ApprovalMode::Never,..Default::default()};
        let policy=Arc::new(Policy::new(&config,directory.path().into()).unwrap());
        let context=ToolContext{github:None,completion:None,policy:policy.clone(),approver:Arc::new(No),timeout:Duration::from_secs(3),max_output_bytes:4096,environment:std::collections::BTreeMap::from([("PATH".into(),"/usr/bin:/bin".into())]),cancellation:CancellationToken::new(),execution_id:uuid::Uuid::new_v4(),interaction:crate::tools::InteractionMode::Attended,redactor:Arc::new(crate::tools::Redactor::default())};
        let manager=Arc::new(ProcessTool::default());
        let started=manager.execute(json!({"action":"start","name":"fixture-shell","command":"stty -echo; printf INNER_READY; exec /bin/sh"}),&context).await.unwrap();
        let id=TerminalId(uuid::Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap());
        let mut stop=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).unwrap();
        let mut pending=PendingInput::default();
        println!("DRIVER_PROMPT");
        if mode!="non-tty" {
            assert_eq!(pending_prompt(&mut pending,CancellationToken::new()).await.unwrap().as_deref(),Some("/terminal"));
        }
        let cancel=CancellationToken::new();
        let operation=attach(manager.clone(),id,policy,cancel.clone());tokio::pin!(operation);
        let result=tokio::select!{ biased;
            _=stop.recv()=>{cancel.cancel();operation.await},
            result=&mut operation=>result,
        };
        match result {
            Ok(detached)=>{
                assert!(!owns_terminal());
                let capture=manager.execute(json!({"action":"read","id":id.0}),&context).await.unwrap();
                assert!(!capture.contains("PRIVATE_CANARY"));
                assert!(capture.contains("private"),"{capture}");
                if !detached.exited {
                    assert_eq!(manager.snapshot(id).await.unwrap().state,TerminalState::Running);
                    pending.append(&detached.pending).unwrap();
                    println!("DRIVER_RESUMED");
                    let line=pending_prompt(&mut pending,CancellationToken::new()).await.unwrap().unwrap();
                    assert_eq!(line,"next🧭");
                    println!("DRIVER_HANDOFF_OK");
                } else {println!("DRIVER_INNER_EXIT");}
            }
            Err(error)=>{assert!(!owns_terminal());println!("DRIVER_ERROR={error}");}
        }
        let cleanup=manager.shutdown(Duration::from_secs(3)).await;
        assert!(cleanup.observation_complete,"{cleanup:?}");
        println!("DRIVER_CLEANUP_OK");
    });
}
#[test]
fn native_pty_contract() {
    let status=std::process::Command::new("python3").arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/system/plain_terminal.py")).arg("--test-binary").arg(std::env::current_exe().unwrap()).status().unwrap();
    assert!(status.success(),"plain terminal native fixture failed");
}
