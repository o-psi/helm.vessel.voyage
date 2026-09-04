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
        atomic::{AtomicU64, Ordering},
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
}

impl McpServer {
    pub async fn connect(
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
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(failed)?;
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
            }),
        };
        server.transport.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"helm","version":env!("CARGO_PKG_VERSION")}})).await?;
        server
            .transport
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(server)
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
                    name: format!("mcp_{}_{}", self.name, sanitize(&remote_name)),
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

    pub async fn shutdown(&self) -> Result<(), ToolError> {
        self.transport
            .child
            .lock()
            .await
            .kill()
            .await
            .map_err(failed)
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
            result = self.transport.request("tools/call", json!({"name":self.remote_name,"arguments":arguments})) => result?,
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
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
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
    #[test]
    fn namespaces_safely() {
        assert_eq!(sanitize("Git Tools!"), "git_tools_");
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
