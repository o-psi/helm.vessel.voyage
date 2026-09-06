//! A public-API future can be aborted independently of CLI signal handling.
#![cfg(unix)]
use std::{
    io::Read,
    os::fd::FromRawFd,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn workflow_input_future_abort_restores_terminal() {
    if std::env::var_os("VOYAGE_WORKFLOW_ABORT_FIXTURE").is_some() {
        child();
        return;
    }
    let mut master = -1;
    let mut slave = -1;
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
    let mut master = unsafe { std::fs::File::from_raw_fd(master) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    use std::os::fd::AsRawFd;
    let mut before: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut before) },
        0
    );
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "workflow_input_future_abort_restores_terminal",
            "--nocapture",
        ])
        .env("VOYAGE_WORKFLOW_ABORT_FIXTURE", "1")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut output = Vec::new();
    let mut sent = false;
    let status = loop {
        let mut bytes = [0; 4096];
        if let Ok(count) = master.read(&mut bytes) {
            output.extend_from_slice(&bytes[..count]);
        }
        if !sent
            && output
                .windows(b"token: ".len())
                .any(|part| part == b"token: ")
        {
            use std::io::Write;
            master.write_all(b"ABORT-SECRET-QUEUED").unwrap();
            sent = true;
        }
        if let Some(status) = process.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            process.kill().unwrap();
            process.wait().unwrap();
            panic!("aborted input future left a running child");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(sent, "native hidden prompt was never reached");
    assert!(status.success(), "aborted collector child failed");
    assert!(
        !output
            .windows(b"ABORT-SECRET".len())
            .any(|part| part == b"ABORT-SECRET")
    );
    let mut after: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut after) }, 0);
    assert_eq!(after.c_iflag, before.c_iflag);
    assert_eq!(after.c_oflag, before.c_oflag);
    assert_eq!(after.c_cflag, before.c_cflag);
    assert_eq!(after.c_lflag, before.c_lflag);
    assert_eq!(after.c_cc, before.c_cc);
    after.c_lflag &= !libc::ICANON;
    after.c_cc[libc::VMIN] = 0;
    after.c_cc[libc::VTIME] = 0;
    assert_eq!(
        unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &after) },
        0
    );
    let mut probe = [0u8; 128];
    assert_eq!(
        unsafe { libc::read(slave.as_raw_fd(), probe.as_mut_ptr().cast(), probe.len()) },
        0
    );
}

fn child() {
    let directory = tempfile::tempdir().unwrap();
    let workflows = directory.path().join("workflows");
    std::fs::create_dir(&workflows).unwrap();
    std::fs::write(workflows.join("abort-input.toml"), "schema_version=1\nid='abort-input'\nversion='1'\ndescription='Abort fixture'\nprompt='Use {{token}}'\n[parameters.token]\ntype='string'\nrequired=true\nsecret=true\n").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let workspace = directory.path().to_owned();
        let args = helm::workflow::WorkflowArgs {
            user_directory: Some(workflows),
            json: false,
            command: helm::workflow::WorkflowCommand::Run(helm::workflow::InputArgs {
                selection: helm::workflow::Selection {
                    id: "abort-input".into(),
                    scope: None,
                },
                inputs: vec![],
                secret_env: vec![],
                trust_repository: None,
                no_save: true,
                prompt_missing: true,
                input_timeout_seconds: 30,
            }),
        };
        let task = tokio::spawn(async move { helm::workflow::prepare(args, &workspace).await });
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let mut mode: libc::termios = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::tcgetattr(0, &mut mode) }, 0);
            if mode.c_lflag & libc::ECHO == 0 {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut mode: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(0, &mut mode) }, 0);
        assert_ne!(
            mode.c_lflag & libc::ECHO,
            0,
            "late reader re-entered raw mode"
        );
    });
}
