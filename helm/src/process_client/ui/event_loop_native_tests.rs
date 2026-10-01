//! Actual Linux PTY input and termios, never the user's terminal or provider.
use super::*;
use portable_pty::{CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    sync::mpsc as sync,
};

const CHILD: &str = "HELM_OWNED_EVENT_LOOP_353";
const TEST: &str = "process_client::ui::event_loop_native_tests::owned_pty_restores_terminal_after_detach_and_initialization_error";

struct OwnedChild(Box<dyn portable_pty::Child + Send + Sync>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn flags(fd: libc::c_int) -> libc::tcflag_t {
    let mut value = std::mem::MaybeUninit::<libc::termios>::uninit();
    assert_eq!(unsafe { libc::tcgetattr(fd, value.as_mut_ptr()) }, 0);
    unsafe { value.assume_init() }.c_lflag & (libc::ICANON | libc::ECHO)
}

fn until_output(receiver: &sync::Receiver<Vec<u8>>, output: &mut Vec<u8>, marker: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if String::from_utf8_lossy(output).contains(marker) {
            return;
        }
        assert!(Instant::now() < deadline, "owned TUI did not show {marker}");
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                assert!(
                    output.len() + chunk.len() <= 1024 * 1024,
                    "owned output bound"
                );
                output.extend(chunk);
            }
            Err(sync::RecvTimeoutError::Timeout) => (),
            Err(error) => panic!("owned TUI output stopped before {marker}: {error}"),
        }
    }
}

#[test]
fn owned_pty_restores_terminal_after_detach_and_initialization_error() {
    if let Ok(mode) = std::env::var(CHILD) {
        // Only this explicitly selected child has PTY stdio; no environment is
        // altered in the parent test process and no live Client is constructed.
        assert!(matches!(
            mode.as_str(),
            "normal" | "registry-refusal" | "initialization-error"
        ));
        let parent: libc::pid_t = std::env::var("HELM_OWNED_EVENT_LOOP_PARENT_353")
            .expect("owned parent witness")
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::getppid() }, parent);
        assert_eq!(
            unsafe { libc::getsid(0) },
            unsafe { libc::getpid() },
            "child must own its new PTY session"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let configuration = (mode == "initialization-error").then(crate::Config::default);
        let result = runtime.block_on(run_with_notice(Vec::new(), None, configuration, None));
        if mode == "initialization-error" {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No Vessel connection is available")
            );
        } else {
            result.unwrap();
        }
        return;
    }
    for mode in ["normal", "registry-refusal", "initialization-error"] {
        let root = tempfile::tempdir().unwrap();
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 36,
                cols: 120,
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
            "HELM_OWNED_EVENT_LOOP_PARENT_353",
            std::process::id().to_string(),
        );
        command.env("HOME", root.path());
        command.env("PATH", "/usr/bin:/bin");
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.env("HELM_MOTION", "never");
        command.cwd(root.path());
        for (key, directory) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            let path = root.path().join(directory);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            command.env(key, path);
        }
        let registry_parent = root.path().join("state/voyage");
        std::fs::create_dir_all(&registry_parent).unwrap();
        std::fs::set_permissions(&registry_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        if mode == "registry-refusal" {
            std::fs::write(
                registry_parent.join("helm-connections"),
                b"owned blocked registry",
            )
            .unwrap();
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (sender, receiver) = sync::sync_channel(8);
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
        let mut entered = false;
        if mode != "initialization-error" {
            until_output(&receiver, &mut output, "\u{1b}[?1049h");
            entered = true;
            writer.write_all(b"\x10").unwrap(); // Actual Ctrl+P menu input.
            writer.flush().unwrap();
            until_output(&receiver, &mut output, "Leave Helm");
            assert_eq!(flags(fd), 0, "owned TUI has not entered raw mode");
            writer
                .write_all(b"\x1b[200~MENU_ONLY_353\x1b[201~\x1b")
                .unwrap();
            writer.flush().unwrap();
            // Resize is a real kernel event; tiny geometry must pause hidden
            // inputs and keep detach available without dispatching any session.
            pair.master
                .resize(PtySize {
                    rows: 8,
                    cols: 20,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .unwrap();
            output.clear();
            until_output(&receiver, &mut output, "Input paused");
            writer.write_all(b"\x03").unwrap(); // Detach through the tiny-layout guard.
            writer.flush().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let exit = loop {
            if let Some(exit) = child.0.try_wait().unwrap() {
                break exit;
            }
            assert!(Instant::now() < deadline, "owned TUI did not exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(exit.success(), "owned TUI child failed in {mode}");
        assert_eq!(
            flags(fd),
            original,
            "Screen did not restore owned terminal flags"
        );
        assert!(!root.path().join("state/voyage/vessel").exists());
        assert!(
            std::fs::read_dir(root.path().join("data"))
                .unwrap()
                .next()
                .is_none()
        );
        if mode == "registry-refusal" {
            assert_eq!(
                std::fs::read(registry_parent.join("helm-connections")).unwrap(),
                b"owned blocked registry"
            );
        }
        drop(writer);
        drop(pair.slave);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reader_job.is_finished() {
            assert!(Instant::now() < deadline, "owned PTY reader did not retire");
            if let Ok(chunk) = receiver.recv_timeout(Duration::from_millis(20)) {
                assert!(output.len() + chunk.len() <= 1024 * 1024);
                output.extend(chunk);
            }
        }
        while let Ok(chunk) = receiver.try_recv() {
            assert!(output.len() + chunk.len() <= 1024 * 1024);
            output.extend(chunk);
        }
        reader_job.join().unwrap();
        let text = String::from_utf8_lossy(&output);
        assert!(entered || text.contains("\u{1b}[?1049h"));
        assert!(
            text.contains("\u{1b}[?1049l"),
            "owned alternate screen not retired"
        );
    }
}
