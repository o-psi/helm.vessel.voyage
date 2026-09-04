use super::{Tool, ToolContext, ToolError, truncate};
use crate::{model::ToolDefinition, policy::Decision};
use async_trait::async_trait;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default)]
pub struct ProcessTool {
    processes: Arc<Mutex<BTreeMap<Uuid, Managed>>>,
}

struct Managed {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Capture>>,
    cursor: usize,
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    base: usize,
}
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

impl Drop for Managed {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Args {
    Start {
        command: String,
        #[serde(default = "default_rows")]
        rows: u16,
        #[serde(default = "default_cols")]
        cols: u16,
    },
    Read {
        id: Uuid,
    },
    Write {
        id: Uuid,
        data: String,
    },
    Resize {
        id: Uuid,
        rows: u16,
        cols: u16,
    },
    Terminate {
        id: Uuid,
    },
    List,
}
fn default_rows() -> u16 {
    24
}
fn default_cols() -> u16 {
    80
}

#[async_trait]
impl Tool for ProcessTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
        name: "process".into(),
        description: "Manage persistent PTY-backed terminal processes. Start, read incremental output, write input, resize, list, or terminate.".into(),
        input_schema: json!({"type":"object","properties":{"action":{"enum":["start","read","write","resize","terminate","list"]},"command":{"type":"string"},"id":{"type":"string"},"data":{"type":"string"},"rows":{"type":"integer","minimum":1},"cols":{"type":"integer","minimum":1}},"required":["action"]}),
    }
    }

    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(value)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        match args {
            Args::Start {
                command,
                rows,
                cols,
            } => {
                match ctx.policy.command(&command) {
                    Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
                    Decision::Ask(reason) if !ctx.approver.approve(&reason).await => {
                        return Err(ToolError::Denied("user declined approval".into()));
                    }
                    _ => {}
                }
                self.start(command, rows, cols, ctx)
            }
            Args::Read { id } => self.read(id, ctx.max_output_bytes),
            Args::Write { id, data } => self.write(id, &data),
            Args::Resize { id, rows, cols } => self.resize(id, rows, cols),
            Args::Terminate { id } => self.terminate(id),
            Args::List => self.list(),
        }
    }
}

impl ProcessTool {
    fn start(
        &self,
        command: String,
        rows: u16,
        cols: u16,
        ctx: &ToolContext,
    ) -> Result<String, ToolError> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(failed)?;
        let mut builder = CommandBuilder::new("sh");
        builder.arg("-lc");
        builder.arg(command);
        builder.cwd(ctx.policy.workspace());
        for (key, value) in &ctx.environment {
            builder.env(key, value);
        }
        let child = pair.slave.spawn_command(builder).map_err(failed)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(failed)?;
        let writer = pair.master.take_writer().map_err(failed)?;
        let output = Arc::new(Mutex::new(Capture::default()));
        let sink = output.clone();
        std::thread::Builder::new()
            .name("helm-pty-reader".into())
            .spawn(move || {
                let mut buffer = [0_u8; 8192];
                loop {
                    match std::io::Read::read(&mut reader, &mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut capture = sink.lock().expect("PTY buffer poisoned");
                            capture.bytes.extend_from_slice(&buffer[..n]);
                            if capture.bytes.len() > MAX_CAPTURE_BYTES {
                                let remove = capture.bytes.len() - MAX_CAPTURE_BYTES;
                                capture.bytes.drain(..remove);
                                capture.base += remove;
                            }
                        }
                    }
                }
            })
            .map_err(failed)?;
        let id = Uuid::new_v4();
        self.processes.lock().map_err(failed)?.insert(
            id,
            Managed {
                master: pair.master,
                child,
                writer,
                output,
                cursor: 0,
            },
        );
        Ok(format!("started PTY process {id}"))
    }
    fn read(&self, id: Uuid, max: usize) -> Result<String, ToolError> {
        let mut processes = self.processes.lock().map_err(failed)?;
        let process = processes
            .get_mut(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        let capture = process.output.lock().map_err(failed)?;
        let offset = process
            .cursor
            .saturating_sub(capture.base)
            .min(capture.bytes.len());
        let result = truncate(capture.bytes[offset..].to_vec(), max);
        process.cursor = capture.base + capture.bytes.len();
        let status = process
            .child
            .try_wait()
            .map_err(failed)?
            .map(|s| format!("exited: {s:?}"))
            .unwrap_or_else(|| "running".into());
        Ok(format!("status: {status}\n{result}"))
    }
    fn write(&self, id: Uuid, data: &str) -> Result<String, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        let p = map
            .get_mut(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        p.writer
            .write_all(data.as_bytes())
            .and_then(|_| p.writer.flush())
            .map_err(failed)?;
        Ok(format!("wrote {} bytes", data.len()))
    }
    fn resize(&self, id: Uuid, rows: u16, cols: u16) -> Result<String, ToolError> {
        let map = self.processes.lock().map_err(failed)?;
        let p = map
            .get(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        p.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(failed)?;
        Ok(format!("resized to {cols}x{rows}"))
    }
    fn terminate(&self, id: Uuid) -> Result<String, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        let mut p = map
            .remove(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        if p.child.try_wait().map_err(failed)?.is_none() {
            p.child.kill().map_err(failed)?;
        }
        Ok(format!("terminated {id}"))
    }
    fn list(&self) -> Result<String, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        let mut rows = Vec::new();
        for (id, p) in map.iter_mut() {
            let state = if p.child.try_wait().map_err(failed)?.is_some() {
                "exited"
            } else {
                "running"
            };
            rows.push(format!("{id}\t{state}"));
        }
        Ok(rows.join("\n"))
    }
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
    struct Yes;
    #[async_trait]
    impl Approver for Yes {
        async fn approve(&self, _: &str) -> bool {
            true
        }
    }
    fn context(root: &std::path::Path) -> ToolContext {
        let config = Config {
            approval: ApprovalMode::Never,
            ..Config::default()
        };
        ToolContext {
            policy: Arc::new(Policy::new(&config, root.to_owned()).unwrap()),
            approver: Arc::new(Yes),
            timeout: Duration::from_secs(2),
            max_output_bytes: 4096,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }
    #[tokio::test]
    async fn pty_process_accepts_incremental_io() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        let started = tool.execute(json!({"action":"start","command":"printf ready; read line; printf 'got:%s' \"$line\""}), &ctx).await.unwrap();
        let id = Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap();
        let mut first = String::new();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            first.push_str(
                &tool
                    .execute(json!({"action":"read","id":id}), &ctx)
                    .await
                    .unwrap(),
            );
            if first.contains("ready") {
                break;
            }
        }
        assert!(first.contains("ready"));
        tool.execute(json!({"action":"write","id":id,"data":"hello\n"}), &ctx)
            .await
            .unwrap();
        let mut second = String::new();
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            second.push_str(
                &tool
                    .execute(json!({"action":"read","id":id}), &ctx)
                    .await
                    .unwrap(),
            );
            if second.contains("got:hello") {
                break;
            }
        }
        assert!(second.contains("got:hello"));
        tool.execute(json!({"action":"terminate","id":id}), &ctx)
            .await
            .unwrap();
    }
}
