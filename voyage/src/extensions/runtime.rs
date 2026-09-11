//! Executing-Voyage integration. Registry construction never launches code.
use super::catalog::{Catalog, ExecutableSnapshot};
use crate::{
    extension_sdk as sdk,
    tools::{Tool, ToolContext, ToolError},
};
use anyhow::{Result, ensure};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Manager {
    state: Mutex<State>,
    read_allowed: AtomicBool,
    private_files: Mutex<Vec<super::PrivateFile>>,
}
#[derive(Default)]
struct State {
    closed: bool,
    executors: Vec<Arc<sdk::Executor>>,
    children: Vec<Arc<OwnedChild>>,
}
struct OwnedRead {
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<String>>>>,
}
impl OwnedRead {
    async fn take(&self) -> Result<String> {
        let mut task = self.task.lock().await;
        let result = task
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("host read already drained"))?
            .await;
        *task = None;
        result.map_err(|_| anyhow::anyhow!("host read worker interrupted"))?
    }
    async fn drain(&self) {
        let mut task = self.task.lock().await;
        if let Some(handle) = task.as_mut() {
            let _ = handle.await;
        }
        *task = None;
    }
}
struct OwnedChild {
    invocation: Uuid,
    reads: Mutex<Vec<Arc<OwnedRead>>>,
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    #[cfg(target_os = "linux")]
    identity: Option<Arc<crate::tools::process::SessionIdentity>>,
    reservation: crate::host_resources::extensions::ExtensionReservation,
    observed: AtomicBool,
    process_observed: AtomicBool,
    observation: tokio::sync::Mutex<Option<tokio::task::JoinHandle<bool>>>,
}
impl OwnedChild {
    fn no_child(reservation: crate::host_resources::extensions::ExtensionReservation) -> Arc<Self> {
        Arc::new(Self {
            invocation: Uuid::nil(),
            reads: Mutex::new(Vec::new()),
            child: tokio::sync::Mutex::new(None),
            #[cfg(target_os = "linux")]
            identity: None,
            reservation,
            observed: AtomicBool::new(false),
            process_observed: AtomicBool::new(true),
            observation: tokio::sync::Mutex::new(None),
        })
    }
    async fn observe(self: &Arc<Self>) -> bool {
        if self.observed.load(Ordering::Acquire) {
            return true;
        }
        // Retain an in-flight observer through caller cancellation/timeouts. A
        // retry waits for the same observer rather than spawning duplicate kills.
        let mut observer = self.observation.lock().await;
        if observer.is_none() {
            let owned = self.clone();
            *observer = Some(tokio::spawn(async move {
                let mut child = owned.child.lock().await;
                if !owned.process_observed.load(Ordering::Acquire) {
                    let Some(child) = child.as_mut() else {
                        return false;
                    };
                    #[cfg(target_os = "linux")]
                    let identity = owned.identity.clone().or_else(|| {
                        child
                            .id()
                            .and_then(|pid| {
                                crate::tools::process::SessionIdentity::capture(pid).ok()
                            })
                            .map(Arc::new)
                    });
                    #[cfg(target_os = "linux")]
                    let empty = if let Some(identity) = identity {
                        tokio::task::spawn_blocking(move || {
                            let deadline = std::time::Instant::now() + Duration::from_secs(4);
                            while std::time::Instant::now() < deadline {
                                match identity.kill_and_observe(deadline) {
                                    Ok(true) => return true,
                                    Ok(false) => std::thread::sleep(Duration::from_millis(5)),
                                    Err(_) => return false,
                                }
                            }
                            false
                        })
                        .await
                        .unwrap_or(false)
                    } else {
                        false
                    };
                    #[cfg(not(target_os = "linux"))]
                    let empty = false;
                    if !empty
                        || !matches!(
                            tokio::time::timeout(Duration::from_millis(250), child.wait()).await,
                            Ok(Ok(_))
                        )
                    {
                        return false;
                    }
                    // Retain positive observation across a later ledger-write
                    // failure; never try to recapture a PID we already reaped.
                    owned.process_observed.store(true, Ordering::Release);
                }
                let reads = match owned.reads.lock() {
                    Ok(reads) => reads.clone(),
                    Err(_) => return false,
                };
                // A cancelled broker future must not let package replacement
                // race a still-running blocking filesystem worker.
                futures_util::future::join_all(reads.iter().map(|read| read.drain())).await;
                if owned.reservation.release_observed().is_err() {
                    return false;
                }
                owned.observed.store(true, Ordering::Release);
                true
            }));
        }
        let result = observer
            .as_mut()
            .expect("owned observer")
            .await
            .unwrap_or(false);
        *observer = None;
        result
    }
}
struct ChildLease(Arc<OwnedChild>);
#[async_trait]
impl sdk::Lease for ChildLease {
    async fn terminate_and_observe(&mut self) -> sdk::Cleanup {
        if self.0.observe().await {
            sdk::Cleanup::Observed
        } else {
            sdk::Cleanup::Pending
        }
    }
}
impl Manager {
    fn start_read(
        &self,
        invocation: Uuid,
        work: impl FnOnce() -> Result<String> + Send + 'static,
    ) -> Result<Arc<OwnedRead>> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension read owner unavailable"))?;
        ensure!(!state.closed, "extension read admission closed");
        let child = state
            .children
            .iter()
            .find(|child| child.invocation == invocation)
            .ok_or_else(|| anyhow::anyhow!("extension read invocation unowned"))?;
        let mut reads = child
            .reads
            .lock()
            .map_err(|_| anyhow::anyhow!("extension reads unavailable"))?;
        ensure!(
            reads.len() < 32 && !child.process_observed.load(Ordering::Acquire),
            "extension read admission closed or full"
        );
        let read = Arc::new(OwnedRead {
            task: tokio::sync::Mutex::new(Some(tokio::task::spawn_blocking(work))),
        });
        reads.push(read.clone());
        Ok(read)
    }
    pub(crate) fn restrict_host_read(&self, allowed: bool) {
        if !allowed {
            self.read_allowed.store(false, Ordering::Release);
        }
    }
    pub(crate) async fn shutdown(&self) -> Result<()> {
        let (executors, children) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("extension ownership unavailable"))?;
            state.closed = true;
            for executor in &state.executors {
                executor.close();
            }
            (state.executors.clone(), state.children.clone())
        };
        // Closed manager admission fences even queued SDK tasks before spawn.
        let (_, result) = tokio::join!(
            futures_util::future::join_all(executors.iter().map(|executor| executor.shutdown())),
            futures_util::future::join_all(children.iter().map(|child| child.observe()))
        );
        // This concrete adapter performs no detached launch effect: every child
        // is retained before returning, and refusal before spawn is observed.
        // Reconcile a prior SDK timeout only after BOTH actual task drain and
        // positive cleanup of every retained child; not merely a dropped waiter.
        ensure!(
            executors.iter().all(|executor| executor.tasks_drained())
                && result.into_iter().all(|observed| observed),
            "executable extension cleanup remains pending"
        );
        Ok(())
    }
    fn add_executor(&self, executor: Arc<sdk::Executor>) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension ownership unavailable"))?;
        ensure!(
            !state.closed && state.executors.len() < 128,
            "extension admission closed or full"
        );
        state.executors.push(executor);
        Ok(())
    }
}
struct Adapter {
    catalog: Arc<Catalog>,
    snapshot: Arc<ExecutableSnapshot>,
    manager: std::sync::Weak<Manager>,
    contexts: Arc<Mutex<BTreeMap<Uuid, (ToolContext, String)>>>,
}
#[async_trait]
impl sdk::LaunchAdapter for Adapter {
    async fn launch(&self, identity: &sdk::Identity, deadline: Instant) -> Result<sdk::Launched> {
        // No await between grant/policy admission, durable reservation and owned
        // spawn registration: cancellation cannot strand an untracked child.
        let (context, action) = self
            .contexts
            .lock()
            .map_err(|_| anyhow::anyhow!("extension context unavailable"))?
            .get(&identity.invocation)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("extension invocation context expired"))?;
        context.policy.check_current()?;
        ensure!(
            Instant::now() < deadline && !context.cancellation.is_cancelled(),
            "extension launch cancelled or expired"
        );
        let (session, incarnation) = crate::host_resources::process_scope()
            .ok_or_else(|| anyhow::anyhow!("supervised extension owner unavailable"))?;
        ensure!(
            identity.session == session
                && identity.incarnation == incarnation
                && identity.run == context.execution_id
                && identity.package == self.snapshot.archive.manifest.id
                && identity.digest == self.snapshot.digest,
            "extension invocation identity mismatch"
        );
        let image = crate::sandbox::ExtensionImage::seal(&self.snapshot.archive.executable()?)?;
        let mut command = std::process::Command::new("/extension");
        command
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Setsid precedes the SDK filter which prevents later session escape.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        context
            .policy
            .sandbox()
            .apply_extension(&mut command, &image)?;
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("extension resource owner retired"))?;
        let mut state = manager
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension ownership unavailable"))?;
        state
            .children
            .retain(|child| !child.observed.load(Ordering::Acquire));
        ensure!(
            !state.closed && state.children.len() < 128,
            "extension admission closed or full"
        );
        let reservation = self.catalog.admit_executable(
            &self.snapshot,
            identity.invocation,
            identity.run,
            &action,
        )?;
        if context.policy.check_current().is_err()
            || context.cancellation.is_cancelled()
            || Instant::now() >= deadline
        {
            state.children.push(OwnedChild::no_child(reservation)); // Retry durable release through owned cleanup.
            anyhow::bail!("extension launch authority changed");
        }
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                state.children.push(OwnedChild::no_child(reservation));
                anyhow::bail!("required executable isolation launch failed");
            }
        };
        #[cfg(target_os = "linux")]
        let process_identity = child
            .id()
            .and_then(|pid| crate::tools::process::SessionIdentity::capture(pid).ok())
            .map(Arc::new);
        let reader = child.stdout.take();
        let writer = child.stdin.take();
        let owned = Arc::new(OwnedChild {
            invocation: identity.invocation,
            reads: Mutex::new(Vec::new()),
            child: tokio::sync::Mutex::new(Some(child)),
            #[cfg(target_os = "linux")]
            identity: process_identity,
            reservation,
            observed: AtomicBool::new(false),
            process_observed: AtomicBool::new(false),
            observation: tokio::sync::Mutex::new(None),
        });
        state.children.push(owned.clone());
        let reader =
            reader.ok_or_else(|| anyhow::anyhow!("extension protocol output unavailable"))?;
        let writer =
            writer.ok_or_else(|| anyhow::anyhow!("extension protocol input unavailable"))?;
        Ok(sdk::Launched {
            reader: Box::new(reader),
            writer: Box::new(writer),
            lease: Box::new(ChildLease(owned)),
        })
    }
}
struct Broker {
    context: ToolContext,
    manager: Arc<Manager>,
    progress: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl sdk::Host for Broker {
    async fn read(
        &self,
        identity: &sdk::Identity,
        path: &str,
        offset: u64,
        max_bytes: usize,
        deadline: Instant,
    ) -> Result<String> {
        let context = &self.context;
        ensure!(
            self.manager.read_allowed.load(Ordering::Acquire),
            "extension host read excluded by tool ceiling"
        );
        ensure!(
            identity.run == context.execution_id
                && max_bytes > 0
                && max_bytes <= sdk::MAX_READ
                && path.len() <= 4096
                && offset == 0,
            "invalid extension host read"
        );
        ensure!(
            Instant::now() < deadline && !context.cancellation.is_cancelled(),
            "extension host read expired"
        );
        context.policy.check_current()?;
        let path = context.policy.resolve_read(std::path::Path::new(path))?;
        // A broad user root is not consent to disclose Voyage account/session
        // stores to an extension. These private interfaces are never brokered.
        let mut private_roots = vec![
            crate::config::default_data_dir(),
            crate::build::resource_root(),
        ];
        if let Some(config) = crate::config::default_config_path()
            && let Some(parent) = config.parent()
        {
            private_roots.push(parent.to_path_buf());
        }
        for private in private_roots {
            if private.try_exists()? {
                ensure!(
                    !path.starts_with(private.canonicalize()?),
                    "extension host read targets private runtime storage"
                );
            }
        }
        let private_files = self
            .manager
            .private_files
            .lock()
            .map_err(|_| anyhow::anyhow!("private file provenance unavailable"))?
            .clone();
        let policy = context.policy.clone();
        let read = self
            .manager
            .start_read(identity.invocation, move || -> Result<String> {
                use cap_fs_ext::{
                    DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt,
                };
                use std::io::{Read, Seek};
                policy.check_current()?;
                // Traverse the exact authorized canonical path descriptor-relatively,
                // refusing symlink replacement at every component and bounded I/O.
                let mut dir =
                    cap_std::fs::Dir::open_ambient_dir("/", cap_std::ambient_authority())?;
                let parent = path
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("invalid host read path"))?;
                for part in parent.components() {
                    match part {
                        std::path::Component::RootDir => {}
                        std::path::Component::Normal(part) => dir = dir.open_dir_nofollow(part)?,
                        _ => anyhow::bail!("invalid host read path"),
                    }
                }
                let mut options = cap_std::fs::OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No).nonblock(true);
                let mut file = dir.open_with(
                    path.file_name()
                        .ok_or_else(|| anyhow::anyhow!("invalid host read name"))?,
                    &options,
                )?;
                let metadata = file.metadata()?;
                ensure!(
                    metadata.is_file() && metadata.len() <= sdk::MAX_READ as u64,
                    "host file exceeds bounded read contract"
                );
                #[cfg(unix)]
                {
                    use cap_std::fs::MetadataExt;
                    ensure!(
                        metadata.nlink() == 1,
                        "hard-linked files are not brokered to executable extensions"
                    );
                }
                ensure!(
                    !private_files
                        .iter()
                        .any(|private| private.matches(&path, &metadata)),
                    "extension host read targets a private configuration source"
                );
                file.seek(std::io::SeekFrom::Start(offset))?;
                let mut bytes = Vec::new();
                file.take(max_bytes as u64 + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= max_bytes,
                    "host read exceeds requested bound"
                );
                policy.check_current()?;
                String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("host file is not UTF-8"))
            })?;
        let value = read.take().await?;
        context.policy.check_current()?;
        ensure!(
            Instant::now() < deadline && !context.cancellation.is_cancelled(),
            "extension host read expired"
        );
        Ok(context.redactor.redact(value))
    }
    async fn progress(&self, identity: &sdk::Identity, text: &str) -> Result<()> {
        ensure!(
            identity.run == self.context.execution_id && !self.context.cancellation.is_cancelled(),
            "extension progress expired"
        );
        self.context.policy.check_current()?;
        let mut progress = self
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("extension progress unavailable"))?;
        ensure!(
            progress.len() < 256
                && progress.iter().map(String::len).sum::<usize>() + text.len()
                    <= self.context.max_output_bytes.min(65536),
            "extension progress budget exceeded"
        );
        // Keep untrusted text private until the complete output passes the
        // existing split-text confidentiality checks. No subprocess diagnostics.
        progress.push(
            text.chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
                .collect(),
        );
        Ok(())
    }
}
/// Check decoded leaves across progress and structured output before JSON
/// punctuation can separate reconstructable fragments. Serialized/typed output
/// checks remain additional independent protections in the ordinary adapter.
fn confidential(
    redactor: &crate::tools::Redactor,
    progress: &[String],
    value: &Value,
) -> Result<()> {
    fn walk(value: &Value, out: &mut String) {
        match value {
            Value::String(text) => out.push_str(text),
            Value::Array(values) => {
                for value in values {
                    walk(value, out)
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    out.push_str(key);
                    walk(value, out)
                }
            }
            Value::Number(number) => out.push_str(&number.to_string()),
            Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Value::Null => {}
        }
    }
    let mut decoded = progress.concat();
    walk(value, &mut decoded);
    ensure!(
        decoded.len() <= 2 * 1024 * 1024 && !redactor.contains_secret(&decoded),
        "executable data contains configured confidential values"
    );
    // Keys must not be usable as separators between string-value fragments.
    fn values(value: &Value, out: &mut String) {
        match value {
            Value::String(text) => out.push_str(text),
            Value::Array(items) => {
                for value in items {
                    values(value, out)
                }
            }
            Value::Object(items) => {
                for value in items.values() {
                    values(value, out)
                }
            }
            Value::Number(number) => out.push_str(&number.to_string()),
            Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Value::Null => {}
        }
    }
    let mut strings = progress.concat();
    values(value, &mut strings);
    ensure!(
        !redactor.contains_secret(&strings),
        "executable output contains split configured confidential values"
    );
    Ok(())
}
struct ExtensionTool {
    manager: Arc<Manager>,
    definition: crate::model::ToolDefinition,
    remote: String,
    kind: sdk::Kind,
    snapshot: Arc<ExecutableSnapshot>,
    executor: Arc<sdk::Executor>,
    contexts: Arc<Mutex<BTreeMap<Uuid, (ToolContext, String)>>>,
}
struct ContextLease {
    id: Uuid,
    contexts: Arc<Mutex<BTreeMap<Uuid, (ToolContext, String)>>>,
}
impl Drop for ContextLease {
    fn drop(&mut self) {
        if let Ok(mut contexts) = self.contexts.lock() {
            contexts.remove(&self.id);
        }
    }
}
#[async_trait]
impl Tool for ExtensionTool {
    fn definition(&self) -> crate::model::ToolDefinition {
        self.definition.clone()
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        Ok(self
            .execute_output(arguments, context)
            .await?
            .text_fallback())
    }
    async fn execute_output(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<voyage_protocol::tool_result::ToolOutput, ToolError> {
        fn fail(_: impl std::fmt::Display) -> ToolError {
            ToolError::Failed("executable extension failed; inspect review and cleanup; do not replay uncertain effects".into())
        }
        confidential(&context.redactor, &[], &arguments).map_err(|_| {
            ToolError::Denied(
                "configured confidential values cannot be passed to executable extensions".into(),
            )
        })?;
        let deadline = Instant::now() + context.timeout.min(Duration::from_secs(120));
        if context.policy.access_mode() == crate::config::AccessMode::ReadOnly {
            return Err(ToolError::Denied(
                "executable extensions are unavailable in read-only mode".into(),
            ));
        }
        if context.policy.access_mode() == crate::config::AccessMode::Approval {
            tokio::time::timeout_at(
                deadline,
                context.approver.approve(&context.approval(
                    "extension.invoke",
                    format!("{} @ {}", self.definition.name, self.snapshot.digest),
                    "Run this reviewed executable in required isolation".into(),
                )),
            )
            .await
            .map_err(|_| ToolError::Timeout(context.timeout))?
            .require_approved()?;
        }
        context.policy.check_execution_authority().map_err(fail)?;
        let (session, incarnation) = crate::host_resources::process_scope()
            .ok_or_else(|| fail(anyhow::anyhow!("owner unavailable")))?;
        let invocation = Uuid::new_v4();
        self.contexts
            .lock()
            .map_err(|_| fail(anyhow::anyhow!("context unavailable")))?
            .insert(
                invocation,
                (
                    context.clone(),
                    format!(
                        "{}.{}",
                        if self.kind == sdk::Kind::Lifecycle {
                            "lifecycle"
                        } else if self.kind == sdk::Kind::Command {
                            "command"
                        } else {
                            "tool"
                        },
                        self.remote
                    ),
                ),
            );
        let _context = ContextLease {
            id: invocation,
            contexts: self.contexts.clone(),
        };
        let progress = Arc::new(Mutex::new(Vec::new()));
        let handle = self
            .executor
            .start(
                sdk::Invocation {
                    identity: sdk::Identity {
                        session,
                        incarnation,
                        run: context.execution_id,
                        invocation,
                        package: self.snapshot.archive.manifest.id.clone(),
                        digest: self.snapshot.digest.clone(),
                    },
                    kind: self.kind,
                    name: self.remote.clone(),
                    arguments,
                    deadline,
                    cancellation: context.cancellation.clone(),
                },
                Arc::new(Broker {
                    context: context.clone(),
                    manager: self.manager.clone(),
                    progress: progress.clone(),
                }),
            )
            .map_err(fail)?;
        let completion = handle.completion().await;
        let accepted = (|| -> Result<voyage_protocol::tool_result::ToolOutput, ToolError> {
            let value = completion.result.map_err(fail)?;
            if completion.cleanup != sdk::Cleanup::Observed {
                return Err(fail(anyhow::anyhow!("cleanup pending")));
            }
            let progress = progress
                .lock()
                .map_err(|_| fail(anyhow::anyhow!("progress unavailable")))?;
            confidential(&context.redactor, &progress, &value).map_err(fail)?;
            let mut content = progress
                .iter()
                .map(|text| json!({"type":"text","text":text}))
                .collect::<Vec<_>>();
            content
                .push(json!({"type":"text","text":serde_json::to_string(&value).map_err(fail)?}));
            crate::tools::output::ingest(
                json!({"content":content,"structuredContent":value}),
                context,
            )
        })();
        if accepted.is_err() {
            self.executor.close();
        }
        // Protocol success alone is not runtime result acceptance. Confidentiality,
        // budget and artifact failures quarantine this package and record failure.
        if let Err(error) = crate::host_resources::extensions::outcome(invocation, accepted.is_ok())
        {
            self.executor.close();
            return Err(fail(error));
        }
        accepted
    }
}

pub(crate) fn register(
    tools: &mut crate::tools::ToolRegistry,
    manager: Arc<Manager>,
    policy: &crate::policy::Policy,
    config: &crate::Config,
) -> Result<()> {
    if !policy.sandbox().required() || crate::host_resources::process_scope().is_none() {
        return Ok(());
    }
    *manager
        .private_files
        .lock()
        .map_err(|_| anyhow::anyhow!("private file provenance unavailable"))? =
        config.extension_private_files.clone();
    manager.read_allowed.store(
        tools
            .definitions()
            .iter()
            .any(|definition| definition.name == "read_file"),
        Ordering::Release,
    );
    let catalog = Arc::new(Catalog::new(
        policy.workspace(),
        &crate::config::default_data_dir(),
    )?);
    for snapshot in catalog.executable_snapshots()? {
        if snapshot
            .archive
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "host.file.read")
            && !config.extension_private_files_complete
        {
            tracing::warn!(
                "executable host-read capability unavailable: reload configuration to establish private source provenance"
            );
            continue;
        }
        let snapshot = Arc::new(snapshot);
        let definitions = sdk::Definitions::parse(
            &snapshot.archive.manifest.definitions,
            &snapshot.archive.manifest.capabilities,
        )?;
        let contexts = Arc::new(Mutex::new(BTreeMap::new()));
        let adapter = Arc::new(Adapter {
            catalog: catalog.clone(),
            snapshot: snapshot.clone(),
            manager: Arc::downgrade(&manager),
            contexts: contexts.clone(),
        });
        let executor = sdk::Executor::new(
            snapshot.archive.manifest.definitions.clone(),
            snapshot.archive.manifest.capabilities.clone(),
            adapter,
        )?;
        let mut candidates: Vec<Arc<dyn Tool>> = Vec::new();
        for (kind, definitions) in [
            (sdk::Kind::Tool, definitions.tools),
            (sdk::Kind::Command, definitions.commands),
            (sdk::Kind::Lifecycle, definitions.lifecycle),
        ] {
            for definition in definitions {
                let prefix = match kind {
                    sdk::Kind::Tool => "ext",
                    sdk::Kind::Command => "extcmd",
                    sdk::Kind::Lifecycle => "extlife",
                };
                let name = format!(
                    "{prefix}_{}_{}",
                    snapshot.archive.manifest.id.replace('-', "_"),
                    definition.name
                );
                candidates.push(Arc::new(ExtensionTool {
                    manager: manager.clone(),
                    definition: crate::model::ToolDefinition {
                        name,
                        description: definition.description,
                        input_schema: definition.input_schema,
                        output_schema: Some(definition.output_schema),
                        annotations: None,
                    },
                    remote: definition.name,
                    kind,
                    snapshot: snapshot.clone(),
                    executor: executor.clone(),
                    contexts: contexts.clone(),
                }));
            }
        }
        // All definitions must be collision-free before this package contributes
        // anything. A broken package cannot erase an unrelated registered tool.
        let existing = tools.definitions();
        if candidates.iter().any(|candidate| {
            existing
                .iter()
                .any(|definition| definition.name == candidate.definition().name)
        }) {
            continue;
        }
        manager.add_executor(executor)?;
        for candidate in candidates {
            let name = candidate.definition().name;
            if name.starts_with("extlife_") {
                let event = if name.ends_with("_run_start") {
                    "run_start"
                } else {
                    "run_finish"
                };
                tools.register_extension_lifecycle(event, candidate)?;
            } else {
                tools.register_arc(candidate)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod confidentiality_tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_broker_wait_retains_blocking_read_until_drained() {
        let (release, wait) = std::sync::mpsc::channel();
        let read = Arc::new(OwnedRead {
            task: tokio::sync::Mutex::new(Some(tokio::task::spawn_blocking(move || {
                wait.recv()
                    .map_err(|_| anyhow::anyhow!("fixture released"))?;
                Ok("private fixture value".into())
            }))),
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), read.take())
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(10), read.drain())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), read.drain())
            .await
            .unwrap();
        assert!(read.task.lock().await.is_none());
    }
    #[test]
    fn catches_progress_and_decoded_result_fragments() {
        let redactor = crate::tools::Redactor::new(vec!["abcdef".into()]);
        assert!(confidential(&redactor, &["abc".into()], &json!("def")).is_err());
        assert!(confidential(&redactor, &[], &json!(["abc", "def"])).is_err());
        assert!(confidential(&redactor, &[], &json!({"a":"abc","b":"def"})).is_err());
        assert!(
            confidential(
                &redactor,
                &["safe progress".into()],
                &json!({"answer":"safe"})
            )
            .is_ok()
        );
    }
}
