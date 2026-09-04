//! ChatGPT-subscription transport through the supported `codex app-server` stdio API.
//!
//! This adapter opts into the generated experimental dynamic-tools surface. Codex is
//! used only as the model transport: dynamic tool requests are returned to Helm's
//! normal agent loop, so Helm remains the authority for policy, approval and execution.
use super::{
    ModelInfo, Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent,
    normalize_models,
};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, mpsc},
};

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const CHANNEL_CAPACITY: usize = 256;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CodexSubscriptionProvider {
    state: Arc<Mutex<State>>,
    program: String,
    args: Vec<String>,
    workspace: PathBuf,
    cancelled: Arc<AtomicBool>,
}

struct State {
    client: Option<Arc<AppClient>>,
    initialized: bool,
    thread_id: Option<String>,
    turn_id: Option<String>,
    pending_tool: Option<PendingTool>,
    active: bool,
}
struct PendingTool {
    request_id: Value,
    call_id: String,
}

struct AppClient {
    stdin: Mutex<ChildStdin>,
    inbox: Mutex<Inbox>,
    child: StdMutex<Child>,
    next_id: AtomicU64,
}
struct Inbox {
    receiver: mpsc::Receiver<Result<Value, String>>,
    backlog: VecDeque<Value>,
}

impl CodexSubscriptionProvider {
    pub fn new(program: String, workspace: PathBuf) -> Self {
        Self::with_command(
            program,
            vec!["app-server".into(), "--stdio".into()],
            workspace,
        )
    }
    fn with_command(program: String, args: Vec<String>, workspace: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                client: None,
                initialized: false,
                thread_id: None,
                turn_id: None,
                pending_tool: None,
                active: false,
            })),
            program,
            args,
            workspace,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl Provider for CodexSubscriptionProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut state = self.state.lock().await;
        if state.active {
            return Err(ProviderError::Unavailable(
                "model discovery is unavailable during an active turn".into(),
            ));
        }
        if state.client.is_none() {
            let client = AppClient::spawn(&self.program, &self.args).await?;
            client.initialize().await?;
            state.initialized = true;
            state.client = Some(client);
        }
        let client = state.client.as_ref().expect("client initialized").clone();
        let mut cursor: Option<String> = None;
        let mut models = Vec::new();
        loop {
            let response = client
                .request(
                    "model/list",
                    json!({"cursor":cursor,"limit":100,"includeHidden":false}),
                )
                .await?;
            let result = response.get("result").ok_or_else(|| {
                ProviderError::InvalidResponse("model/list omitted result".into())
            })?;
            let data = result
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("model/list omitted result.data".into())
                })?;
            for value in data {
                if value
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    continue;
                }
                let id = value
                    .get("model")
                    .or_else(|| value.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ProviderError::InvalidResponse("model/list entry omitted model".into())
                    })?;
                let reasoning_efforts = value
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.get("reasoningEffort").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect();
                let input_modalities = value
                    .get("inputModalities")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                models.push(ModelInfo {
                    id: id.to_owned(),
                    display_name: value
                        .get("displayName")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_owned(),
                    description: value
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    is_default: value
                        .get("isDefault")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    reasoning_efforts,
                    input_modalities,
                });
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        normalize_models(&mut models);
        Ok(models)
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        use futures_util::StreamExt;
        let mut stream = self.stream(request).await?;
        while let Some(event) = stream.next().await {
            if let ProviderStreamEvent::Completed(response) = event? {
                return Ok(response);
            }
        }
        Err(ProviderError::InvalidResponse(
            "codex app-server stream ended without completion".into(),
        ))
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let state = self.state.clone();
        let program = self.program.clone();
        let args = self.args.clone();
        let workspace = self.workspace.clone();
        let cancelled = self.cancelled.clone();
        Ok(Box::pin(async_stream::try_stream! {
            let mut state=state.lock().await;
            if cancelled.swap(false,Ordering::SeqCst) { state.client=None;state.initialized=false;state.thread_id=None;state.active=false;state.pending_tool=None;state.turn_id=None; }
            if state.client.is_none() { state.client=Some(AppClient::spawn(&program,&args).await?); }
            let client=state.client.as_ref().expect("client initialized").clone();
            let mut startup_guard=StartupGuard { client:client.clone(),cancelled:cancelled.clone(),armed:true };
            if state.thread_id.is_none() {
                if !state.initialized { client.initialize().await?; state.initialized=true; }
                let tools=request.tools.iter().map(|tool|json!({"type":"function","name":tool.name,"description":tool.description,"inputSchema":tool.input_schema,"deferLoading":false})).collect::<Vec<_>>();
                let system=request.messages.iter().filter(|m|m.role==Role::System).map(|m|m.content.as_str()).collect::<Vec<_>>().join("\n\n");
                let tool_names=request.tools.iter().map(|tool|tool.name.as_str()).collect::<Vec<_>>().join(", ");
                let developer_instructions=format!("You are the model inside Helm, not the Codex host around it. Never use or claim built-in command, filesystem, web, MCP, app, plugin, skill, collaboration, image, document, or patch capabilities. Use only client-provided dynamic tools; Helm enforces all policy and executes every action. The exact available call names are: {tool_names}. If asked about tools, answer from that list only.");
                let result=client.request("thread/start",json!({"cwd":workspace,"model":request.model,"approvalPolicy":"never","sandbox":"read-only","ephemeral":true,"baseInstructions":system,"developerInstructions":developer_instructions,"dynamicTools":tools})).await?;
                state.thread_id=Some(result.pointer("/result/thread/id").and_then(Value::as_str).map(str::to_owned).ok_or_else(||ProviderError::InvalidResponse("thread/start omitted result.thread.id".into()))?);
            }
            if let Some(pending)=state.pending_tool.take() {
                let message=request.messages.iter().rev().find(|m|m.role==Role::Tool&&m.tool_call_id.as_deref()==Some(&pending.call_id)).ok_or_else(||ProviderError::InvalidResponse(format!("missing Helm result for dynamic tool {}",pending.call_id)))?;
                client.respond(pending.request_id,json!({"success":message.tool_success.unwrap_or(false),"contentItems":[{"type":"inputText","text":message.content}]})).await?;
            } else if !state.active {
                let input=if state.turn_id.is_none(){initial_input(&request.messages)}else{request.messages.iter().rev().find(|m|m.role==Role::User).map(|m|m.content.clone()).unwrap_or_default()};
                let thread_id=state.thread_id.clone().expect("thread initialized");
                let result=client.request("turn/start",json!({"threadId":thread_id,"input":[{"type":"text","text":input}],"model":request.model})).await?;
                state.turn_id=result.pointer("/result/turn/id").and_then(Value::as_str).map(str::to_owned);
                state.active=true;
            }
            let thread_id=state.thread_id.clone().expect("thread initialized");
            let mut text=String::new();let mut usage=Usage::default();
            startup_guard.armed=false;
            let mut guard=TurnGuard{client:client.clone(),thread_id:thread_id.clone(),turn_id:state.turn_id.clone(),cancelled:cancelled.clone(),armed:true};
            loop {
                let message=client.next().await?;
                let method=message.get("method").and_then(Value::as_str).unwrap_or_default();
                match method {
                    "item/agentMessage/delta" if belongs(&message,&thread_id,state.turn_id.as_deref()) => {if let Some(delta)=message.pointer("/params/delta").and_then(Value::as_str){text.push_str(delta);yield ProviderStreamEvent::Delta(ProviderDelta::Text(delta.into()));}},
                    "thread/tokenUsage/updated" if belongs(&message,&thread_id,state.turn_id.as_deref()) => {usage.input_tokens=message.pointer("/params/tokenUsage/last/inputTokens").and_then(Value::as_u64).unwrap_or(usage.input_tokens);usage.output_tokens=message.pointer("/params/tokenUsage/last/outputTokens").and_then(Value::as_u64).unwrap_or(usage.output_tokens);},
                    "item/tool/call" if message.get("id").is_some() => {let params=&message["params"];let call_id=params.get("callId").and_then(Value::as_str).ok_or_else(||ProviderError::InvalidResponse("dynamic tool request omitted callId".into()))?.to_owned();let name=params.get("tool").and_then(Value::as_str).ok_or_else(||ProviderError::InvalidResponse("dynamic tool request omitted tool".into()))?.to_owned();let arguments=params.get("arguments").cloned().unwrap_or_else(||json!({}));state.pending_tool=Some(PendingTool{request_id:message["id"].clone(),call_id:call_id.clone()});guard.armed=false;yield ProviderStreamEvent::Delta(ProviderDelta::ToolCall{index:0,id:Some(call_id.clone()),name:Some(name.clone()),arguments:arguments.to_string()});yield ProviderStreamEvent::Completed(ModelResponse{message:Message{role:Role::Assistant,content:text,tool_call_id:None,tool_calls:vec![ToolCall{id:call_id,name,arguments}],tool_success:None,provider_state:None},usage});return;},
                    "turn/completed" if message.pointer("/params/threadId").and_then(Value::as_str)==Some(&thread_id)&&message.pointer("/params/turn/id").and_then(Value::as_str)==state.turn_id.as_deref() => {let status=message.pointer("/params/turn/status").and_then(Value::as_str).unwrap_or("failed");guard.armed=false;state.active=false;if status!="completed"{Err(classify_turn_error(&message,status))?;}yield ProviderStreamEvent::Completed(ModelResponse{message:Message{role:Role::Assistant,content:text,tool_call_id:None,tool_calls:Vec::new(),tool_success:None,provider_state:None},usage});return;},
                    "error" => Err(classify_notification_error(&message))?,
                    _ if message.get("id").is_some()&&message.get("method").is_some()=>{client.reject(message["id"].clone(),"Helm rejects non-dynamic app-server tool requests").await?;},
                    _=>{}
                }
            }
        }))
    }
}

impl Drop for CodexSubscriptionProvider {
    fn drop(&mut self) {
        if let Ok(state) = self.state.try_lock()
            && let Some(client) = &state.client
        {
            client.stop();
        }
    }
}

impl AppClient {
    async fn spawn(program: &str, args: &[String]) -> Result<Arc<Self>, ProviderError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| {
            ProviderError::Unavailable(format!("failed to start `{program} app-server`: {e}"))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProviderError::Unavailable("app-server stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::Unavailable("app-server stdout unavailable".into()))?;
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        tokio::spawn(read_messages(stdout, sender));
        Ok(Arc::new(Self {
            stdin: Mutex::new(stdin),
            inbox: Mutex::new(Inbox {
                receiver,
                backlog: VecDeque::new(),
            }),
            child: StdMutex::new(child),
            next_id: AtomicU64::new(1),
        }))
    }
    async fn initialize(&self) -> Result<(), ProviderError> {
        self.request("initialize",json!({"clientInfo":{"name":"helm","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        self.send(json!({"method":"initialized"})).await
    }
    async fn request(&self, method: &str, params: Value) -> Result<Value, ProviderError> {
        tokio::time::timeout(CONTROL_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| {
                ProviderError::Unavailable(format!(
                    "codex app-server `{method}` timed out after {} seconds",
                    CONTROL_TIMEOUT.as_secs()
                ))
            })?
    }
    async fn request_inner(&self, method: &str, params: Value) -> Result<Value, ProviderError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        let mut inbox = self.inbox.lock().await;
        loop {
            if let Some(position) = inbox.backlog.iter().position(|v| {
                v.get("id").and_then(Value::as_u64) == Some(id) && v.get("method").is_none()
            }) {
                return Ok(inbox.backlog.remove(position).expect("position exists"));
            }
            let value = inbox
                .receiver
                .recv()
                .await
                .ok_or_else(|| ProviderError::Unavailable("app-server exited".into()))?
                .map_err(ProviderError::InvalidResponse)?;
            if value.get("id").and_then(Value::as_u64) == Some(id) && value.get("method").is_none()
            {
                if let Some(error) = value.get("error") {
                    return Err(classify_rpc_error(error));
                }
                return Ok(value);
            }
            if inbox.backlog.len() >= CHANNEL_CAPACITY {
                return Err(ProviderError::InvalidResponse(
                    "app-server correlation backlog exceeded 256 messages".into(),
                ));
            }
            inbox.backlog.push_back(value);
        }
    }
    async fn next(&self) -> Result<Value, ProviderError> {
        let mut inbox = self.inbox.lock().await;
        if let Some(value) = inbox.backlog.pop_front() {
            return Ok(value);
        }
        inbox
            .receiver
            .recv()
            .await
            .ok_or_else(|| ProviderError::Unavailable("app-server exited".into()))?
            .map_err(ProviderError::InvalidResponse)
    }
    async fn respond(&self, id: Value, result: Value) -> Result<(), ProviderError> {
        self.send(json!({"id":id,"result":result})).await
    }
    async fn reject(&self, id: Value, message: &str) -> Result<(), ProviderError> {
        self.send(json!({"id":id,"error":{"code":-32601,"message":message}}))
            .await
    }
    async fn send(&self, value: Value) -> Result<(), ProviderError> {
        let mut bytes = serde_json::to_vec(&value)
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(ProviderError::Request(
                "app-server request exceeded 4 MiB".into(),
            ));
        }
        bytes.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))
    }
    fn interrupt(self: &Arc<Self>, thread_id: String, turn_id: String) {
        let client = self.clone();
        tokio::spawn(async move {
            let _ = client
                .request(
                    "turn/interrupt",
                    json!({"threadId":thread_id,"turnId":turn_id}),
                )
                .await;
        });
    }
    fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }
}

struct TurnGuard {
    client: Arc<AppClient>,
    thread_id: String,
    turn_id: Option<String>,
    cancelled: Arc<AtomicBool>,
    armed: bool,
}

struct StartupGuard {
    client: Arc<AppClient>,
    cancelled: Arc<AtomicBool>,
    armed: bool,
}
impl Drop for StartupGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::SeqCst);
            self.client.stop();
        }
    }
}
impl Drop for TurnGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancelled.store(true, Ordering::SeqCst);
            if let Some(turn) = self.turn_id.take() {
                self.client.interrupt(self.thread_id.clone(), turn);
            }
        }
    }
}

async fn read_messages(
    stdout: tokio::process::ChildStdout,
    sender: mpsc::Sender<Result<Value, String>>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) => break,
            Ok(_) if line.len() > MAX_MESSAGE_BYTES => {
                let _ = sender
                    .send(Err("app-server message exceeded 4 MiB".into()))
                    .await;
                break;
            }
            Ok(_) => match serde_json::from_slice(&line) {
                Ok(value) => {
                    if sender.send(Ok(value)).await.is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender
                        .send(Err(format!("invalid app-server JSON: {error}")))
                        .await;
                    break;
                }
            },
            Err(error) => {
                let _ = sender.send(Err(error.to_string())).await;
                break;
            }
        }
    }
}
fn belongs(message: &Value, thread: &str, turn: Option<&str>) -> bool {
    message.pointer("/params/threadId").and_then(Value::as_str) == Some(thread)
        && turn
            .is_none_or(|id| message.pointer("/params/turnId").and_then(Value::as_str) == Some(id))
}
fn initial_input(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| {
            let calls = m
                .tool_calls
                .iter()
                .map(|call| format!("tool_call {} {} {}", call.id, call.name, call.arguments))
                .collect::<Vec<_>>()
                .join("\n");
            let outcome = m
                .tool_success
                .map(|success| format!("tool_success: {success}\n"))
                .unwrap_or_default();
            format!(
                "{:?}: {}{}{}",
                m.role,
                outcome,
                m.content,
                if calls.is_empty() {
                    String::new()
                } else {
                    format!("\n{calls}")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn classify_rpc_error(error: &Value) -> ProviderError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("app-server request failed")
        .to_owned();
    if message.to_ascii_lowercase().contains("auth") {
        ProviderError::Authentication(message)
    } else {
        ProviderError::Request(message)
    }
}
fn classify_notification_error(value: &Value) -> ProviderError {
    let message = value
        .pointer("/params/error/message")
        .or_else(|| value.pointer("/params/message"))
        .and_then(Value::as_str)
        .unwrap_or("app-server error")
        .to_owned();
    classify_message(message)
}
fn classify_turn_error(value: &Value, status: &str) -> ProviderError {
    let message = value
        .pointer("/params/turn/error/message")
        .and_then(Value::as_str)
        .unwrap_or(status)
        .to_owned();
    match value
        .pointer("/params/turn/error/codexErrorInfo")
        .and_then(Value::as_str)
    {
        Some("rateLimitExceeded" | "usageLimitExceeded") => {
            return ProviderError::RateLimit {
                message,
                retry_after: None,
            };
        }
        Some("serverOverloaded" | "internalServerError") => {
            return ProviderError::Unavailable(message);
        }
        Some("unauthorized") => return ProviderError::Authentication(message),
        _ => {}
    }
    classify_message(message)
}
fn classify_message(message: String) -> ProviderError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("usage limit") || lower.contains("rate limit") {
        ProviderError::RateLimit {
            message,
            retry_after: None,
        }
    } else if lower.contains("unauthorized") || lower.contains("login") {
        ProviderError::Authentication(message)
    } else {
        ProviderError::Request(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    async fn completed(stream: &mut ProviderStream) -> ModelResponse {
        while let Some(event) = stream.next().await {
            if let ProviderStreamEvent::Completed(response) = event.unwrap() {
                return response;
            }
        }
        panic!("stream ended")
    }
    #[tokio::test]
    async fn fixture_correlates_streams_tools_and_repeated_turns() {
        let script = r#"read init; echo '{"id":1,"result":{"userAgent":"fake","platformFamily":"unix","platformOs":"linux","codexHome":"/tmp"}}'; read initialized; read thread; echo '{"id":2,"result":{"thread":{"id":"th1"}}}'; read turn; echo '{"id":3,"result":{"turn":{"id":"tu1","status":"inProgress","items":[]}}}'; echo '{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","itemId":"i","delta":"hello "}}'; echo '{"id":99,"method":"item/tool/call","params":{"threadId":"th1","turnId":"tu1","callId":"c1","tool":"read_file","arguments":{"path":"a"}}}'; read tool_result; echo '{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","itemId":"i","delta":"done"}}'; echo '{"method":"thread/tokenUsage/updated","params":{"threadId":"th1","turnId":"tu1","tokenUsage":{"last":{"inputTokens":4,"outputTokens":2}}}}'; echo '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","status":"completed","items":[]}}}'; read turn2; echo '{"id":4,"result":{"turn":{"id":"tu2","status":"inProgress","items":[]}}}'; echo '{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu2","itemId":"i2","delta":"again"}}'; echo '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu2","status":"completed","items":[]}}}'"#;
        let provider = CodexSubscriptionProvider::with_command(
            "sh".into(),
            vec!["-c".into(), script.into()],
            PathBuf::from("/tmp"),
        );
        let tools = vec![crate::model::ToolDefinition {
            name: "read_file".into(),
            description: "read".into(),
            input_schema: json!({"type":"object"}),
        }];
        let request = ModelRequest {
            model: "test".into(),
            messages: vec![
                Message::new(Role::System, "rules"),
                Message::new(Role::User, "go"),
            ],
            tools: tools.clone(),
            temperature: None,
            max_tokens: None,
        };
        let mut stream = provider.stream(request.clone()).await.unwrap();
        let first = completed(&mut stream).await;
        assert_eq!(first.message.tool_calls[0].name, "read_file");
        drop(stream);
        let mut messages = request.messages;
        messages.push(first.message);
        messages.push(Message::tool_result("c1", "file", true));
        let mut stream = provider
            .stream(ModelRequest {
                model: "test".into(),
                messages: messages.clone(),
                tools: tools.clone(),
                temperature: None,
                max_tokens: None,
            })
            .await
            .unwrap();
        let response = completed(&mut stream).await;
        assert_eq!(response.message.content, "done");
        assert_eq!(response.usage.input_tokens, 4);
        drop(stream);
        let mut stream = provider
            .stream(ModelRequest {
                model: "test".into(),
                messages: vec![Message::new(Role::User, "next")],
                tools,
                temperature: None,
                max_tokens: None,
            })
            .await
            .unwrap();
        let again = completed(&mut stream).await;
        assert_eq!(again.message.content, "again");
    }
    #[tokio::test]
    async fn discovers_models_then_reuses_initialized_client_for_a_turn() {
        let script = r#"read init; echo '{"id":1,"result":{"userAgent":"fake","platformFamily":"unix","platformOs":"linux","codexHome":"/tmp"}}'; read initialized; read models; echo '{"id":2,"result":{"data":[{"id":"catalog-id","model":"model-a","displayName":"Model A","description":"A model","hidden":false,"isDefault":true,"defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low","description":"fast"}],"inputModalities":["text"]}],"nextCursor":null}}'; read thread; echo '{"id":3,"result":{"thread":{"id":"th1"}}}'; read turn; echo '{"id":4,"result":{"turn":{"id":"tu1","status":"inProgress","items":[]}}}'; echo '{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","itemId":"i","delta":"ok"}}'; echo '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","status":"completed","items":[]}}}'"#;
        let provider = CodexSubscriptionProvider::with_command(
            "sh".into(),
            vec!["-c".into(), script.into()],
            PathBuf::from("/tmp"),
        );
        let models = provider.models().await.unwrap();
        assert_eq!(models[0].id, "model-a");
        assert!(models[0].is_default);
        assert_eq!(models[0].reasoning_efforts, vec!["low"]);
        let mut stream = provider
            .stream(ModelRequest {
                model: "model-a".into(),
                messages: vec![Message::new(Role::User, "hello")],
                tools: Vec::new(),
                temperature: None,
                max_tokens: None,
            })
            .await
            .unwrap();
        assert_eq!(completed(&mut stream).await.message.content, "ok");
    }
    #[tokio::test]
    async fn rejects_oversized_fixture_lines() {
        let (sender, mut receiver) = mpsc::channel::<Result<Value, String>>(1);
        let (mut writer, reader) = tokio::io::duplex(MAX_MESSAGE_BYTES + 16);
        tokio::spawn(async move {
            let _ = writer.write_all(&vec![b'x'; MAX_MESSAGE_BYTES + 1]).await;
            let _ = writer.write_u8(b'\n').await;
        });
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut line = Vec::new();
            let _ = reader.read_until(b'\n', &mut line).await;
            if line.len() > MAX_MESSAGE_BYTES {
                let _ = sender
                    .send(Err("app-server message exceeded 4 MiB".into()))
                    .await;
            }
        });
        assert!(receiver.recv().await.unwrap().is_err());
    }
}
