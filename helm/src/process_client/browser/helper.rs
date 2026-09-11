//! Private, bounded local adapter IPC. Neither helper diagnostics nor human input enter history.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{broadcast, oneshot},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const FRAME_LIMIT: usize = 4 * 1024 * 1024;
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>>;

pub(super) struct Helper {
    input: tokio::sync::Mutex<ChildStdin>,
    child: tokio::sync::Mutex<Child>,
    pending: Pending,
    events: broadcast::Sender<Value>,
    stopped: CancellationToken,
    reader: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    pid: u32,
}
impl Helper {
    pub async fn start(script: &Path) -> Result<Arc<Self>> {
        let mut command = tokio::process::Command::new("node");
        command.env_clear();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LANG",
            "LC_ALL",
            "TMPDIR",
            "XDG_RUNTIME_DIR",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // A dedicated group lets forced cleanup include browser descendants, not another browser.
        #[cfg(unix)]
        command.process_group(0);
        // Node instrumentation/preloads may capture private input. Never inherit them implicitly.
        for key in [
            "NODE_OPTIONS",
            "NODE_PATH",
            "DEBUG",
            "PWDEBUG",
            "PW_TEST_TRACE",
        ] {
            command.env_remove(key);
        }
        let mut child = command.spawn().context(
            "Cannot start local browser adapter; install Node and run helm browser setup",
        )?;
        let pid = child
            .id()
            .context("browser adapter process identity unavailable")?;
        let input = child
            .stdin
            .take()
            .context("browser adapter input unavailable")?;
        let output = child
            .stdout
            .take()
            .context("browser adapter output unavailable")?;
        let pending: Pending = Default::default();
        let (events, _) = broadcast::channel(32);
        let stopped = CancellationToken::new();
        let (p, e, s) = (pending.clone(), events.clone(), stopped.clone());
        let reader = tokio::spawn(async move {
            let mut output = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                // read_until without a limit can allocate arbitrary bytes from a broken helper.
                let read = async {
                    use tokio::io::AsyncReadExt;
                    (&mut output)
                        .take((FRAME_LIMIT + 1) as u64)
                        .read_until(b'\n', &mut line)
                        .await
                }
                .await;
                if !matches!(read, Ok(n) if n > 0)
                    || line.len() > FRAME_LIMIT
                    || !line.ends_with(b"\n")
                {
                    break;
                }
                let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                    break;
                };
                if let Some(id) = value.get("id").and_then(Value::as_str) {
                    let Some(reply) = p.lock().expect("adapter pending lock").remove(id) else {
                        continue;
                    };
                    let _ = reply.send(Ok(value));
                } else if value.get("event").is_some() {
                    if e.send(value).is_err() { /* startup may precede subscription */ }
                } else {
                    break;
                }
            }
            s.cancel();
            for (_, reply) in p.lock().expect("adapter pending lock").drain() {
                let _ = reply.send(Err(anyhow::anyhow!(
                    "Local browser adapter disconnected; action outcome may be unknown"
                )));
            }
        });
        Ok(Arc::new(Self {
            input: tokio::sync::Mutex::new(input),
            child: tokio::sync::Mutex::new(child),
            pending,
            events,
            stopped,
            reader: tokio::sync::Mutex::new(Some(reader)),
            pid,
        }))
    }
    pub fn events(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
    pub fn stopped(&self) -> CancellationToken {
        self.stopped.clone()
    }
    pub async fn call(&self, request: Value, timeout: Duration) -> Result<Value> {
        ensure!(
            !self.stopped.is_cancelled(),
            "Local browser adapter unavailable"
        );
        let id = Uuid::new_v4().to_string();
        let envelope = self.call_exact(request, id, timeout).await?;
        ensure!(
            envelope.get("ok") == Some(&Value::Bool(true)),
            "Local browser refused the operation; inspect its private companion"
        );
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    }
    /// Durable browser actions retain their remote request UUID, not an ephemeral IPC identity.
    pub async fn call_exact(
        &self,
        mut request: Value,
        id: String,
        timeout: Duration,
    ) -> Result<Value> {
        ensure!(
            !self.stopped.is_cancelled(),
            "Local browser adapter unavailable"
        );
        request
            .as_object_mut()
            .context("invalid adapter request")?
            .insert("id".into(), json!(id));
        let mut bytes = serde_json::to_vec(&request)?;
        ensure!(
            bytes.len() < FRAME_LIMIT,
            "Local browser request exceeds limit"
        );
        bytes.push(b'\n');
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("adapter pending lock");
            ensure!(pending.len() < 16, "Local browser request queue full");
            ensure!(
                !pending.contains_key(&id),
                "Local browser request already in flight"
            );
            pending.insert(id.clone(), sender);
        }
        let result = tokio::time::timeout(timeout, async {
            {
                let mut input = self.input.lock().await;
                input
                    .write_all(&bytes)
                    .await
                    .context("Local browser write failed; outcome may be unknown")?;
                input.flush().await?;
            }
            receiver
                .await
                .context("Local browser reply unavailable; outcome may be unknown")?
        })
        .await
        .context("Local browser deadline elapsed; action must not be replayed");
        self.pending
            .lock()
            .expect("adapter pending lock")
            .remove(&id);
        result?
    }
    pub async fn shutdown(&self) -> Result<()> {
        let graceful = self
            .call(json!({"op":"shutdown"}), Duration::from_secs(5))
            .await;
        let mut child = self.child.lock().await;
        let observed = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        if !matches!(observed, Ok(Ok(_))) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
            let _ = child.start_kill();
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .context("Local browser cleanup not observed")??;
        }
        self.stopped.cancel();
        if let Some(reader) = self.reader.lock().await.take() {
            reader.abort();
            let _ = reader.await;
        }
        // A forced process exit is not evidence that an external website effect was undone.
        graceful.map(|_| ())
    }
}
