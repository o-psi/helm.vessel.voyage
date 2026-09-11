mod input;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod isolated;
mod shutdown;
use super::{Tool, ToolContext, ToolError};
use crate::terminal::{
    InteractiveTerminals, TerminalCell, TerminalColor, TerminalError, TerminalEvent, TerminalId,
    TerminalModes, TerminalSnapshot, TerminalState as UiState, TerminalSummary,
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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct ProcessTool {
    origin_policies: Arc<Mutex<Vec<Arc<crate::policy::Policy>>>>,
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
impl ProcessTool {
    pub(crate) fn same_manager(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.processes, &other.processes)
    }
    pub(crate) fn retain_policy(&self, policy: Arc<crate::policy::Policy>) -> anyhow::Result<()> {
        let mut policies = self
            .origin_policies
            .lock()
            .map_err(|_| anyhow::anyhow!("terminal origins poisoned"))?;
        anyhow::ensure!(policies.len() < 256, "terminal origin capacity reached");
        policies.push(policy);
        Ok(())
    }
}

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
    writer: input::Input,
    output: Arc<Mutex<Capture>>,
    notification_pending: Arc<AtomicBool>,
    cursor: usize,
}

struct Capture {
    privacy: Option<crate::terminal::TerminalPrivacy>,
    bytes: Vec<u8>,
    base: usize,
    dropped: u64,
    parser: vt100::Parser,
    revision: u64,
}
impl Capture {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            privacy: None,
            bytes: Vec::new(),
            base: 0,
            dropped: 0,
            parser: vt100::Parser::new(rows, cols, 0),
            revision: 0,
        }
    }
    fn make_private(&mut self, cursor: usize) {
        if self.privacy.is_none() {
            let offset = cursor.saturating_sub(self.base).min(self.bytes.len());
            self.privacy = Some(crate::terminal::TerminalPrivacy {
                discarded_unread_bytes: (self.bytes.len() - offset) as u64,
                suppressed_output_bytes: 0,
            });
            self.base += self.bytes.len();
            self.bytes.clear();
        }
    }
    fn resize(&mut self, rows: u16, columns: u16) {
        self.parser.set_size(rows, columns);
        self.revision = self.revision.saturating_add(1);
    }
    fn capture_output(&mut self, bytes: &[u8], max_unread_bytes: usize) {
        self.parser.process(bytes);
        self.revision = self.revision.saturating_add(1);
        if let Some(privacy) = &mut self.privacy {
            privacy.suppressed_output_bytes = privacy
                .suppressed_output_bytes
                .saturating_add(bytes.len() as u64);
            self.base = self.base.saturating_add(bytes.len());
        } else {
            self.bytes.extend_from_slice(bytes);
            if self.bytes.len() > max_unread_bytes {
                let remove = self.bytes.len() - max_unread_bytes;
                self.bytes.drain(..remove);
                self.base += remove;
                self.dropped += remove as u64;
            }
        }
    }
}
const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
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
            output_schema: None,
            annotations: None,
        name: "process".into(),
        description: "Manage multiple persistent PTY-backed terminals with stable IDs and optional names, cwd, and environment. Start, read, write, resize, interrupt, rename, list, or terminate. Human attachment permanently disables model capture and input for that terminal; reads report a privacy gap. Start a new terminal for model-observed work. When a human using Helm's full-screen interface needs this program, tell them to press F3, select its name, and press Enter. Ctrl+] returns to Helm. Do not imply that a separate terminal window has opened. Passwords belong only in that private terminal, never in chat. Use shell for isolated one-shot commands.".into(),
        input_schema: super::action_schema::schema(json!({
            "command":{"type":"string"},"id":{"type":["string","null"],"format":"uuid"},
            "name":{"type":["string","null"]},"current_name":{"type":["string","null"]},
            "cwd":{"type":["string","null"]},"env":{"type":"object","additionalProperties":{"type":"string"}},
            "data":{"type":"string"},"rows":{"type":"integer","minimum":1,"maximum":120},"cols":{"type":"integer","minimum":1,"maximum":240}
        }), &[], &[
            ("start", &["command"], &["name","cwd","env","rows","cols"]),
            ("read", &[], &["id","name"]), ("write", &["data"], &["id","name"]),
            ("resize", &["rows","cols"], &["id","name"]), ("interrupt", &[], &["id","name"]),
            ("terminate", &[], &["id","name"]), ("select", &[], &["id","name"]),
            ("rename", &[], &["id","current_name","name"]), ("list", &[], &[]),
        ]),
    }
    }

    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        for policy in self.origin_policies.lock().map_err(failed)?.iter() {
            policy.check_current().map_err(failed)?;
        }
        let args: Args = serde_json::from_value(value.clone())
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        super::action_schema::reject_extra_fields(&value, &args)?;
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
                #[cfg(windows)]
                return Err(ToolError::Denied("CMD terminal command policy analysis is not supported; POSIX syntax analysis cannot authorize cmd.exe".into()));
                #[cfg(not(windows))]
                match ctx.policy.command(&command) {
                    Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
                    Decision::Ask(reason) => ctx
                        .approver
                        .approve(&ctx.approval("process.start", &command, reason.clone()))
                        .await
                        .require_approved()?,
                    _ => {}
                }
                self.start(command, name, cwd, env, rows, cols, ctx)
            }
            Args::Read { id, name } => {
                self.read(self.resolve(id, name.as_deref())?, ctx.max_output_bytes)
            }
            Args::Write { id, name, data } => {
                let writer = self.input(self.resolve(id, name.as_deref())?)?;
                let count = data.len();
                tokio::select! { biased;
                    _ = ctx.cancellation.cancelled() => Err(ToolError::Failed("terminal input cancelled; delivery may be partial".into())),
                    result = writer.write_model(data.into_bytes()) => result.map(|()|format!("wrote {count} bytes")).map_err(failed),
                }
            }
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
    /// Conservative nonmutating handoff check. Even an exited terminal must be
    /// explicitly closed so its readers and original process session are observed.
    pub fn has_owned_work(&self) -> bool {
        self.uncertain.load(Ordering::Acquire)
            || self.starting.try_lock().is_err()
            || self
                .processes
                .lock()
                .map_or(true, |items| !items.is_empty())
            || self.pending.lock().map_or(true, |items| !items.is_empty())
    }

    /// Only a detached, empty observer can retire from a resource registry.
    pub fn can_retire(&self) -> bool {
        Arc::strong_count(&self.starting) == 1 && !self.has_owned_work()
    }

    pub fn with_limits(max_count: usize, max_unread_bytes: usize) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            processes: Arc::new(Mutex::new(BTreeMap::new())),
            origin_policies: Arc::new(Mutex::new(Vec::new())),
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
        check_size(rows, cols)?;
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
        ctx.policy
            .check_execution_authority()
            .map_err(|_| ToolError::Denied("foreground execution authority unavailable".into()))?;
        let reservation =
            crate::host_resources::Reservation::acquire("terminals", ctx.execution_id, 1)
                .map_err(failed)?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let spawn_result = if ctx.policy.sandbox().required() {
            isolated::spawn(&*pair.master, &builder, &ctx.policy)
        } else {
            pair.slave.spawn_command(builder)
        };
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        let spawn_result = pair.slave.spawn_command(builder);
        let spawned = match spawn_result {
            Ok(child) => child,
            Err(error) => {
                reservation.release_observed().map_err(failed)?;
                return Err(failed(error));
            }
        };
        let child = StartupChild::new(self, spawned, reservation);
        after_spawn()?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(failed)?;
        #[cfg(unix)]
        input::make_nonblocking(&*pair.master).map_err(failed)?;
        let writer = input::Input::start(
            pair.master.take_writer().map_err(failed)?,
            child.input_stop(),
            child.input_done(),
        )
        .map_err(failed)?;
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
                        Ok(0) => break,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            continue;
                        }
                        Err(_) => break,
                        Ok(n) => {
                            let mut capture = sink.lock().expect("PTY buffer poisoned");
                            capture.capture_output(&buffer[..n], max_unread_bytes);
                            let cursor_report = buffer[..n]
                                .windows(4)
                                .any(|window| window == b"\x1b[6n")
                                .then(|| {
                                    let (row, column) = capture.parser.screen().cursor_position();
                                    format!("\x1b[{};{}R", row + 1, column + 1)
                                });
                            if !reader_pending.swap(true, Ordering::AcqRel) {
                                let _ = events.send(TerminalEvent::Changed(TerminalId(id)));
                            }
                            drop(capture);
                            if let Some(report) = cursor_report {
                                reader_writer.report(report.into_bytes());
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
    fn input(&self, id: Uuid) -> Result<input::Input, ToolError> {
        self.processes
            .lock()
            .map_err(failed)?
            .get(&id)
            .map(|process| process.writer.clone())
            .ok_or_else(|| ToolError::Failed(format!("unknown process {id}")))
    }
    async fn human_input(&self, id: TerminalId) -> Result<input::Input, TerminalError> {
        let writer = {
            let map = self
                .processes
                .lock()
                .map_err(|_| TerminalError::Failed("terminal manager unavailable".into()))?;
            let process = map.get(&id.0).ok_or(TerminalError::NotFound(id))?;
            let mut capture = process
                .output
                .lock()
                .map_err(|_| TerminalError::Failed("terminal capture unavailable".into()))?;
            let newly_private = capture.privacy.is_none();
            // Suppress capture before waiting: unconfirmed attach must not leak output.
            capture.make_private(process.cursor);
            if newly_private && !process.notification_pending.swap(true, Ordering::AcqRel) {
                let _ = self.events.send(TerminalEvent::Changed(id));
            }
            process.writer.clone()
        };
        writer
            .make_private()
            .await
            .map_err(|error| TerminalError::Failed(error.to_string()))?;
        Ok(writer)
    }
    fn resize(&self, id: Uuid, rows: u16, cols: u16) -> Result<String, ToolError> {
        check_size(rows, cols)?;
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
        let mut capture = p.output.lock().map_err(failed)?;
        capture.resize(rows, cols);
        if !p.notification_pending.swap(true, Ordering::AcqRel) {
            let _ = self.events.send(TerminalEvent::Changed(TerminalId(id)));
        }
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
    async fn attach(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        self.human_input(id).await?;
        self.snapshot(id).await
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
        let cells = screen_cells(screen);
        Ok(TerminalSnapshot {
            id,
            title: p.name.clone().unwrap_or_else(|| p.command.clone()),
            state,
            revision: capture.revision,
            cells,
            modes: TerminalModes {
                app_cursor: screen.application_cursor(),
                app_keypad: screen.application_keypad(),
                bracketed_paste: screen.bracketed_paste(),
            },
            cursor: (!screen.hide_cursor()).then(|| {
                let (row, column) = screen.cursor_position();
                (column, row)
            }),
            dropped_unread_bytes: capture.dropped,
            privacy: capture.privacy.clone(),
        })
    }
    async fn write(&self, id: TerminalId, bytes: Vec<u8>) -> Result<(), TerminalError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(TerminalError::Failed(
                "terminal manager is shutting down".into(),
            ));
        }
        let writer = self.human_input(id).await?;
        writer
            .write(bytes)
            .await
            .map_err(|error| TerminalError::Failed(error.to_string()))
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
fn screen_cells(screen: &vt100::Screen) -> Vec<Vec<TerminalCell>> {
    let (rows, columns) = screen.size();
    (0..rows)
        .map(|row| {
            (0..columns)
                .map(|col| {
                    screen
                        .cell(row, col)
                        .map_or_else(TerminalCell::default, |cell| TerminalCell {
                            text: if cell.is_wide_continuation() {
                                String::new()
                            } else {
                                let text = cell.contents();
                                safe_cell_text(&text)
                            },
                            width: if cell.is_wide_continuation() {
                                0
                            } else if cell.is_wide() {
                                2
                            } else {
                                1
                            },
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
        .collect()
}
fn check_size(rows: u16, columns: u16) -> Result<(), ToolError> {
    use voyage_protocol::terminal::{MAX_COLUMNS, MAX_ROWS};
    if !(1..=MAX_COLUMNS).contains(&columns) || !(1..=MAX_ROWS).contains(&rows) {
        return Err(ToolError::InvalidArguments(
            "terminal size must be 1..240 columns by 1..120 rows".into(),
        ));
    }
    Ok(())
}
fn safe_cell_text(text: &str) -> String {
    use voyage_protocol::terminal::{MAX_CELL_BYTES, safe_cell_char};
    let mut safe = String::new();
    for ch in text.chars().filter(|ch| safe_cell_char(*ch)) {
        if safe.len() + ch.len_utf8() > MAX_CELL_BYTES {
            break;
        }
        safe.push(ch);
    }
    if safe.is_empty() {
        safe.push(' ');
    }
    safe
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
    if capture.privacy.is_some() {
        return ("[model capture unavailable: human attachment made this terminal private; output remains withheld after detach. Start a new terminal for model-observed work.]".into(), capture.base);
    }
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
mod schema_tests {
    use super::*;
    #[test]
    fn process_schema_is_action_specific() {
        let schema = ProcessTool::default().definition().input_schema;
        let compiled = crate::tools::schema::CompiledSchema::compile(&schema).unwrap();
        for value in [
            json!({"action":"start","command":"pwd","cwd":null}),
            json!({"action":"resize","rows":24,"cols":80}),
            json!({"action":"write","data":"text"}),
            json!({"action":"rename","name":null}),
        ] {
            compiled.validate(&value).unwrap();
            serde_json::from_value::<Args>(value).unwrap();
        }
        for value in [
            json!({"action":"read","data":"wrong"}),
            json!({"action":"start"}),
            json!({"action":"resize","rows":65536,"cols":80}),
            json!({"action":"start","command":"pwd","env":{"KEY":1}}),
        ] {
            assert!(compiled.validate(&value).is_err());
        }
    }
}

#[cfg(test)]
mod screen_tests {
    use super::*;
    #[test]
    fn emulated_screen_preserves_styles_wide_cells_combining_modes_and_cursor() {
        let mut capture = Capture::new(3, 12);
        capture.capture_output(
            "\x1b[1;3;4;7;38;5;123;48;2;1;2;3m界e\u{301}\x1b[?1h\x1b=\x1b[?2004h\x1b[?25l"
                .as_bytes(),
            1024,
        );
        let screen = capture.parser.screen();
        let cells = screen_cells(screen);
        assert_eq!(cells[0][0].text, "界");
        assert_eq!(cells[0][0].width, 2);
        assert_eq!(cells[0][1].text, "");
        assert_eq!(cells[0][1].width, 0);
        assert_eq!(cells[0][2].text, "e\u{301}");
        assert_eq!(cells[0][0].foreground, TerminalColor::Indexed(123));
        assert_eq!(cells[0][0].background, TerminalColor::Rgb(1, 2, 3));
        assert!(
            cells[0][0].bold
                && cells[0][0].italic
                && cells[0][0].underlined
                && cells[0][0].reversed
        );
        assert!(screen.hide_cursor());
        let modes = TerminalModes {
            app_cursor: screen.application_cursor(),
            app_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
        };
        assert!(modes.app_cursor && modes.app_keypad && modes.bracketed_paste);
        crate::terminal::TerminalScreen {
            version: 1,
            columns: 12,
            height: 3,
            cells,
            modes,
        }
        .validate()
        .unwrap();
        capture.capture_output(b"\x1b[?1049hALT\x1b[?1049l\x1b[?25h", 1024);
        assert!(!capture.parser.screen().hide_cursor());
        assert_eq!(screen_cells(capture.parser.screen())[0][0].text, "界");
    }
    #[test]
    fn private_output_and_resize_advance_screen_revision_without_model_capture() {
        let mut capture = Capture::new(2, 4);
        capture.capture_output(b"old", 1024);
        let revision = capture.revision;
        capture.make_private(0);
        capture.capture_output(b"secret", 1024);
        assert!(capture.revision > revision);
        let revision = capture.revision;
        capture.resize(3, 5);
        assert!(capture.revision > revision);
        assert_eq!(capture.parser.screen().size(), (3, 5));
        assert!(capture.bytes.is_empty());
        let (read, _) = unread_chunk(&capture, 0, 1024);
        assert!(!read.contains("secret"));
        assert_eq!(capture.privacy.as_ref().unwrap().suppressed_output_bytes, 6);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn native_echo_and_repeat_stay_out_of_private_model_capture() {
        use std::{io::Read, time::Duration};
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 8,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        input::make_nonblocking(&*pair.master).unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.arg("-c");
        command.arg(
            "stty echo; printf 'ready\\n'; IFS= read -r secret; printf 'repeat:%s\\n' \"$secret\"",
        );
        struct ChildGuard(Box<dyn Child + Send + Sync>);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let _child = ChildGuard(pair.slave.spawn_command(command).unwrap());
        let mut reader = pair.master.try_clone_reader().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let writer = input::Input::start(
            pair.master.take_writer().unwrap(),
            stop.clone(),
            done.clone(),
        )
        .unwrap();
        struct Stop(Arc<AtomicBool>);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let stopper = Stop(stop);
        let mut capture = Capture::new(8, 80);
        async fn receive(reader: &mut dyn Read, capture: &mut Capture, marker: &str) {
            tokio::time::timeout(Duration::from_secs(3), async {
                let mut bytes = [0; 1024];
                while !capture.parser.screen().contents().contains(marker) {
                    match reader.read(&mut bytes) {
                        Ok(n) if n > 0 => capture.capture_output(&bytes[..n], 4096),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => (),
                        result => panic!("native PTY ended before marker: {result:?}"),
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
        receive(&mut *reader, &mut capture, "ready").await;
        capture.make_private(0);
        writer.make_private().await.unwrap();
        writer
            .write(b"synthetic-private-value\n".to_vec())
            .await
            .unwrap();
        receive(&mut *reader, &mut capture, "repeat:synthetic-private-value").await;
        assert!(
            capture
                .parser
                .screen()
                .contents()
                .matches("synthetic-private-value")
                .count()
                >= 2
        );
        assert!(capture.bytes.is_empty());
        assert!(
            !unread_chunk(&capture, 0, 4096)
                .0
                .contains("synthetic-private-value")
        );
        assert_eq!(
            writer.write_model(b"blocked".to_vec()).await,
            Err(input::InputError::Private)
        );
        drop(stopper);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !done.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    #[test]
    fn bounds_and_cell_sanitization_are_utf8_safe() {
        assert!(check_size(120, 240).is_ok());
        for (rows, cols) in [(0, 80), (24, 0), (121, 80), (24, 241)] {
            assert!(check_size(rows, cols).is_err());
        }
        let text = safe_cell_text(&format!("\x1b\u{202e}{}", "界".repeat(30)));
        assert_eq!(text.len(), 63);
        assert!(text.chars().all(voyage_protocol::terminal::safe_cell_char));
        assert_eq!(safe_cell_text("\u{2066}"), " ");
    }
}
