//! Real owned Linux PTYs and descriptors; all input is private synthetic data.
use super::*;
use crate::terminal::{TerminalError, TerminalEvent, TerminalModes};
use portable_pty::{CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    sync::{Mutex, mpsc},
    time::Instant,
};
use uuid::Uuid;

const CHILD: &str = "HELM_OWNED_PLAIN_LOOP_353";
const TEST: &str = "plain_terminal::native_tests::owned_linux_plain_prompt_and_attachment_loops_restore_input_and_output";

fn flags(fd: i32) -> libc::tcflag_t {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) }, 0);
    unsafe { termios.assume_init() }.c_lflag & (libc::ICANON | libc::ECHO)
}

#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        ensure!(self.0.load(Ordering::Acquire), "owned authority retired");
        Ok(())
    }
}

struct Manager {
    id: TerminalId,
    mode: String,
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<Vec<u8>>>,
    sizes: Mutex<Vec<(u16, u16)>>,
    events: Mutex<Option<tokio::sync::broadcast::Sender<TerminalEvent>>>,
    cancel: CancellationToken,
    authority: Arc<Authority>,
}
impl Manager {
    fn snapshot_value(&self, exited: bool) -> TerminalSnapshot {
        TerminalSnapshot {
            id: self.id,
            title: "OWNED_PLAIN_SCREEN".into(),
            state: if exited {
                TerminalState::Exited { code: Some(0) }
            } else {
                TerminalState::Running
            },
            revision: 7,
            cells: vec![],
            cursor: None,
            modes: TerminalModes::default(),
            dropped_unread_bytes: 0,
            privacy: None,
        }
    }
    fn call(&self, id: TerminalId, name: &str) {
        assert_eq!(id, self.id);
        assert!(
            owns_terminal(),
            "manager called without outer terminal ownership"
        );
        self.calls.lock().unwrap().push(name.into());
    }
}
#[async_trait::async_trait]
impl InteractiveTerminals for Manager {
    async fn list(&self) -> std::result::Result<Vec<TerminalSummary>, TerminalError> {
        panic!("attachment must not select or open another terminal")
    }
    async fn attach(&self, id: TerminalId) -> std::result::Result<TerminalSnapshot, TerminalError> {
        self.call(id, "attach");
        assert!(Ownership::acquire().is_err());
        assert!(owns_terminal());
        if self.mode == "attach-refusal" {
            return Err(TerminalError::Closed);
        }
        Ok(self.snapshot_value(false))
    }
    async fn snapshot(
        &self,
        id: TerminalId,
    ) -> std::result::Result<TerminalSnapshot, TerminalError> {
        self.call(id, "snapshot");
        if self.mode == "snapshot-refusal" {
            return Err(TerminalError::Closed);
        }
        Ok(self.snapshot_value(
            self.mode == "exited"
                || (self.mode.starts_with("events-refresh")
                    && !self.writes.lock().unwrap().is_empty()),
        ))
    }
    async fn write(
        &self,
        id: TerminalId,
        bytes: Vec<u8>,
    ) -> std::result::Result<(), TerminalError> {
        self.call(id, "write");
        self.writes.lock().unwrap().push(bytes);
        if self.mode == "cancel" {
            self.cancel.cancel();
        }
        if self.mode == "revoke" {
            self.authority.0.store(false, Ordering::Release);
        }
        if self.mode.starts_with("events-refresh") {
            let events = self.events.lock().unwrap();
            for _ in 0..if self.mode == "events-refresh-lagged" {
                20
            } else {
                1
            } {
                events
                    .as_ref()
                    .unwrap()
                    .send(TerminalEvent::Changed(self.id))
                    .unwrap();
            }
        }
        if self.mode.starts_with("write-refusal") {
            return Err(TerminalError::Closed);
        }
        Ok(())
    }
    async fn resize(
        &self,
        id: TerminalId,
        columns: u16,
        rows: u16,
    ) -> std::result::Result<(), TerminalError> {
        self.call(id, "resize");
        self.sizes.lock().unwrap().push((columns, rows));
        if self.mode == "resize-refusal" {
            return Err(TerminalError::Closed);
        }
        if columns == 40 {
            println!("OWNED_RESIZE_40_11");
        }
        Ok(())
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TerminalEvent> {
        let mut events = self.events.lock().unwrap();
        let receiver = events.as_ref().unwrap().subscribe();
        if self.mode == "events-closed" {
            events.take();
        }
        receiver
    }
}

async fn prompt_child(mode: &str) {
    let mut pending = PendingInput::default();
    if mode == "prompt-buffered" {
        pending.append(b"first\r\nsecond\n").unwrap();
    } else if mode == "prompt-limit" {
        pending.append(&vec![b'x'; MAX_PENDING]).unwrap();
    } else if mode == "prompt-cancel" {
        pending.append(b"retained unsent").unwrap();
    }
    let value = pending_prompt(&mut pending, CancellationToken::new())
        .await
        .unwrap();
    match mode {
        "prompt-edit" => assert_eq!(value.as_deref(), Some("^éZx")),
        "prompt-limit" => assert_eq!(value.as_deref(), Some("ok")),
        "prompt-cancel" => {
            assert!(value.is_none());
            assert_eq!(pending.bytes, b"retained unsent");
        }
        "prompt-eof" => {
            assert!(value.is_none());
            assert!(pending.is_empty());
        }
        "prompt-buffered" => {
            assert_eq!(value.as_deref(), Some("first"));
            assert_eq!(pending.take_line().unwrap().as_deref(), Some("second"));
        }
        _ => panic!("unowned prompt fixture mode"),
    }
}

async fn attach_child(mode: &str) {
    let (events, _) = tokio::sync::broadcast::channel(4);
    let cancel = CancellationToken::new();
    let authority = Arc::new(Authority(AtomicBool::new(true)));
    let manager = Arc::new(Manager {
        id: TerminalId(Uuid::new_v4()),
        mode: mode.into(),
        calls: Mutex::new(vec![]),
        writes: Mutex::new(vec![]),
        sizes: Mutex::new(vec![]),
        events: Mutex::new(Some(events)),
        cancel: cancel.clone(),
        authority: authority.clone(),
    });
    let config = crate::Config {
        access: Some(if mode == "read-only" {
            crate::config::AccessMode::ReadOnly
        } else {
            crate::config::AccessMode::Unrestricted
        }),
        ..Default::default()
    };
    let policy = Arc::new(
        Policy::new(&config, std::env::current_dir().unwrap())
            .unwrap()
            .with_execution_authority(authority),
    );
    let result = attach(manager.clone(), manager.id, policy, cancel).await;
    assert!(!owns_terminal(), "attachment ownership was not released");
    let calls = manager.calls.lock().unwrap();
    let writes = manager.writes.lock().unwrap();
    match mode {
        "detach" | "write-refusal-detach" | "resize" => {
            let detached = result.unwrap();
            assert_eq!(detached.pending, b"suffix");
            assert!(!detached.exited);
            assert_eq!(detached.delivery_failed, mode == "write-refusal-detach");
            assert_eq!(*writes, vec![b"opaque".to_vec()]);
        }
        "exited" | "events-refresh" | "events-refresh-lagged" => {
            let detached = result.unwrap();
            assert!(detached.exited);
            assert!(!detached.delivery_failed);
            assert!(detached.pending.is_empty());
            if mode == "exited" {
                assert!(writes.is_empty());
            } else {
                assert_eq!(*writes, vec![b"opaque".to_vec()]);
            }
        }
        "read-only" => {
            assert!(
                result
                    .err()
                    .expect("owned refusal")
                    .to_string()
                    .contains("read-only")
            );
            assert!(calls.is_empty() && writes.is_empty());
        }
        "small" => {
            assert!(
                result
                    .err()
                    .expect("owned refusal")
                    .to_string()
                    .contains("too small")
            );
            assert!(writes.is_empty());
        }
        "cancel" => {
            assert!(
                result
                    .err()
                    .expect("owned refusal")
                    .to_string()
                    .contains("interrupted")
            );
            assert_eq!(*writes, vec![b"opaque".to_vec()]);
        }
        "revoke" => {
            assert!(
                result
                    .err()
                    .expect("owned refusal")
                    .to_string()
                    .contains("owned authority retired")
            );
            assert_eq!(*writes, vec![b"opaque".to_vec()]);
        }
        "write-refusal" => {
            assert!(result.is_err());
            assert_eq!(*writes, vec![b"opaque".to_vec()]);
        }
        "events-closed" => {
            assert!(
                result
                    .err()
                    .expect("owned refusal")
                    .to_string()
                    .contains("events closed")
            );
            assert!(writes.is_empty());
        }
        "attach-refusal" | "snapshot-refusal" | "resize-refusal" => {
            assert!(result.is_err());
            assert!(writes.is_empty());
        }
        _ => panic!("unowned attachment fixture mode"),
    }
    if mode != "read-only" {
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.as_str() == "attach")
                .count(),
            1
        );
    }
    if mode == "resize" {
        assert_eq!(*manager.sizes.lock().unwrap(), vec![(80, 23), (40, 11)]);
    }
}

struct SavedOutput(i32);
impl SavedOutput {
    fn new() -> Self {
        let fd = unsafe { libc::dup(1) };
        assert!(fd >= 0);
        Self(fd)
    }
    fn restore(&self) {
        assert_eq!(unsafe { libc::dup2(self.0, 1) }, 1);
    }
}
impl Drop for SavedOutput {
    fn drop(&mut self) {
        let _ = unsafe { libc::dup2(self.0, 1) };
        let _ = unsafe { libc::close(self.0) };
    }
}
fn output_child() {
    let saved = SavedOutput::new();
    assert_eq!(unsafe { libc::close(1) }, 0);
    assert!(Output::new().is_err());
    saved.restore();
    let mut output = Output::new().unwrap();
    assert_eq!(unsafe { libc::close(1) }, 0);
    assert!(output.write(b"owned failed write").is_err());
    saved.restore();
    drop(output);
    let mut pipe = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    assert_eq!(unsafe { libc::dup2(pipe[1], 1) }, 1);
    let original = unsafe { libc::fcntl(1, libc::F_GETFL) };
    let mut output = Output::new().unwrap();
    assert_ne!(
        unsafe { libc::fcntl(1, libc::F_GETFL) } & libc::O_NONBLOCK,
        0
    );
    let started = Instant::now();
    assert!(
        output
            .write(&vec![b'x'; 1024 * 1024])
            .unwrap_err()
            .to_string()
            .contains("stalled")
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(output);
    assert_eq!(unsafe { libc::fcntl(1, libc::F_GETFL) }, original);
    saved.restore();
    for fd in pipe {
        assert_eq!(unsafe { libc::close(fd) }, 0);
    }
}

struct OwnedChild(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn until(receiver: &mpsc::Receiver<Vec<u8>>, output: &mut Vec<u8>, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !String::from_utf8_lossy(output).contains(marker) {
        assert!(
            Instant::now() < deadline,
            "owned plain loop did not produce marker {marker}"
        );
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => {
                assert!(output.len() + chunk.len() <= 1024 * 1024);
                output.extend(chunk);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(error) => panic!("owned plain loop stopped before marker: {error}"),
        }
    }
}

#[test]
fn owned_linux_plain_prompt_and_attachment_loops_restore_input_and_output() {
    if let Ok(mode) = std::env::var(CHILD) {
        let parent: libc::pid_t = std::env::var("HELM_OWNED_PLAIN_PARENT_353")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::getppid() }, parent);
        assert_eq!(unsafe { libc::getsid(0) }, unsafe { libc::getpid() });
        let original_input = flags(0);
        let original_output = unsafe { libc::fcntl(1, libc::F_GETFL) };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        if mode.starts_with("prompt-") {
            runtime.block_on(prompt_child(&mode));
        } else if mode == "output-faults" {
            output_child();
        } else if mode == "discard" {
            let mut pending = PendingInput::default();
            pending.append(b"discarded fixture tail").unwrap();
            discard_failed_attempt(&mut pending).unwrap();
            assert!(pending.is_empty());
        } else {
            runtime.block_on(attach_child(&mode));
        }
        assert_eq!(flags(0), original_input);
        assert_eq!(unsafe { libc::fcntl(1, libc::F_GETFL) }, original_output);
        assert!(!owns_terminal());
        println!("OWNED_PLAIN_DONE");
        return;
    }
    for mode in [
        "prompt-buffered",
        "prompt-edit",
        "prompt-limit",
        "prompt-cancel",
        "prompt-eof",
        "detach",
        "write-refusal-detach",
        "write-refusal",
        "resize",
        "small",
        "cancel",
        "revoke",
        "events-closed",
        "events-refresh",
        "events-refresh-lagged",
        "attach-refusal",
        "snapshot-refusal",
        "resize-refusal",
        "exited",
        "read-only",
        "discard",
        "output-faults",
    ] {
        let root = tempfile::tempdir().unwrap();
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let fd = pair.master.as_raw_fd().unwrap();
        let original = flags(fd);
        assert_eq!(original, libc::ICANON | libc::ECHO);
        let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
        command.args(["--exact", TEST, "--nocapture"]);
        command.env_clear();
        command.env(CHILD, mode);
        command.env(
            "HELM_OWNED_PLAIN_PARENT_353",
            std::process::id().to_string(),
        );
        command.env("HOME", root.path());
        command.env("PATH", "/usr/bin:/bin");
        command.env("TERM", "xterm-256color");
        command.cwd(root.path());
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            command.env(key, path);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (sender, receiver) = mpsc::sync_channel(8);
        let reader_job = std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => {
                        if sender.send(buffer[..length].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut writer = pair.master.take_writer().unwrap();
        let mut child = OwnedChild(pair.slave.spawn_command(command).unwrap());
        let mut output = Vec::new();
        if matches!(
            mode,
            "prompt-edit" | "prompt-limit" | "prompt-cancel" | "prompt-eof"
        ) {
            until(&receiver, &mut output, "helm> ");
            assert_eq!(flags(fd), 0);
            let bytes: &[u8] = match mode {
                "prompt-edit" => "é🦀x\x1b[D\x7fZ\x1b[H^\x1b[F!\x1b[D\x1b[3~\x02\r".as_bytes(),
                "prompt-limit" => b"a",
                "prompt-cancel" => b"\x03",
                _ => b"\x04",
            };
            writer.write_all(bytes).unwrap();
            writer.flush().unwrap();
            if mode == "prompt-limit" {
                until(&receiver, &mut output, "limit; Ctrl+U> ");
                writer.write_all(b"\x15ok\r").unwrap();
                writer.flush().unwrap();
            }
        } else if matches!(
            mode,
            "detach"
                | "write-refusal-detach"
                | "write-refusal"
                | "resize"
                | "small"
                | "cancel"
                | "revoke"
                | "events-refresh"
                | "events-refresh-lagged"
        ) {
            until(&receiver, &mut output, "OWNED_PLAIN_SCREEN");
            assert_eq!(flags(fd), 0);
            if mode == "resize" {
                pair.master
                    .resize(PtySize {
                        rows: 12,
                        cols: 40,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .unwrap();
                until(&receiver, &mut output, "OWNED_RESIZE_40_11");
            }
            if mode == "small" {
                pair.master
                    .resize(PtySize {
                        rows: 2,
                        cols: 2,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .unwrap();
            } else {
                writer
                    .write_all(
                        if matches!(mode, "detach" | "write-refusal-detach" | "resize") {
                            b"opaque\x14suffix"
                        } else {
                            b"opaque"
                        },
                    )
                    .unwrap();
                writer.flush().unwrap();
            }
        }
        until(&receiver, &mut output, "OWNED_PLAIN_DONE");
        let deadline = Instant::now() + Duration::from_secs(5);
        let exit = loop {
            if let Some(exit) = child.0.try_wait().unwrap() {
                break exit;
            }
            assert!(Instant::now() < deadline, "owned child did not retire");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(exit.success(), "owned plain child failed in {mode}");
        assert_eq!(
            flags(fd),
            original,
            "owned terminal flags not restored in {mode}"
        );
        if !mode.starts_with("prompt-")
            && !matches!(mode, "read-only" | "discard" | "output-faults")
        {
            assert!(String::from_utf8_lossy(&output).contains("\x1b[?1049h"));
            assert!(String::from_utf8_lossy(&output).contains("\x1b[?1049l"));
        }
        for name in ["config", "data", "state", "cache"] {
            assert!(
                std::fs::read_dir(root.path().join(name))
                    .unwrap()
                    .next()
                    .is_none(),
                "plain fixture created runtime or account state"
            );
        }
        drop(writer);
        drop(pair.slave);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reader_job.is_finished() {
            assert!(Instant::now() < deadline, "owned reader did not retire");
            if let Ok(chunk) = receiver.recv_timeout(Duration::from_millis(20)) {
                assert!(output.len() + chunk.len() <= 1024 * 1024);
                output.extend(chunk);
            }
        }
        reader_job.join().unwrap();
    }
}
