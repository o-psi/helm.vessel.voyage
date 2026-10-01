//! Shared ordinary server fixtures. Every HTTP peer and file is owned here;
//! provider credentials, external services, native root and browser workers are
//! never needed. Held responses establish real activity before cancellation.
use super::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use voyage_protocol::process::{
    RuntimeCommand, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

#[derive(Clone)]
pub(super) struct Reply {
    pub status: u16,
    pub body: String,
    pub held: bool,
}
impl Reply {
    pub fn text(text: &str) -> Self {
        Self {
            status: 200,
            held: false,
            body: format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":"stop"}]})
            ),
        }
    }
    pub fn held(text: &str) -> Self {
        Self {
            held: true,
            ..Self::text(text)
        }
    }
    pub fn error(status: u16) -> Self {
        Self {
            status,
            held: false,
            body: json!({"error":{"message":"owned synthetic refusal","type":"fixture"}})
                .to_string(),
        }
    }
}
pub(super) struct Provider {
    pub url: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
    pub disconnected: Arc<AtomicUsize>,
    pub release: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Provider {
    pub async fn new(replies: Vec<Reply>) -> Self {
        Self::with_models(
            replies,
            Reply {
                status: 200,
                held: false,
                body: json!({"data":[{"id":"fixture","input_modalities":["text","image"]}]})
                    .to_string(),
            },
        )
        .await
    }
    pub async fn with_models(replies: Vec<Reply>, models: Reply) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let disconnected = Arc::new(AtomicUsize::new(0));
        let closed = disconnected.clone();
        let release = CancellationToken::new();
        let released = release.clone();
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    Some(result) = clients.join_next(), if !clients.is_empty() => { result.unwrap(); },
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let captured = captured.clone(); let replies = replies.clone();
                        let released = released.clone(); let closed = closed.clone(); let model_reply=models.clone();
                        clients.spawn(async move {
                            let read = async {
                                let mut bytes = Vec::new();
                                let end = loop {
                                    let mut block = [0;4096]; let n = socket.read(&mut block).await.unwrap();
                                    if n == 0 { return None; }
                                    bytes.extend_from_slice(&block[..n]); assert!(bytes.len() <= 2*1024*1024);
                                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break end+4; }
                                };
                                let headers = String::from_utf8_lossy(&bytes[..end]);
                                let models = headers.starts_with("GET /v1/models ");
                                if !models { assert!(headers.starts_with("POST /v1/chat/completions ")); }
                                let length = headers.lines().find_map(|line| { let (name,value)=line.split_once(':')?; name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap()) }).unwrap_or(0);
                                assert!(length <= 1024*1024);
                                while bytes.len() < end+length { let mut block=[0;4096]; let n=socket.read(&mut block).await.unwrap(); if n==0 {return None;} bytes.extend_from_slice(&block[..n]); }
                                if models { Some(model_reply) }
                                else { captured.lock().await.push(serde_json::from_slice(&bytes[end..end+length]).unwrap()); Some(replies.lock().await.pop_front().unwrap_or_else(|| Reply::text("Owned fixture answer"))) }
                            };
                            let Some(reply) = tokio::time::timeout(Duration::from_secs(10), read).await.unwrap() else { return; };
                            if reply.held {
                                let mut byte = [0];
                                tokio::select! {
                                    _ = released.cancelled() => {},
                                    read = socket.read(&mut byte) => { assert_eq!(read.unwrap(),0); closed.fetch_add(1,Ordering::SeqCst); return; }
                                }
                            }
                            let content = if reply.body.starts_with("data:") {"text/event-stream"} else {"application/json"};
                            let response=format!("HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",reply.status,content,reply.body.len(),reply.body);
                            let _ = socket.write_all(response.as_bytes()).await; let _ = socket.shutdown().await;
                        });
                    }
                }
            }
        });
        Self {
            url,
            requests,
            disconnected,
            release,
            task,
        }
    }
    pub async fn wait_requests(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.requests.lock().await.len() < count {
                assert!(!self.task.is_finished());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    pub async fn wait_disconnected(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.disconnected.load(Ordering::SeqCst) == 0 {
                assert!(!self.task.is_finished());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    pub fn config(&self) -> Config {
        Config {
            provider: crate::config::ProviderKind::OpenaiChat,
            model: "fixture".into(),
            base_url: Some(self.url.clone()),
            api_key_required: false,
            api_key_env: format!("VOYAGE_OWNED_UNUSED_{}", Uuid::new_v4().simple()),
            access: Some(crate::config::AccessMode::ReadOnly),
            provider_retry_attempts: 1,
            provider_response_timeout_ms: 10_000,
            provider_stream_idle_ms: 10_000,
            ..Default::default()
        }
    }
}
pub(super) fn expiry() -> u64 {
    (chrono::Utc::now().timestamp_millis() + 120_000) as u64
}
pub(super) fn auth(state: &State) -> authorization::Authorization {
    authorization::Authorization {
        scope_source: None,
        browser_history: false,
        owner_connection: false,
        authority: None,
        actor: state.actor,
        grant: None,
    }
}
pub(super) async fn call(state: &Arc<State>, command: RuntimeCommand) -> Result<Value> {
    dispatch::dispatch(state, command, auth(state)).await
}
pub(super) async fn configured(replies: Vec<Reply>) -> (tempfile::TempDir, Arc<State>, Provider) {
    let (root, state) = super::tests::fixture().await;
    let provider = Provider::new(replies).await;
    *state.config.write().await = provider.config();
    (root, state, provider)
}
pub(super) async fn fixture() -> (tempfile::TempDir, Arc<State>, Provider) {
    let (root, state, provider) = configured(vec![]).await;
    state.config.write().await.access = Some(crate::config::AccessMode::Unrestricted);
    (root, state, provider)
}
pub(super) fn submit(revision: u64, prompt: &str) -> RuntimeCommand {
    RuntimeCommand::Submit {
        budget: None,
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        prompt: prompt.into(),
    }
}
pub(super) async fn finish(state: &Arc<State>) -> Value {
    tokio::time::timeout(Duration::from_secs(30),async {
        loop {
            state.cleanup.advance(1).await.unwrap();
            let snapshot=match state.owner.process_snapshot().await {
                Ok(snapshot)=>snapshot,
                Err(error) if matches!(error.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_)) if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked)) => {
                    tokio::time::sleep(Duration::from_millis(10)).await; continue;
                }
                Err(error)=>panic!("owned terminal observation failed: {error:#}"),
            };
            if state.active.lock().await.is_none() && !snapshot["run"].is_null()
                && !matches!(snapshot["run"]["state"].as_str(),Some("accepted"|"running")) && snapshot["pending_cleanup_run"].is_null() {
                state.controls.shutdown_retained(&state.owner).await.unwrap(); return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap()
}
pub(super) async fn socket_call(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    command: RuntimeCommand,
) -> RuntimeResponse {
    let mut stream = tokio::net::UnixStream::connect(directory.join("runtime.sock"))
        .await
        .unwrap();
    write_frame(
        &mut stream,
        &RuntimeRequest {
            protocol: 1,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            token: registration.token.clone(),
            authorization: None,
            scope_authority: None,
            command,
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream))
        .await
        .unwrap()
        .unwrap()
}
