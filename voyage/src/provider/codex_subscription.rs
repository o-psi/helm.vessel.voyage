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

mod cleanup;
pub(crate) use cleanup::shutdown as shutdown_owned;

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const CHANNEL_CAPACITY: usize = 256;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

pub struct CodexSubscriptionProvider {
    state: Arc<Mutex<State>>,
    sandbox: Option<crate::sandbox::Sandbox>,
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
    system_instructions: Option<String>,
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
    cleanup_observed: AtomicBool,
    _slot: tokio::sync::OwnedSemaphorePermit,
    #[cfg(target_os = "linux")]
    identity: Option<Arc<crate::tools::SessionIdentity>>,
}
struct Inbox {
    receiver: mpsc::Receiver<Result<Value, String>>,
    backlog: VecDeque<Value>,
}

impl CodexSubscriptionProvider {
    pub fn with_sandbox(
        mut self,
        settings: &crate::sandbox::Settings,
    ) -> Result<Self, ProviderError> {
        self.sandbox = Some(
            crate::sandbox::Sandbox::bridge(settings)
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?,
        );
        Ok(self)
    }
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
                system_instructions: None,
            })),
            sandbox: None,
            program,
            args,
            workspace,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl Provider for CodexSubscriptionProvider {
    fn supports_steering(&self) -> bool {
        false
    }

    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut state = self.state.lock().await;
        if state.active {
            return Err(ProviderError::Unavailable(
                "model discovery is unavailable during an active turn".into(),
            ));
        }
        if state.client.is_none() {
            let client = AppClient::spawn(&self.program, &self.args, self.sandbox.as_ref()).await?;
            client.initialize().await?;
            state.initialized = true;
            state.client = Some(client);
        }
        let client = state.client.as_ref().expect("client initialized").clone();
        let mut cursor: Option<String> = None;
        let mut models = Vec::new();
        let mut cursors = std::collections::BTreeSet::new();
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
            if models.len().saturating_add(data.len()) > super::catalog::MAX_MODELS {
                return Err(ProviderError::InvalidResponse(
                    "model list exceeds 1024 entries".into(),
                ));
            }
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
                let reasoning_efforts = super::catalog::strings(
                    value,
                    "supportedReasoningEfforts",
                    Some("reasoningEffort"),
                )?;
                let input_modalities = super::catalog::strings(value, "inputModalities", None)?;
                models.push(ModelInfo {
                    id: id.to_owned(),
                    display_name: super::catalog::optional_text(value, "displayName", id)?
                        .to_owned(),
                    description: super::catalog::optional_text(value, "description", "")?
                        .to_owned(),
                    is_default: super::catalog::optional_bool(value, "isDefault", false)?,
                    reasoning_efforts,
                    reasoning_support_known: value
                        .get("supportedReasoningEfforts")
                        .is_some_and(Value::is_array),
                    default_reasoning_effort: super::catalog::nullable_text(
                        value,
                        "defaultReasoningEffort",
                    )?,
                    service_tiers: super::catalog::strings(value, "serviceTiers", Some("id"))?,
                    service_support_known: value.get("serviceTiers").is_some_and(Value::is_array),
                    default_service_tier: super::catalog::nullable_text(
                        value,
                        "defaultServiceTier",
                    )?,
                    observed_at_ms: Some(super::catalog::now_ms()),
                    input_modalities,
                });
            }
            // Reject an oversized or unsafe accumulated catalog before another query.
            super::validate_models(&models, &[])?;
            cursor = match result.get("nextCursor") {
                None | Some(Value::Null) => None,
                Some(Value::String(cursor)) => Some(cursor.clone()),
                _ => {
                    return Err(ProviderError::InvalidResponse(
                        "model/list cursor must be a string or null".into(),
                    ));
                }
            };
            if let Some(cursor) = &cursor {
                super::catalog::validate_text(cursor, 512, true, &[])?;
                if !cursors.insert(cursor.clone()) || cursors.len() >= super::catalog::MAX_PAGES {
                    return Err(ProviderError::InvalidResponse(
                        "model pagination repeated or exceeded 16 pages".into(),
                    ));
                }
            } else {
                break;
            }
        }
        super::validate_models(&models, &[])?;
        normalize_models(&mut models);
        Ok(models)
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        super::multimodal::refuse_codex(&request)?;
        super::inference::validate_request(
            &crate::config::ProviderKind::CodexSubscription,
            &request,
        )?;
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
        super::multimodal::refuse_codex(&request)?;
        super::inference::validate_request(
            &crate::config::ProviderKind::CodexSubscription,
            &request,
        )?;
        let state = self.state.clone();
        let program = self.program.clone();
        let args = self.args.clone();
        let sandbox = self.sandbox.clone();
        let workspace = self.workspace.clone();
        let cancelled = self.cancelled.clone();
        Ok(Box::pin(async_stream::try_stream! {
            let mut state=state.lock().await;
            if cancelled.swap(false,Ordering::SeqCst) { state.client=None;state.initialized=false;state.thread_id=None;state.active=false;state.pending_tool=None;state.turn_id=None; }
            if state.client.is_none() { state.client=Some(AppClient::spawn(&program,&args,sandbox.as_ref()).await?); }
            let client=state.client.as_ref().expect("client initialized").clone();
            let mut startup_guard=StartupGuard { client:client.clone(),cancelled:cancelled.clone(),armed:true };
            let system=request.messages.iter().filter(|m|m.role==Role::System).map(|m|m.content.as_str()).collect::<Vec<_>>().join("\n\n");
            // Thread instructions are immutable in the compatibility bridge. Rebuild only
            // between completed turns, replaying canonical history into the new thread.
            if !state.active && state.pending_tool.is_none()
                && state.system_instructions.as_ref().is_some_and(|old| old != &system) {
                state.thread_id=None;
                state.turn_id=None;
            }
            if state.thread_id.is_none() {
                if !state.initialized { client.initialize().await?; state.initialized=true; }
                let tools=request.tools.iter().map(|tool|json!({"type":"function","name":tool.name,"description":tool.description,"inputSchema":tool.input_schema,"deferLoading":false})).collect::<Vec<_>>();
                let tool_names=request.tools.iter().map(|tool|tool.name.as_str()).collect::<Vec<_>>().join(", ");
                let developer_instructions=format!("You are the model inside Helm, not the Codex host around it. Never use or claim built-in command, filesystem, web, MCP, app, plugin, skill, collaboration, image, document, or patch capabilities. Use only client-provided dynamic tools; Helm enforces all policy and executes every action. The exact available call names are: {tool_names}. If asked about tools, answer from that list only.");
                let result=client.request("thread/start",json!({"cwd":workspace,"model":request.model,"approvalPolicy":"never","sandbox":"read-only","ephemeral":true,"baseInstructions":system,"developerInstructions":developer_instructions,"dynamicTools":tools})).await?;
                state.thread_id=Some(result.pointer("/result/thread/id").and_then(Value::as_str).map(str::to_owned).ok_or_else(||ProviderError::InvalidResponse("thread/start omitted result.thread.id".into()))?);
                state.system_instructions=Some(system);
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
                    "item/tool/call" if message.get("id").is_some() => {let params=&message["params"];let call_id=params.get("callId").and_then(Value::as_str).ok_or_else(||ProviderError::InvalidResponse("dynamic tool request omitted callId".into()))?.to_owned();let name=params.get("tool").and_then(Value::as_str).ok_or_else(||ProviderError::InvalidResponse("dynamic tool request omitted tool".into()))?.to_owned();let arguments=params.get("arguments").cloned().unwrap_or_else(||json!({}));state.pending_tool=Some(PendingTool{request_id:message["id"].clone(),call_id:call_id.clone()});guard.armed=false;yield ProviderStreamEvent::Delta(ProviderDelta::ToolCall{index:0,id:Some(call_id.clone()),name:Some(name.clone()),arguments:arguments.to_string()});yield ProviderStreamEvent::Completed(ModelResponse{service_tier:None,message: Message { tool_outcome: None, tool_output: None, parts: Vec::new(),image_data: Default::default(),operator_name: None,created_at:Some(chrono::Utc::now()),role:Role::Assistant,content:text,tool_call_id:None,tool_calls:vec![ToolCall{id:call_id,name,arguments}],tool_success:None,provider_state:None,steering:None},usage});return;},
                    "turn/completed" if message.pointer("/params/threadId").and_then(Value::as_str)==Some(&thread_id)&&message.pointer("/params/turn/id").and_then(Value::as_str)==state.turn_id.as_deref() => {let status=message.pointer("/params/turn/status").and_then(Value::as_str).unwrap_or("failed");guard.armed=false;state.active=false;if status!="completed"{Err(classify_turn_error(&message,status))?;}yield ProviderStreamEvent::Completed(ModelResponse{service_tier:None,message: Message { tool_outcome: None, tool_output: None, parts: Vec::new(),image_data: Default::default(),operator_name: None,created_at:Some(chrono::Utc::now()),role:Role::Assistant,content:text,tool_call_id:None,tool_calls:Vec::new(),tool_success:None,provider_state:None,steering:None},usage});return;},
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
    async fn spawn(
        program: &str,
        args: &[String],
        sandbox: Option<&crate::sandbox::Sandbox>,
    ) -> Result<Arc<Self>, ProviderError> {
        let slot = cleanup::capacity()?;
        let mut command = Command::new(program);
        if let Some(sandbox) = sandbox.filter(|sandbox| sandbox.required()) {
            command.env_clear();
            for key in &sandbox.settings.bridge_inherit_env {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
        }
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(target_os = "linux")]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        if let Some(sandbox) = sandbox {
            sandbox
                .apply(command.as_std_mut(), std::path::Path::new("/tmp"))
                .map_err(|error| ProviderError::Unavailable(error.to_string()))?;
        }
        let mut child = command.spawn().map_err(|e| {
            ProviderError::Unavailable(format!("failed to start `{program} app-server`: {e}"))
        })?;
        #[cfg(target_os = "linux")]
        let identity = child
            .id()
            .and_then(|id| crate::tools::SessionIdentity::capture(id).ok())
            .map(Arc::new);
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
        let client = Arc::new(Self {
            stdin: Mutex::new(stdin),
            inbox: Mutex::new(Inbox {
                receiver,
                backlog: VecDeque::new(),
            }),
            child: StdMutex::new(child),
            next_id: AtomicU64::new(1),
            cleanup_observed: AtomicBool::new(false),
            _slot: slot,
            #[cfg(target_os = "linux")]
            identity,
        });
        cleanup::retain(client.clone())?;
        Ok(client)
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
        Some("usageLimitExceeded") => return ProviderError::UsageLimit,
        Some("rateLimitExceeded") => {
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
    if lower.contains("usage limit") {
        ProviderError::UsageLimit
    } else if lower.contains("rate limit") {
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
