use super::*;
use helm::terminal::{
    InteractiveTerminals, TerminalError, TerminalEvent, TerminalId, TerminalSnapshot,
    TerminalSummary,
};
struct Held {
    ready: tokio::sync::Notify,
    events: tokio::sync::broadcast::Sender<TerminalEvent>,
    panic: bool,
}
#[async_trait]
impl InteractiveTerminals for Held {
    async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError> {
        Ok(vec![])
    }
    async fn attach(&self, _: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        self.ready.notify_one();
        assert!(!self.panic, "synthetic attachment panic");
        std::future::pending().await
    }
    async fn snapshot(&self, _: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        std::future::pending().await
    }
    async fn write(&self, _: TerminalId, _: Vec<u8>) -> Result<(), TerminalError> {
        panic!("no key may be forwarded")
    }
    async fn resize(&self, _: TerminalId, _: u16, _: u16) -> Result<(), TerminalError> {
        Ok(())
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TerminalEvent> {
        self.events.subscribe()
    }
}
#[test]
fn frontend_driver() {
    let Ok(mode) = std::env::var("HELM_PLAIN_FRONTEND_DRIVER") else {
        return;
    };
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        use futures_util::FutureExt;
        let directory=tempfile::tempdir().unwrap();
        let config=Config{approval:helm::config::ApprovalMode::Never,..Default::default()};
        let policy=Arc::new(helm::policy::Policy::new(&config,directory.path().into()).unwrap());
        let held=Arc::new(Held{ready:tokio::sync::Notify::new(),events:tokio::sync::broadcast::channel(1).0,panic:mode=="panic"});
        let operation=std::panic::AssertUnwindSafe(helm::plain_terminal::attach(held.clone(),TerminalId(uuid::Uuid::new_v4()),policy,tokio_util::sync::CancellationToken::new())).catch_unwind();
        if mode=="panic" {assert!(operation.await.is_err());}
        else {
            tokio::pin!(operation);
            tokio::select! { biased;
                result=&mut operation=>panic!("unexpected attachment result: {}",result.is_ok()),
                _=held.ready.notified()=>{
                    assert!(helm::plain_terminal::owns_terminal());
                    let terminal=Terminal::default();
                    let request=ApprovalRequest{id:uuid::Uuid::new_v4(),execution_id:uuid::Uuid::new_v4(),action:"shell".into(),target:"must-not-approve".into(),reason:"private keys cannot approve".into(),mode:helm::tools::InteractionMode::Attended};
                    assert_eq!(tokio::time::timeout(std::time::Duration::from_millis(100),terminal.approve(&request)).await.unwrap(),ApprovalOutcome::Unavailable);
                    assert_eq!(terminal.ask_question(&helm::tools::Question{question:"Select?".into(),options:vec!["A".into(),"B".into()]}).await,helm::tools::QuestionAnswer::Unavailable);
                    terminal.emit(AgentEvent::AssistantText("SUPPRESSED_FRONTEND_CANARY".into())).await;
                }
            }
        }
        assert!(!helm::plain_terminal::owns_terminal());
        println!("FRONTEND_OWNERSHIP_OK");
    });
}
#[test]
fn frontend_pty_contract() {
    let status = std::process::Command::new("python3")
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/system/plain_terminal_frontend.py"),
        )
        .arg(std::env::current_exe().unwrap())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "actual plain frontend ownership fixture failed"
    );
}
