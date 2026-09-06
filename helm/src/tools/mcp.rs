//! Model Context Protocol stdio tool discovery and invocation.
//!
//! Each server is isolated behind a serialized JSON-RPC transport. Server tools are
//! namespaced (`mcp_<server>_<tool>`) so an untrusted server cannot shadow built-ins.
use super::{Tool, ToolContext, ToolError};
use crate::model::ToolDefinition;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};

pub struct McpServer {
    name: String,
    transport: Arc<Transport>,
}
struct Transport {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    rpc: Mutex<()>,
    next_id: AtomicU64,
    observed: AtomicBool,
    #[cfg(not(target_os = "linux"))]
    direct_observed: AtomicBool,
    #[cfg(target_os = "linux")]
    identity: Option<Arc<super::process::SessionIdentity>>,
}

impl McpServer {
    pub async fn connect(
        name: impl Into<String>,
        program: &str,
        args: &[String],
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, ToolError> {
        let server = Self::start(name, program, args, environment)?;
        if let Err(error) = server.initialize().await {
            server.shutdown().await?;
            return Err(error);
        }
        Ok(server)
    }

    /// Acquire the child before awaiting initialization, so callers can retain
    /// and explicitly reap it when an initialization deadline expires.
    pub fn start(
        name: impl Into<String>,
        program: &str,
        args: &[String],
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, ToolError> {
        let name = sanitize(&name.into());
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // An MCP server shares the parent terminal. Its diagnostics must not write through
            // Helm's alternate-screen TUI and corrupt cursor state.
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(target_os = "linux")]
        // SAFETY: setsid is async-signal-safe. Retain this original session's
        // waitable leader until descendants have been observed, just like PTYs.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command.spawn().map_err(failed)?;
        #[cfg(target_os = "linux")]
        let identity = child
            .id()
            .and_then(|id| super::process::SessionIdentity::capture(id).ok())
            .map(Arc::new);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| failed("MCP server has no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failed("MCP server has no stdout"))?;
        let server = Self {
            name,
            transport: Arc::new(Transport {
                child: Mutex::new(child),
                stdin: Mutex::new(stdin),
                stdout: Mutex::new(BufReader::new(stdout)),
                rpc: Mutex::new(()),
                next_id: AtomicU64::new(1),
                observed: AtomicBool::new(false),
                #[cfg(not(target_os = "linux"))]
                direct_observed: AtomicBool::new(false),
                #[cfg(target_os = "linux")]
                identity,
            }),
        };
        Ok(server)
    }

    pub async fn initialize(&self) -> Result<(), ToolError> {
        self.transport.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"helm","version":env!("CARGO_PKG_VERSION")}})).await?;
        self.transport
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(())
    }

    pub async fn discover(&self) -> Result<Vec<Arc<dyn Tool>>, ToolError> {
        let response = self.transport.request("tools/list", json!({})).await?;
        let tools = response
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .ok_or_else(|| failed("MCP tools/list response omitted result.tools"))?;
        tools
            .iter()
            .map(|tool| {
                let remote_name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failed("MCP tool omitted name"))?
                    .to_owned();
                let definition = ToolDefinition {
                    name: tool_name(&self.name, &remote_name),
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("MCP tool")
                        .to_owned(),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type":"object"})),
                };
                Ok(Arc::new(McpTool {
                    remote_name,
                    definition,
                    transport: self.transport.clone(),
                }) as Arc<dyn Tool>)
            })
            .collect()
    }

    /// Non-Linux direct-child retirement is not full session observation. Live
    /// authority handoff must continue checking observed(), never this predicate.
    pub fn can_retire(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.observed()
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.transport.direct_observed.load(Ordering::Acquire)
        }
    }
    pub fn observed(&self) -> bool {
        self.transport.observed.load(Ordering::Acquire)
    }

    /// Idempotent and observation-based: a cancelled earlier shutdown can be
    /// retried without treating a sent kill signal as evidence of exit.
    pub async fn shutdown(&self) -> Result<(), ToolError> {
        let mut child = self.transport.child.lock().await;
        if self.observed() {
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        {
            let identity =
                self.transport.identity.clone().ok_or_else(|| {
                    failed("MCP session identity unavailable; cleanup unconfirmed")
                })?;
            let observed = tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
                while std::time::Instant::now() < deadline {
                    match identity.kill_and_observe(deadline) {
                        Ok(true) => return true,
                        Ok(false) => std::thread::sleep(std::time::Duration::from_millis(5)),
                        Err(_) => return false,
                    }
                }
                false
            })
            .await
            .map_err(|_| failed("MCP cleanup observer failed"))?;
            if !observed {
                return Err(failed("MCP original session cleanup unconfirmed"));
            }
            child.wait().await.map_err(failed)?;
            self.transport.observed.store(true, Ordering::Release);
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Preserve ordinary direct-child cleanup on other platforms, but do
            // not attest process-session observation for an authority handoff.
            if child.try_wait().map_err(failed)?.is_none() {
                child.kill().await.map_err(failed)?;
            }
            self.transport
                .direct_observed
                .store(true, Ordering::Release);
        }
        Ok(())
    }
}

struct McpTool {
    remote_name: String,
    definition: ToolDefinition,
    transport: Arc<Transport>,
}
#[async_trait]
impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        let response = tokio::select! {
            _ = context.cancellation.cancelled() => return Err(ToolError::Cancelled),
            result = tokio::time::timeout(context.timeout, self.transport.request("tools/call", json!({"name":self.remote_name,"arguments":arguments}))) => result.map_err(|_| ToolError::Timeout(context.timeout))??,
        };
        if let Some(error) = response.get("error") {
            return Err(failed(format!("MCP error: {error}")));
        }
        let result = response.get("result").cloned().unwrap_or(Value::Null);
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(failed(extract_content(&result)));
        }
        Ok(extract_content(&result))
    }
}

impl Transport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, ToolError> {
        let _guard = self.rpc.lock().await;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        loop {
            let value = self.receive().await?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(value);
            }
        }
    }
    async fn notify(&self, method: &str, params: Value) -> Result<(), ToolError> {
        self.send(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }
    async fn send(&self, value: Value) -> Result<(), ToolError> {
        let mut stdin = self.stdin.lock().await;
        let mut line = serde_json::to_vec(&value).map_err(failed)?;
        line.push(b'\n');
        stdin.write_all(&line).await.map_err(failed)?;
        stdin.flush().await.map_err(failed)
    }
    async fn receive(&self) -> Result<Value, ToolError> {
        let mut line = String::new();
        let count = self
            .stdout
            .lock()
            .await
            .read_line(&mut line)
            .await
            .map_err(failed)?;
        if count == 0 {
            return Err(failed("MCP server closed stdout"));
        }
        serde_json::from_str(&line).map_err(failed)
    }
}
fn extract_content(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| result.to_string())
}
fn sanitize(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "unnamed".into()
    } else {
        value
    }
}
fn tool_name(server: &str, remote: &str) -> String {
    use sha2::{Digest, Sha256};
    let full = format!("mcp_{}_{}", sanitize(server), sanitize(remote));
    if full.len() <= 64 {
        return full;
    }
    let digest = hex::encode(Sha256::digest(full.as_bytes()));
    format!("{}_{}", &full[..55], &digest[..8])
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

/// Tool-registry ownership. A cancelled runtime construction or child execution
/// still arranges bounded cleanup; retained observers report whether it completed.
pub struct McpLease(pub Arc<McpServer>);
impl Drop for McpLease {
    fn drop(&mut self) {
        if self.0.can_retire() {
            return;
        }
        let server = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), server.shutdown())
                    .await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ApprovalMode, Config},
        policy::Policy,
        tools::Approver,
    };
    use std::time::Duration;
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn registry_drop_observes_server_exit_and_shutdown_is_idempotent() {
        let server = Arc::new(
            McpServer::start(
                "fixture",
                "sh",
                &["-c".into(), "read forever".into()],
                &BTreeMap::new(),
            )
            .unwrap(),
        );
        let mut registry = crate::tools::ToolRegistry::default();
        registry.own_mcp(server.clone());
        assert!(!server.observed());
        drop(registry);
        tokio::time::timeout(Duration::from_secs(3), async {
            while !server.observed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("registry cancellation must reap retained MCP observer");
        server.shutdown().await.unwrap();
        assert!(
            server
                .transport
                .child
                .lock()
                .await
                .try_wait()
                .unwrap()
                .is_some()
        );
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn shutdown_observes_descendants_after_leader_exit_with_held_pipes() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("descendant");
        let server = McpServer::start(
            "fork",
            "sh",
            &[
                "-c".into(),
                r#"sleep 30 & echo $! > "$1"; exit 0"#.into(),
                "fixture".into(),
                marker.to_string_lossy().into_owned(),
            ],
            &BTreeMap::new(),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(marker)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        server.shutdown().await.unwrap();
        assert!(server.observed());
        let live = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|stat| !stat.rsplit_once(") ").unwrap().1.starts_with('Z'));
        assert!(!live, "descendant retaining server stdout survived cleanup");
        server.shutdown().await.unwrap();
    }
    #[test]
    fn namespaces_safely() {
        assert_eq!(sanitize("Git Tools!"), "git_tools_");
        assert!(tool_name(&"a".repeat(80), &"b".repeat(80)).len() <= 64);
    }
    struct Yes;
    #[async_trait]
    impl Approver for Yes {
        async fn approve(
            &self,
            _: &crate::tools::ApprovalRequest,
        ) -> crate::tools::ApprovalOutcome {
            crate::tools::ApprovalOutcome::Approved
        }
    }
    #[tokio::test]
    async fn discovers_and_calls_stdio_tool() {
        let script = r#"read init
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake","version":"1"}}}'
read initialized
read list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"echo","inputSchema":{"type":"object"}}]}}'
read call
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"called"}]}}'
"#;
        let server = McpServer::connect(
            "demo",
            "sh",
            &["-c".into(), script.into()],
            &BTreeMap::new(),
        )
        .await
        .unwrap();
        let tools = server.discover().await.unwrap();
        assert_eq!(tools[0].definition().name, "mcp_demo_echo");
        let directory = tempfile::tempdir().unwrap();
        let config = Config {
            approval: ApprovalMode::Never,
            ..Config::default()
        };
        let context = ToolContext {
            completion: None,
            policy: Arc::new(Policy::new(&config, directory.path().to_owned()).unwrap()),
            approver: Arc::new(Yes),
            timeout: Duration::from_secs(1),
            max_output_bytes: 4096,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Attended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        };
        assert_eq!(
            tools[0]
                .execute(json!({"value":"x"}), &context)
                .await
                .unwrap(),
            "called"
        );
    }
}
