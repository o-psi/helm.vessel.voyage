//! Linux PTY fixtures own their child descriptors; never read or configure a
//! human terminal. Source prepared for the coordinated final validation pass.
use super::*;
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    process::{Command, Stdio},
};
const MODE: &str = "VOYAGE_WORKFLOW_PTY_MODE";
const ROOT: &str = "VOYAGE_WORKFLOW_PTY_ROOT";

#[test]
fn collector_child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let parameter: Parameter = serde_json::from_value(serde_json::json!({"type":"string","secret":true,"max_length":if mode=="invalid" {3} else {8192},"description":"Fixture input"})).unwrap();
        let timeout = if mode=="timeout" {Duration::from_millis(500)} else {Duration::from_secs(3)};
        let result = collect(vec![Field{name:"fixture".into(),parameter}],timeout).await;
        match mode.as_str() {
            "success" | "monitor" => {
                let collected = result.unwrap();
                assert_eq!(collected.values.len(),1);
                assert!(collected.values[0].1.as_str()=="fixture-secret-é");
                assert!(!collected.monitor.cancellation().is_cancelled());
                if mode=="monitor" {
                    std::fs::write(root.join("collected"),b"ready").unwrap();
                    tokio::time::timeout(Duration::from_secs(2),collected.monitor.cancellation().cancelled()).await.unwrap();
                }
                drop(collected);
            }
            "cancel" => assert!(matches!(result.err().unwrap().downcast_ref::<InputFailure>(),Some(InputFailure::Cancelled))),
            "timeout" => assert!(matches!(result.err().unwrap().downcast_ref::<InputFailure>(),Some(InputFailure::TimedOut))),
            "invalid" => assert!(result.err().unwrap().to_string().contains("three attempts")),
            _ => panic!("unknown fixture mode"),
        }
        std::fs::write(root.join("completed"),b"completed").unwrap();
    });
}
struct Pty {
    master: File,
    slave: File,
    saved: libc::termios,
}
impl Pty {
    fn new() -> Self {
        let (mut master, mut slave) = (-1, -1);
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            0
        );
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut saved = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut saved) }, 0);
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        Self {
            master,
            slave,
            saved,
        }
    }
    fn restored(&self) {
        let mut now: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave.as_raw_fd(), &mut now) },
            0
        );
        assert_eq!(
            (now.c_iflag, now.c_oflag, now.c_cflag, now.c_lflag),
            (
                self.saved.c_iflag,
                self.saved.c_oflag,
                self.saved.c_cflag,
                self.saved.c_lflag
            )
        );
        assert_eq!(now.c_cc, self.saved.c_cc);
    }
}
fn scenario(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut pty = Pty::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "workflow::prompt::pty_tests::collector_child"])
        .env(MODE, mode)
        .env(ROOT, root.path())
        .stdin(Stdio::from(pty.slave.try_clone().unwrap()))
        .stderr(Stdio::from(pty.slave.try_clone().unwrap()))
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut transcript = Vec::new();
    let mut submitted = 0;
    let mut signalled = false;
    loop {
        let mut data = [0; 4096];
        match pty.master.read(&mut data) {
            Ok(n) => {
                transcript.extend_from_slice(&data[..n]);
                assert!(transcript.len() < 65536);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
            Err(e) => panic!("owned PTY read failed: {e}"),
        }
        let prompts = String::from_utf8_lossy(&transcript)
            .matches("fixture: ")
            .count();
        if prompts > submitted {
            let bytes: &[u8] = match mode {
                "success" | "monitor" => "fixture-secret-é\r".as_bytes(),
                "cancel" => b"\x1b",
                "invalid" => b"long\r",
                _ => b"",
            };
            pty.master.write_all(bytes).unwrap();
            submitted = prompts;
        }
        if mode == "monitor" && !signalled && root.path().join("collected").exists() {
            assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
            signalled = true;
        }
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "isolated collector failed ({mode})");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated collector exceeded bounded deadline ({mode})");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(root.path().join("completed")).unwrap(),
        b"completed"
    );
    assert!(submitted >= 1, "collector never reached a prompt");
    pty.restored();
    assert!(!String::from_utf8_lossy(&transcript).contains("fixture-secret"));
    if mode == "invalid" {
        assert_eq!(submitted, 3);
    }
    if mode == "monitor" {
        assert!(signalled);
    }
}
#[test]
fn hidden_unicode_input_restores_owned_terminal_without_echo() {
    scenario("success");
}
#[test]
fn cancellation_restores_owned_terminal() {
    scenario("cancel");
}
#[test]
fn deadline_restores_owned_terminal() {
    scenario("timeout");
}
#[test]
fn validation_retries_are_bounded_without_disclosing_input() {
    scenario("invalid");
}
#[test]
fn post_collection_signal_cancels_monitor_without_reentering_terminal() {
    scenario("monitor");
}
