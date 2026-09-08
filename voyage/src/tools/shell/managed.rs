//! Managed shell ownership. Linux session membership is observable, but is not
//! containment: a process which deliberately creates another session can escape.
use super::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

enum Output {
    Public(String),
    Private(SecretShellOutcome),
}

const MAX_JOBS: usize = 64;
const MAX_CAPTURE: usize = 8 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct ManagedShell {
    state: Arc<Mutex<State>>,
}
#[derive(Default)]
struct State {
    closed: bool,
    jobs: BTreeMap<Uuid, Arc<Job>>,
}
impl Drop for State {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.cancel.cancel();
        }
    }
}
struct Job {
    cancel: CancellationToken,
    finished: AtomicBool,
    observed: AtomicBool,
    // On failed observation retain the unreaped leader, preventing PID reuse.
    retained: Mutex<Option<std::process::Child>>,
}
#[derive(Debug)]
pub struct ShellShutdown {
    pub observation_complete: bool,
    pub remaining: Vec<Uuid>,
}
impl ManagedShell {
    pub fn has_owned_work(&self) -> bool {
        self.state.lock().map_or(true, |state| {
            state
                .jobs
                .values()
                .any(|job| !job.observed.load(Ordering::Acquire))
        })
    }

    pub fn can_retire(&self) -> bool {
        Arc::strong_count(&self.state) == 1 && !self.has_owned_work()
    }

    pub fn new() -> Self {
        Self::default()
    }
    pub async fn shutdown(&self, timeout: Duration) -> ShellShutdown {
        let jobs = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state
                .jobs
                .iter()
                .map(|(id, job)| {
                    job.cancel.cancel();
                    (*id, job.clone())
                })
                .collect::<Vec<_>>()
        };
        let now = tokio::time::Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        loop {
            let remaining = jobs
                .iter()
                .filter(|(_, job)| !job.observed.load(Ordering::Acquire))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            if remaining.is_empty()
                || tokio::time::Instant::now() >= deadline
                || jobs
                    .iter()
                    .all(|(_, job)| job.finished.load(Ordering::Acquire))
            {
                return ShellShutdown {
                    observation_complete: remaining.is_empty(),
                    remaining,
                };
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

#[async_trait]
impl Tool for ManagedShell {
    fn definition(&self) -> ToolDefinition {
        Shell.definition()
    }
    async fn execute(&self, value: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        match self.execute_mode(value, ctx, None).await? {
            Output::Public(output) => Ok(output),
            Output::Private(_) => Err(ToolError::Failed("unexpected private shell outcome".into())),
        }
    }
    async fn execute_secret_environment(
        &self,
        value: Value,
        ctx: &ToolContext,
        environment: crate::workflow::secrets::BoundEnvironment,
    ) -> Result<SecretShellOutcome, ToolError> {
        match self
            .execute_mode(value, ctx, Some(environment))
            .await
            .map_err(private_shell_error)?
        {
            Output::Private(outcome) => Ok(outcome),
            Output::Public(_) => Err(ToolError::Failed("unexpected public shell outcome".into())),
        }
    }
}

impl ManagedShell {
    async fn execute_mode(
        &self,
        value: Value,
        ctx: &ToolContext,
        environment: Option<crate::workflow::secrets::BoundEnvironment>,
    ) -> Result<Output, ToolError> {
        if Instant::now().checked_add(ctx.timeout).is_none() {
            return Err(ToolError::InvalidArguments(
                "shell timeout is out of range".into(),
            ));
        }
        let args: Args = serde_json::from_value(value)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        if environment.is_none() && !args.workflow_secrets.is_empty() {
            return Err(ToolError::InvalidArguments(
                "workflow secret references require the current-run registry binding".into(),
            ));
        }
        if environment.is_some() && args.workflow_secrets.is_empty() {
            return Err(ToolError::InvalidArguments(
                "private one-shot shell requires explicit workflow secret references".into(),
            ));
        }
        if environment.is_some() {
            authorize_secret_command(&args, ctx).await?;
        } else {
            match ctx.policy.command(&args.command) {
                Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
                Decision::Ask(reason) => {
                    let approval = ctx.approval("shell", &args.command, reason);
                    let approved = tokio::select! {
                        _ = ctx.cancellation.cancelled() => return Err(ToolError::Cancelled),
                        outcome = ctx.approver.approve(&approval) => outcome,
                    };
                    approved.require_approved()?;
                }
                Decision::Allow => {}
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (args, environment);
            return Err(ToolError::Failed(
                "managed shell cleanup observation is unavailable on this platform".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let id = Uuid::new_v4();
            let cancel = ctx.cancellation.child_token();
            let cancel_on_drop = cancel.clone().drop_guard();
            let job = Arc::new(Job {
                cancel,
                finished: AtomicBool::new(false),
                observed: AtomicBool::new(false),
                retained: Mutex::new(None),
            });
            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .jobs
                    .retain(|_, job| !job.observed.load(Ordering::Acquire));
                if state.closed || job.cancel.is_cancelled() {
                    return Err(ToolError::Cancelled);
                }
                if state.jobs.len() >= MAX_JOBS {
                    return Err(ToolError::Failed("managed shell capacity exhausted".into()));
                }
                state.jobs.insert(id, job.clone());
                let weak = Arc::downgrade(&self.state);
                let ctx = ctx.clone();
                let started = std::thread::Builder::new()
                    .name("managed-shell".into())
                    .spawn(move || linux::run(weak, job, args.command, ctx, environment, tx));
                if let Err(error) = started {
                    state.jobs.remove(&id);
                    return Err(ToolError::Failed(error.to_string()));
                }
            }
            let result = rx.await.map_err(|_| {
                ToolError::Failed("managed shell worker stopped before reporting output".into())
            })?;
            cancel_on_drop.disarm();
            result
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::tools::process::SessionIdentity;
    use std::{
        io::{self, Read},
        os::{fd::AsRawFd, unix::process::CommandExt},
    };
    struct Capture<R> {
        pipe: R,
        bytes: Vec<u8>,
        eof: bool,
        limit: usize,
    }
    impl<R: Read + AsRawFd> Capture<R> {
        fn new(pipe: R, limit: usize) -> io::Result<Self> {
            let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                    < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                pipe,
                bytes: Vec::new(),
                eof: false,
                limit,
            })
        }
        fn drain(&mut self) -> io::Result<()> {
            // Bound work per poll even when a child continuously floods a pipe.
            let mut buffer = [0; 8192];
            for _ in 0..8 {
                match self.pipe.read(&mut buffer) {
                    Ok(0) => {
                        self.eof = true;
                        break;
                    }
                    Ok(count) => {
                        let keep = count.min(self.limit.saturating_sub(self.bytes.len()));
                        self.bytes.extend_from_slice(&buffer[..keep]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        }
    }
    pub(super) fn run(
        state: std::sync::Weak<Mutex<State>>,
        job: Arc<Job>,
        command: String,
        ctx: ToolContext,
        environment: Option<crate::workflow::secrets::BoundEnvironment>,
        tx: tokio::sync::oneshot::Sender<Result<Output, ToolError>>,
    ) {
        let mut tx = Some(tx);
        let result = work(state, &job, &command, &ctx, environment.as_ref(), &mut tx);
        if let Some(tx) = tx {
            let _ = tx.send(result);
        }
        job.finished.store(true, Ordering::Release);
    }
    fn work(
        state: std::sync::Weak<Mutex<State>>,
        job: &Job,
        command: &str,
        ctx: &ToolContext,
        environment: Option<&crate::workflow::secrets::BoundEnvironment>,
        tx: &mut Option<tokio::sync::oneshot::Sender<Result<Output, ToolError>>>,
    ) -> Result<Output, ToolError> {
        let Some(deadline) = Instant::now().checked_add(ctx.timeout) else {
            job.observed.store(true, Ordering::Release);
            return Err(ToolError::InvalidArguments(
                "shell timeout is out of range".into(),
            ));
        };
        let Some(state) = state.upgrade() else {
            job.observed.store(true, Ordering::Release);
            return Err(ToolError::Cancelled);
        };
        let guard = state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.closed || job.cancel.is_cancelled() {
            job.observed.store(true, Ordering::Release);
            return Err(ToolError::Cancelled);
        }
        // This job remains unobserved across spawn, including cancellation
        // racing this admission. Shutdown never reports it cleaned prematurely.
        drop(guard);
        drop(state);
        let mut cmd = match ctx.policy.process_command("sh", ctx.policy.workspace()) {
            Ok(command) => command,
            Err(error) => {
                job.observed.store(true, Ordering::Release);
                return Err(ToolError::Failed(error.to_string()));
            }
        };
        cmd.arg("-lc")
            .arg(command)
            .current_dir(ctx.policy.workspace())
            .env_clear()
            .envs(&ctx.environment)
            .stdin(Stdio::null())
            .stdout(if environment.is_some() {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stderr(if environment.is_some() {
                Stdio::null()
            } else {
                Stdio::piped()
            });
        if let Some(environment) = environment {
            cmd.envs(environment.iter());
        }
        // SAFETY: setsid is async-signal-safe and does not access Rust state.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        if ctx.policy.check_execution_authority().is_err() {
            job.observed.store(true, Ordering::Release);
            return Err(ToolError::Denied(
                "foreground execution authority unavailable".into(),
            ));
        }
        if environment.is_some() {
            // A worker may have waited after approval. Recheck immediately before
            // releasing its private environment; no subprocess exists on failure.
            if let Err(error) = check_private_policy(ctx) {
                job.observed.store(true, Ordering::Release);
                return Err(error);
            }
            if job.cancel.is_cancelled() || Instant::now() >= deadline {
                job.observed.store(true, Ordering::Release);
                return Err(if job.cancel.is_cancelled() {
                    ToolError::Cancelled
                } else {
                    ToolError::Timeout(ctx.timeout)
                });
            }
        }
        if let Err(error) = ctx.policy.isolate_process(&mut cmd, ctx.policy.workspace()) {
            job.observed.store(true, Ordering::Release);
            return Err(ToolError::Failed(error.to_string()));
        }
        let child = cmd.spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                job.observed.store(true, Ordering::Release);
                return Err(ToolError::Failed(e.to_string()));
            }
        };
        let identity = match SessionIdentity::capture(child.id()) {
            Ok(identity) => identity,
            Err(_) => {
                // Child has not been reaped, so its numeric PID is still owned.
                let _ = child.kill();
                *job.retained.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                return Err(ToolError::Failed(
                    "managed shell session identity unavailable; cleanup unconfirmed".into(),
                ));
            }
        };
        let limit = ctx.max_output_bytes.min(MAX_CAPTURE);
        let captures = if environment.is_some() {
            Ok(None)
        } else {
            child
                .stdout
                .take()
                .zip(child.stderr.take())
                .ok_or_else(|| io::Error::other("missing shell pipes"))
                .and_then(|(out, err)| {
                    Ok(Some((
                        Capture::new(out, limit + 1)?,
                        Capture::new(err, limit + 1)?,
                    )))
                })
        };
        let result = match captures {
            Ok(mut captures) => loop {
                if job.cancel.is_cancelled() {
                    break Err(ToolError::Cancelled);
                }
                if tx.is_some() && Instant::now() >= deadline {
                    break Err(ToolError::Timeout(ctx.timeout));
                }
                if let Some((stdout, stderr)) = &mut captures
                    && let Err(e) = stdout.drain().and_then(|()| stderr.drain())
                {
                    break Err(ToolError::Failed(e.to_string()));
                }
                match identity.exit_status() {
                    Err(_) => {
                        break Err(ToolError::Failed(
                            "managed shell exit observation unavailable".into(),
                        ));
                    }
                    Ok(Some(status))
                        if captures
                            .as_ref()
                            .is_none_or(|(stdout, stderr)| stdout.eof && stderr.eof) =>
                    {
                        if let Some(tx) = tx.take() {
                            let output = if let Some((stdout, stderr)) = &captures {
                                let combined = format!(
                                    "exit: {}\nstdout:\n{}\nstderr:\n{}",
                                    status
                                        .signal()
                                        .map(|_| "signal".to_owned())
                                        .unwrap_or_else(|| status.exit_code().to_string()),
                                    String::from_utf8_lossy(&stdout.bytes),
                                    String::from_utf8_lossy(&stderr.bytes)
                                );
                                Output::Public(truncate(combined.into_bytes(), limit))
                            } else {
                                Output::Private(if status.signal().is_some() {
                                    SecretShellOutcome::Signalled
                                } else {
                                    SecretShellOutcome::Exited {
                                        code: i64::from(status.exit_code()),
                                    }
                                })
                            };
                            let _ = tx.send(Ok(output));
                        }
                        if identity.observe_empty(Instant::now() + Duration::from_millis(100))
                            == Ok(true)
                        {
                            if child.wait().is_ok() {
                                job.observed.store(true, Ordering::Release);
                                return Ok(Output::Public(String::new()));
                            }
                            break Err(ToolError::Failed("managed shell child reap failed".into()));
                        }
                    }
                    _ => {}
                }
                std::thread::sleep(Duration::from_millis(if tx.is_some() { 5 } else { 100 }));
            },
            Err(e) => Err(ToolError::Failed(e.to_string())),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match identity.kill_and_observe(deadline) {
                Ok(true) => {
                    if identity.exit_status().is_ok_and(|status| status.is_some())
                        && child.wait().is_ok()
                    {
                        job.observed.store(true, Ordering::Release);
                        return result;
                    }
                    break;
                }
                Ok(false) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
        *job.retained.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
        result
    }
}
