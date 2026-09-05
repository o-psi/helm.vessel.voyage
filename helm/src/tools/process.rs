mod shutdown;
use super::{Tool, ToolContext, ToolError};
use crate::terminal::{
    InteractiveTerminals, TerminalCell, TerminalColor, TerminalError, TerminalEvent, TerminalId,
    TerminalSnapshot, TerminalState as UiState, TerminalSummary,
};
use crate::{model::ToolDefinition, policy::Decision};
use async_trait::async_trait;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(target_os = "linux")]
pub(crate) use shutdown::linux::SessionIdentity;
use shutdown::{OwnedChild, ReaderDone, StartupChild};
pub use shutdown::{TerminalShutdown, TerminalShutdownFailure};
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct ProcessTool {
    processes: Arc<Mutex<BTreeMap<Uuid, Managed>>>,
    pending: Arc<Mutex<BTreeMap<Uuid, OwnedChild>>>,
    starting: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    uncertain: Arc<AtomicBool>,
    selected: Arc<Mutex<Option<Uuid>>>,
    max_count: usize,
    max_unread_bytes: usize,
    events: broadcast::Sender<TerminalEvent>,
}
pub type TerminalManager = ProcessTool;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalMetadata {
    pub id: Uuid,
    pub name: Option<String>,
    pub command: String,
    pub cwd: std::path::PathBuf,
    pub rows: u16,
    pub cols: u16,
    pub state: String,
    pub unread_bytes: usize,
}

impl Default for ProcessTool {
    fn default() -> Self {
        Self::with_limits(16, MAX_CAPTURE_BYTES)
    }
}

struct Managed {
    name: Option<String>,
    command: String,
    cwd: std::path::PathBuf,
    rows: u16,
    cols: u16,
    master: Box<dyn MasterPty + Send>,
    child: OwnedChild,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<Capture>>,
    notification_pending: Arc<AtomicBool>,
    cursor: usize,
}

struct Capture {
    bytes: Vec<u8>,
    base: usize,
    dropped: u64,
    parser: vt100::Parser,
}
impl Capture {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            bytes: Vec::new(),
            base: 0,
            dropped: 0,
            parser: vt100::Parser::new(rows, cols, 0),
        }
    }
}
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Args {
    Start {
        command: String,
        name: Option<String>,
        cwd: Option<std::path::PathBuf>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default = "default_rows")]
        rows: u16,
        #[serde(default = "default_cols")]
        cols: u16,
    },
    Read {
        id: Option<Uuid>,
        name: Option<String>,
    },
    Write {
        id: Option<Uuid>,
        name: Option<String>,
        data: String,
    },
    Resize {
        id: Option<Uuid>,
        name: Option<String>,
        rows: u16,
        cols: u16,
    },
    Terminate {
        id: Option<Uuid>,
        name: Option<String>,
    },
    Rename {
        id: Option<Uuid>,
        current_name: Option<String>,
        name: Option<String>,
    },
    Interrupt {
        id: Option<Uuid>,
        name: Option<String>,
    },
    Select {
        id: Option<Uuid>,
        name: Option<String>,
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
        description: "Manage multiple persistent PTY-backed terminals with stable IDs and optional names, cwd, and environment. Start, read, write, resize, interrupt, rename, list, or terminate. Use shell for isolated one-shot commands.".into(),
        input_schema: json!({"type":"object","properties":{"action":{"enum":["start","read","write","resize","interrupt","rename","select","terminate","list"]},"command":{"type":"string"},"id":{"type":["string","null"]},"name":{"type":["string","null"]},"current_name":{"type":["string","null"]},"cwd":{"type":"string"},"env":{"type":"object"},"data":{"type":"string"},"rows":{"type":"integer","minimum":1},"cols":{"type":"integer","minimum":1}},"required":["action"]}),
    }
    }

    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(value)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        if self.shutting_down.load(Ordering::SeqCst)
            && !matches!(args, Args::Read { .. } | Args::List)
        {
            return Err(ToolError::Failed(
                "terminal manager is shutting down".into(),
            ));
        }
        match args {
            Args::Start {
                command,
                name,
                cwd,
                env,
                rows,
                cols,
            } => {
                match ctx.policy.command(&command) {
                    Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
                    Decision::Ask(reason)
                        if !ctx
                            .approver
                            .approve(&ctx.approval("process.start", &command, reason.clone()))
                            .await
                            .approved() =>
                    {
                        return Err(ToolError::Denied("user declined approval".into()));
                    }
                    _ => {}
                }
                self.start(command, name, cwd, env, rows, cols, ctx)
            }
            Args::Read { id, name } => {
                self.read(self.resolve(id, name.as_deref())?, ctx.max_output_bytes)
            }
            Args::Write { id, name, data } => self.write(self.resolve(id, name.as_deref())?, &data),
            Args::Resize {
                id,
                name,
                rows,
                cols,
            } => self.resize(self.resolve(id, name.as_deref())?, rows, cols),
            Args::Terminate { id, name } => {
                self.terminate(self.resolve(id, name.as_deref())?).await
            }
            Args::Rename {
                id,
                current_name,
                name,
            } => self.rename(self.resolve(id, current_name.as_deref())?, name),
            Args::Interrupt { id, name } => self.interrupt(self.resolve(id, name.as_deref())?),
            Args::Select { id, name } => {
                let id = self.resolve(id, name.as_deref())?;
                *self.selected.lock().map_err(failed)? = Some(id);
                Ok(format!("selected {id}"))
            }
            Args::List => self.list(),
        }
    }
}

impl ProcessTool {
    pub fn with_limits(max_count: usize, max_unread_bytes: usize) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            processes: Arc::new(Mutex::new(BTreeMap::new())),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            starting: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            uncertain: Arc::new(AtomicBool::new(false)),
            selected: Arc::new(Mutex::new(None)),
            max_count: max_count.max(1),
            max_unread_bytes: max_unread_bytes.max(1024),
            events,
        }
    }
    pub fn metadata(&self) -> Result<Vec<TerminalMetadata>, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        map.iter_mut()
            .map(|(id, p)| {
                let status = p
                    .child
                    .try_wait()
                    .map_err(failed)?
                    .map_or_else(|| "running".into(), |s| format!("exited: {s:?}"));
                let capture = p.output.lock().map_err(failed)?;
                let unread = capture
                    .bytes
                    .len()
                    .saturating_sub(p.cursor.saturating_sub(capture.base));
                Ok(TerminalMetadata {
                    id: *id,
                    name: p.name.clone(),
                    command: p.command.clone(),
                    cwd: p.cwd.clone(),
                    rows: p.rows,
                    cols: p.cols,
                    state: status,
                    unread_bytes: unread,
                })
            })
            .collect()
    }
    fn resolve(&self, id: Option<Uuid>, name: Option<&str>) -> Result<Uuid, ToolError> {
        if let Some(id) = id {
            return Ok(id);
        }
        let map = self.processes.lock().map_err(failed)?;
        if let Some(name) = name {
            return map
                .iter()
                .find(|(_, p)| p.name.as_deref() == Some(name))
                .map(|(id, _)| *id)
                .ok_or_else(|| ToolError::Failed(format!("unknown terminal name `{name}`")));
        }
        self.selected
            .lock()
            .map_err(failed)?
            .ok_or_else(|| ToolError::Failed("no selected terminal".into()))
    }
    #[allow(clippy::too_many_arguments)]
    fn start(
        &self,
        command: String,
        name: Option<String>,
        cwd: Option<std::path::PathBuf>,
        environment: BTreeMap<String, String>,
        rows: u16,
        cols: u16,
        ctx: &ToolContext,
    ) -> Result<String, ToolError> {
        self.start_after_spawn(command, name, cwd, environment, rows, cols, ctx, || Ok(()))
    }
    #[allow(clippy::too_many_arguments)]
    fn start_after_spawn(
        &self,
        command: String,
        name: Option<String>,
        cwd: Option<std::path::PathBuf>,
        environment: BTreeMap<String, String>,
        rows: u16,
        cols: u16,
        ctx: &ToolContext,
        after_spawn: impl FnOnce() -> Result<(), ToolError>,
    ) -> Result<String, ToolError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(ToolError::Failed(
                "terminal manager is shutting down".into(),
            ));
        }
        let _starting = self.starting.lock().map_err(failed)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(ToolError::Failed(
                "terminal manager is shutting down".into(),
            ));
        }
        {
            let map = self.processes.lock().map_err(failed)?;
            if map
                .len()
                .saturating_add(self.pending.lock().map_err(failed)?.len())
                >= self.max_count
            {
                return Err(ToolError::Failed(format!(
                    "terminal limit {} reached",
                    self.max_count
                )));
            }
            if let Some(ref name) = name
                && map.values().any(|p| p.name.as_ref() == Some(name))
            {
                return Err(ToolError::InvalidArguments(format!(
                    "terminal name `{name}` already exists"
                )));
            }
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(failed)?;
        let mut builder = platform_command(&command);
        builder.env_clear();
        let cwd = ctx
            .policy
            .resolve_read(cwd.as_deref().unwrap_or(ctx.policy.workspace()))
            .map_err(failed)?;
        if !cwd.is_dir() {
            return Err(ToolError::InvalidArguments(
                "terminal cwd must be a directory".into(),
            ));
        }
        builder.cwd(&cwd);
        for (key, value) in &ctx.environment {
            builder.env(key, value);
        }
        for (key, value) in environment {
            builder.env(key, value);
        }
        let child = StartupChild::new(self, pair.slave.spawn_command(builder).map_err(failed)?);
        after_spawn()?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(failed)?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(failed)?));
        let output = Arc::new(Mutex::new(Capture::new(rows, cols)));
        let max_unread_bytes = self.max_unread_bytes;
        let sink = output.clone();
        let id = child.id();
        let reader_done = child.reader_done();
        reader_done.store(false, Ordering::Release);
        let reader_finished = reader_done.clone();
        let events = self.events.clone();
        let notification_pending = Arc::new(AtomicBool::new(false));
        let reader_pending = notification_pending.clone();
        let reader_writer = writer.clone();
        std::thread::Builder::new()
            .name("helm-pty-reader".into())
            .spawn(move || {
                let _done = ReaderDone(reader_finished);
                let mut buffer = [0_u8; 8192];
                loop {
                    match std::io::Read::read(&mut reader, &mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut capture = sink.lock().expect("PTY buffer poisoned");
                            capture.parser.process(&buffer[..n]);
                            let cursor_report = buffer[..n]
                                .windows(4)
                                .any(|window| window == b"\x1b[6n")
                                .then(|| {
                                    let (row, column) = capture.parser.screen().cursor_position();
                                    format!("\x1b[{};{}R", row + 1, column + 1)
                                });
                            capture.bytes.extend_from_slice(&buffer[..n]);
                            if capture.bytes.len() > max_unread_bytes {
                                let remove = capture.bytes.len() - max_unread_bytes;
                                capture.bytes.drain(..remove);
                                capture.base += remove;
                                capture.dropped += remove as u64;
                            }
                            if !reader_pending.swap(true, Ordering::AcqRel) {
                                let _ = events.send(TerminalEvent::Changed(TerminalId(id)));
                            }
                            drop(capture);
                            if let Some(report) = cursor_report
                                && let Ok(mut writer) = reader_writer.lock()
                            {
                                let _ = writer.write_all(report.as_bytes());
                                let _ = writer.flush();
                            }
                        }
                    }
                }
            })
            .map_err(|error| {
                reader_done.store(true, Ordering::Release);
                failed(error)
            })?;
        self.processes.lock().map_err(failed)?.insert(
            id,
            Managed {
                name,
                command,
                cwd,
                rows,
                cols,
                master: pair.master,
                child: child.take(),
                writer,
                output,
                notification_pending,
                cursor: 0,
            },
        );
        *self.selected.lock().map_err(failed)? = Some(id);
        let _ = self.events.send(TerminalEvent::Added(TerminalId(id)));
        Ok(format!("started PTY process {id}"))
    }
    fn read(&self, id: Uuid, max: usize) -> Result<String, ToolError> {
        let mut processes = self.processes.lock().map_err(failed)?;
        let process = processes
            .get_mut(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        let capture = process.output.lock().map_err(failed)?;
        let (result, cursor) = unread_chunk(&capture, process.cursor, max);
        process.cursor = cursor;
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
        let mut writer = p.writer.lock().map_err(failed)?;
        writer.write_all(data.as_bytes()).map_err(failed)?;
        writer.flush().map_err(failed)?;
        Ok(format!("wrote {} bytes", data.len()))
    }
    fn resize(&self, id: Uuid, rows: u16, cols: u16) -> Result<String, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        let p = map
            .get_mut(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        p.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(failed)?;
        p.output.lock().map_err(failed)?.parser.set_size(rows, cols);
        p.rows = rows;
        p.cols = cols;
        Ok(format!("resized to {cols}x{rows}"))
    }
    fn rename(&self, id: Uuid, name: Option<String>) -> Result<String, ToolError> {
        let mut map = self.processes.lock().map_err(failed)?;
        if let Some(ref name) = name
            && map
                .iter()
                .any(|(other, p)| *other != id && p.name.as_ref() == Some(name))
        {
            return Err(ToolError::InvalidArguments(
                "terminal name already exists".into(),
            ));
        }
        map.get_mut(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?
            .name = name;
        Ok(format!("renamed {id}"))
    }
    fn interrupt(&self, id: Uuid) -> Result<String, ToolError> {
        let map = self.processes.lock().map_err(failed)?;
        let p = map
            .get(&id)
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
        interrupt_process_group(p.child.process_id());
        Ok(format!("interrupted {id}"))
    }
    async fn terminate(&self, id: Uuid) -> Result<String, ToolError> {
        {
            let _starting = self.starting.lock().map_err(failed)?;
            let mut map = self.processes.lock().map_err(failed)?;
            let mut pending = self.pending.lock().map_err(failed)?;
            let mut process = map
                .remove(&id)
                .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))?;
            process.child.stop_best_effort();
            pending.insert(id, process.child);
            let _ = self.events.send(TerminalEvent::Removed(TerminalId(id)));
        }
        self.observe_closed(id, std::time::Duration::from_secs(1))
            .await;
        Ok(format!("termination requested for {id}"))
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
            rows.push(format!(
                "{id}\t{state}\tname={}\tcwd={}\tsize={}x{}\tcommand={}",
                p.name.as_deref().unwrap_or("-"),
                p.cwd.display(),
                p.cols,
                p.rows,
                p.command
            ));
        }
        Ok(rows.join("\n"))
    }
}

#[async_trait]
impl InteractiveTerminals for ProcessTool {
    async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError> {
        self.metadata().map_err(terminal_error).map(|items| {
            items
                .into_iter()
                .map(|item| TerminalSummary {
                    id: TerminalId(item.id),
                    title: item.name.unwrap_or(item.command),
                    state: if item.state == "running" {
                        UiState::Running
                    } else {
                        UiState::Exited { code: None }
                    },
                })
                .collect()
        })
    }
    async fn snapshot(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        let mut map = self
            .processes
            .lock()
            .map_err(|e| TerminalError::Failed(e.to_string()))?;
        let p = map.get_mut(&id.0).ok_or(TerminalError::NotFound(id))?;
        let state = p
            .child
            .try_wait()
            .map_err(|e| TerminalError::Failed(e.to_string()))?
            .map_or(UiState::Running, |_| UiState::Exited { code: None });
        let capture = p
            .output
            .lock()
            .map_err(|e| TerminalError::Failed(e.to_string()))?;
        p.notification_pending.store(false, Ordering::Release);
        let screen = capture.parser.screen();
        let cells = (0..p.rows)
            .map(|row| {
                (0..p.cols)
                    .map(|col| {
                        screen
                            .cell(row, col)
                            .map_or_else(TerminalCell::default, |cell| TerminalCell {
                                text: cell.contents(),
                                foreground: color(cell.fgcolor()),
                                background: color(cell.bgcolor()),
                                bold: cell.bold(),
                                dim: false,
                                italic: cell.italic(),
                                underlined: cell.underline(),
                                reversed: cell.inverse(),
                            })
                    })
                    .collect()
            })
            .collect();
        Ok(TerminalSnapshot {
            id,
            title: p.name.clone().unwrap_or_else(|| p.command.clone()),
            state,
            revision: (capture.base + capture.bytes.len()) as u64,
            cells,
            cursor: Some({
                let (row, column) = screen.cursor_position();
                (column, row)
            }),
            dropped_unread_bytes: capture.dropped,
        })
    }
    async fn write(&self, id: TerminalId, bytes: Vec<u8>) -> Result<(), TerminalError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(TerminalError::Failed(
                "terminal manager is shutting down".into(),
            ));
        }
        let mut map = self
            .processes
            .lock()
            .map_err(|e| TerminalError::Failed(e.to_string()))?;
        let p = map.get_mut(&id.0).ok_or(TerminalError::NotFound(id))?;
        let mut writer = p
            .writer
            .lock()
            .map_err(|e| TerminalError::Failed(e.to_string()))?;
        writer
            .write_all(&bytes)
            .and_then(|_| writer.flush())
            .map_err(|e| TerminalError::Failed(e.to_string()))
    }
    async fn resize(&self, id: TerminalId, columns: u16, rows: u16) -> Result<(), TerminalError> {
        self.resize(id.0, rows, columns)
            .map(|_| ())
            .map_err(terminal_error)
    }
    fn subscribe(&self) -> broadcast::Receiver<TerminalEvent> {
        self.events.subscribe()
    }
}
fn terminal_error(error: ToolError) -> TerminalError {
    TerminalError::Failed(error.to_string())
}
fn color(value: vt100::Color) -> TerminalColor {
    match value {
        vt100::Color::Default => TerminalColor::Default,
        vt100::Color::Idx(value) => TerminalColor::Indexed(value),
        vt100::Color::Rgb(r, g, b) => TerminalColor::Rgb(r, g, b),
    }
}
#[cfg(unix)]
fn platform_command(command: &str) -> CommandBuilder {
    let mut builder = CommandBuilder::new("/bin/sh");
    builder.arg("-lc");
    builder.arg(command);
    builder
}
#[cfg(windows)]
fn platform_command(command: &str) -> CommandBuilder {
    let mut builder = CommandBuilder::new("cmd.exe");
    builder.arg("/D");
    builder.arg("/S");
    builder.arg("/C");
    builder.arg(command);
    builder
}
#[cfg(not(any(unix, windows)))]
fn platform_command(command: &str) -> CommandBuilder {
    let mut builder = CommandBuilder::new("sh");
    builder.arg("-lc");
    builder.arg(command);
    builder
}
fn unread_chunk(capture: &Capture, cursor: usize, max: usize) -> (String, usize) {
    let offset = cursor.saturating_sub(capture.base).min(capture.bytes.len());
    let end = offset.saturating_add(max).min(capture.bytes.len());
    let remaining = capture.bytes.len() - end;
    let mut result = String::from_utf8_lossy(&capture.bytes[offset..end]).into_owned();
    if remaining > 0 {
        result.push_str(&format!("\n\n[{remaining} unread bytes remain]"));
    }
    (result, capture.base + end)
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

#[cfg(unix)]
fn terminate_process_group(process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        // portable-pty starts the child in its own process group on Unix.
        unsafe {
            libc::kill(-(process_id as i32), libc::SIGKILL);
        }
    }
}
#[cfg(unix)]
fn interrupt_process_group(process_id: Option<u32>) {
    if let Some(id) = process_id {
        unsafe {
            libc::kill(-(id as i32), libc::SIGINT);
        }
    }
}
#[cfg(not(unix))]
fn interrupt_process_group(_: Option<u32>) {}
#[cfg(not(unix))]
fn terminate_process_group(_: Option<u32>) {}

#[cfg(test)]
mod tests {
    mod shutdown;
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
        async fn approve(
            &self,
            _: &crate::tools::ApprovalRequest,
        ) -> crate::tools::ApprovalOutcome {
            crate::tools::ApprovalOutcome::Approved
        }
    }
    pub(super) fn context(root: &std::path::Path) -> ToolContext {
        let config = Config {
            approval: ApprovalMode::Never,
            ..Config::default()
        };
        let mut environment = BTreeMap::new();
        if let Ok(path) = std::env::var("PATH") {
            environment.insert("PATH".into(), path);
        }
        ToolContext {
            completion: None,
            policy: Arc::new(Policy::new(&config, root.to_owned()).unwrap()),
            approver: Arc::new(Yes),
            timeout: Duration::from_secs(2),
            max_output_bytes: 4096,
            environment,
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Attended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        }
    }
    #[tokio::test]
    async fn pty_process_accepts_incremental_io() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        let started = tool
            .execute(
                json!({"action":"start","command":interactive_command()}),
                &ctx,
            )
            .await
            .unwrap();
        let id = Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap();

        let mut first = String::new();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
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
        assert!(
            first.contains("ready"),
            "interactive PTY did not become ready: {first:?}"
        );

        tool.execute(
            json!({"action":"write","id":id,"data":interactive_input()}),
            &ctx,
        )
        .await
        .unwrap();
        let mut second = String::new();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            second.push_str(
                &tool
                    .execute(json!({"action":"read","id":id}), &ctx)
                    .await
                    .unwrap(),
            );
            if second.contains(interactive_response()) {
                break;
            }
        }
        assert!(
            second.contains(interactive_response()),
            "interactive PTY response was not captured: {second:?}"
        );
        tool.execute(json!({"action":"terminate","id":id}), &ctx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn live_process_metadata_does_not_deadlock() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        let started = tool
            .execute(
                json!({"action":"start","command":interactive_command()}),
                &ctx,
            )
            .await
            .unwrap();
        let id = Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap();

        let metadata = tokio::time::timeout(Duration::from_secs(1), async { tool.metadata() })
            .await
            .expect("metadata must not deadlock")
            .unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].id, id);

        tool.execute(json!({"action":"terminate","id":id}), &ctx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dropping_manager_with_live_terminal_does_not_hang() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        tool.execute(
            json!({"action":"start","command":long_running_command()}),
            &ctx,
        )
        .await
        .unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(tool);
            let _ = sent.send(());
        });
        assert!(
            received.recv_timeout(Duration::from_secs(2)).is_ok(),
            "dropping the last terminal manager reference hung"
        );
    }

    #[tokio::test]
    async fn output_notifications_are_coalesced_until_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        let mut events = tool.subscribe();
        let started = tool
            .execute(json!({"action":"start","command":burst_command()}), &ctx)
            .await
            .unwrap();
        let id = TerminalId(Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap());

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(events.recv().await, Ok(TerminalEvent::Changed(_))) {
                    break;
                }
            }
        })
        .await
        .expect("burst must produce an output notification");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut additional_changed = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event, TerminalEvent::Changed(_)) {
                additional_changed += 1
            }
        }
        assert_eq!(
            additional_changed, 0,
            "output events must be coalesced to keep TUI input responsive"
        );
        let _ = tool.snapshot(id).await.unwrap();
    }

    #[test]
    fn truncated_reads_preserve_the_unread_tail() {
        let capture = Capture {
            bytes: b"abcdefgh".to_vec(),
            base: 10,
            dropped: 10,
            parser: vt100::Parser::new(24, 80, 0),
        };
        let (first, cursor) = unread_chunk(&capture, 10, 3);
        assert!(first.starts_with("abc"));
        assert_eq!(cursor, 13);
        let (second, cursor) = unread_chunk(&capture, cursor, 3);
        assert!(second.starts_with("def"));
        assert_eq!(cursor, 16);
        let (third, cursor) = unread_chunk(&capture, cursor, 3);
        assert_eq!(third, "gh");
        assert_eq!(cursor, 18);
    }
    #[test]
    fn vt_parser_tracks_styles_cursor_and_gap_count() {
        let mut capture = Capture::new(2, 8);
        capture.parser.process(b"\x1b[31;1mred\x1b[0m\r\nnext");
        capture.dropped = 17;
        let screen = capture.parser.screen();
        let cell = screen.cell(0, 0).unwrap();
        assert_eq!(cell.contents(), "r");
        assert!(cell.bold());
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
        assert_eq!(screen.cursor_position(), (1, 4));
        assert_eq!(capture.dropped, 17);
    }
    #[cfg(unix)]
    fn interactive_command() -> &'static str {
        "printf ready; read line; printf 'got:%s' \"$line\""
    }
    #[cfg(windows)]
    fn interactive_command() -> &'static str {
        "echo ready&& set /p line="
    }
    #[cfg(not(windows))]
    fn interactive_input() -> &'static str {
        "hello\n"
    }
    #[cfg(windows)]
    fn interactive_input() -> &'static str {
        "hello\r\n"
    }
    #[cfg(not(windows))]
    fn interactive_response() -> &'static str {
        "got:hello"
    }
    #[cfg(windows)]
    fn interactive_response() -> &'static str {
        // ConPTY echoes console input, proving the bytes reached the live shell.
        "hello"
    }
    #[cfg(unix)]
    fn long_running_command() -> &'static str {
        "sleep 300 & wait"
    }
    #[cfg(windows)]
    fn long_running_command() -> &'static str {
        "ping -n 300 127.0.0.1 >NUL"
    }
    #[cfg(unix)]
    fn burst_command() -> &'static str {
        "i=0; while [ $i -lt 10000 ]; do echo x; i=$((i+1)); done; sleep 1"
    }
    #[cfg(windows)]
    fn burst_command() -> &'static str {
        "for /L %i in (1,1,10000) do @echo x & ping -n 2 127.0.0.1 >NUL"
    }
}
