//! Explicit synthetic native driver; default return is not behavioral evidence.
use super::*;
use crate::tools::{Approver, ApprovalRequest, ApprovalOutcome, Tool, ToolContext, ProcessTool};
use serde_json::json;
struct No;
#[async_trait::async_trait]
impl Approver for No {
    async fn approve(&self,_:&ApprovalRequest)->ApprovalOutcome {ApprovalOutcome::Unavailable}
}
struct FailingWrite(Arc<ProcessTool>);
#[async_trait::async_trait]
impl InteractiveTerminals for FailingWrite {
    async fn list(&self)->std::result::Result<Vec<TerminalSummary>,crate::terminal::TerminalError>{self.0.list().await}
    async fn attach(&self,id:TerminalId)->std::result::Result<TerminalSnapshot,crate::terminal::TerminalError>{self.0.attach(id).await}
    async fn snapshot(&self,id:TerminalId)->std::result::Result<TerminalSnapshot,crate::terminal::TerminalError>{self.0.snapshot(id).await}
    async fn resize(&self,id:TerminalId,columns:u16,rows:u16)->std::result::Result<(),crate::terminal::TerminalError>{self.0.resize(id,columns,rows).await}
    async fn write(&self,_:TerminalId,_:Vec<u8>)->std::result::Result<(),crate::terminal::TerminalError>{Err(crate::terminal::TerminalError::Closed)}
    fn subscribe(&self)->tokio::sync::broadcast::Receiver<crate::terminal::TerminalEvent>{self.0.subscribe()}
}
#[test]
fn native_driver() {
    let Ok(mode)=std::env::var("HELM_PLAIN_TERMINAL_DRIVER") else{return};
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let directory=tempfile::tempdir().unwrap();
        use crate::policy_profile::{Builtin,Overrides,selection::{Selection,SelectionRequest},store::{Action,ProfileChange,ProfileStore}};
        let mut config=crate::Config{approval:crate::config::ApprovalMode::Never,..Default::default()};
        let profiles=if mode=="policy" {
            let path=directory.path().join("profiles");let store=ProfileStore::open(&path).unwrap();
            let snapshot=store.change(&ProfileChange{operation_id:uuid::Uuid::new_v4(),name:"plain-fixture".into(),expected_revision:0,action:Action::Create{rules:Builtin::Autonomous.document().rules}}).unwrap().snapshot;
            let request=SelectionRequest{directory:path,name:snapshot.name.clone(),revision:snapshot.revision,digest:snapshot.digest().unwrap(),explicit:Overrides::default()};
            let preview=Selection::preview(&config,directory.path(),&request).unwrap();
            config.policy_profile=Some(Selection::bind(&config,directory.path(),request,Some(&preview.transition_digest)).unwrap());
            Some((store,snapshot.revision))
        }else{None};
        let policy=Arc::new(Policy::new(&config,directory.path().into()).unwrap());
        let context=ToolContext{github:None,completion:None,policy:policy.clone(),approver:Arc::new(No),timeout:Duration::from_secs(3),max_output_bytes:4096,environment:std::collections::BTreeMap::from([("PATH".into(),"/usr/bin:/bin".into())]),cancellation:CancellationToken::new(),execution_id:uuid::Uuid::new_v4(),interaction:crate::tools::InteractionMode::Attended,redactor:Arc::new(crate::tools::Redactor::default())};
        let manager=Arc::new(ProcessTool::default());
        let started=manager.execute(json!({"action":"start","name":"fixture-shell","command":"stty -echo; printf INNER_READY; exec /bin/sh"}),&context).await.unwrap();
        let id=TerminalId(uuid::Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap());
        let mut policy_signal=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2()).unwrap();
        let mut stop=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()).unwrap();
        let mut pending=PendingInput::default();
        println!("DRIVER_PROMPT");
        if mode!="non-tty" {
            assert_eq!(pending_prompt(&mut pending,CancellationToken::new()).await.unwrap().as_deref(),Some("/terminal"));
        }
        let cancel=CancellationToken::new();
        let frontend:Arc<dyn InteractiveTerminals>=if mode.starts_with("write-failure"){Arc::new(FailingWrite(manager.clone()))}else{manager.clone()};
        let attachment_policy=if mode=="read-only" {Arc::new(Policy::new(&crate::Config{access:Some(crate::config::AccessMode::ReadOnly),..Default::default()},directory.path().into()).unwrap())}else{policy};
        let operation=attach(frontend,id,attachment_policy,cancel.clone());tokio::pin!(operation);
        let result=tokio::select!{ biased;
            _=stop.recv()=>{cancel.cancel();operation.await},
            _=policy_signal.recv()=>{
                let (store,revision)=profiles.as_ref().unwrap();
                store.change(&ProfileChange{operation_id:uuid::Uuid::new_v4(),name:"plain-fixture".into(),expected_revision:*revision,action:Action::Delete{}}).unwrap();
                operation.await
            },
            result=&mut operation=>result,
        };
        match result {
            Ok(detached)=>{
                assert!(!owns_terminal());
                assert_eq!(detached.delivery_failed,mode=="write-failure-detach");
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
            Err(error)=>{
                assert!(!owns_terminal());
                if mode=="write-failure-private" {
                    let mut input=crate::terminal_input::Terminal::enter_preserving_input().unwrap();
                    assert!(input.read().unwrap().is_none(),"unread private bytes escaped into the next prompt");input.restore().unwrap();
                }
                println!("DRIVER_ERROR={error}");
            }
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
