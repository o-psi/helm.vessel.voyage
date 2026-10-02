//! Linux adapter journeys run only in private self-libtest children.
//! PATH contains our scripts, never the human's desktop clipboard utilities.
use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    process::Command as ProcessCommand,
};

const CHILD: &str = "clipboard::owned_helper_tests::owned_clipboard_child";
const SCRIPT: &str = r#"#!/usr/bin/python3
import json, os, pathlib, sys, time
case = CASE
program = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path.cwd()
with (root/'calls.jsonl').open('a') as output:
    output.write(json.dumps({'program':program,'args':args,'pid':os.getpid(),
        'keys':sorted(os.environ), 'pgid':os.getpgrp(),
        'start_ticks':int(pathlib.Path('/proc/self/stat').read_text().rsplit(')',1)[1].split()[19])})+'\n')
if 'CLIPBOARD_NEVER_EXPORT' in os.environ: sys.exit(90)
if case == 'timeout': time.sleep(30)
if case == 'cancel': time.sleep(30)
if case == 'failure':
    sys.stderr.write('synthetic-private-stderr')
    sys.exit(4)
listing = args == ['--list-types'] or args == ['-selection','clipboard','-out','-target','TARGETS']
if program == 'powershell.exe':
    assert args[:5] == ['-NoLogo','-NoProfile','-NonInteractive','-STA','-Command']
    assert len(args) == 6
    script = args[5]
    if case == 'wsl-file':
        if 'GetFileDropList' in script: sys.stdout.buffer.write(b'file:///tmp/owned%20one\nfile:///tmp/owned-two')
    elif case == 'wsl-image':
        if 'GetImage' in script: sys.stdout.buffer.write(b'\x89PNG\r\nowned')
    elif case == 'wsl-text':
        if 'GetText' in script: sys.stdout.buffer.write('private 日本語 text'.encode())
    elif case == 'wsl-invalid-file':
        if 'GetFileDropList' in script: sys.stdout.buffer.write(b'https://invalid.test/private')
    elif case != 'wsl-empty': sys.exit(4)
    sys.exit(0)
if case == 'xclip' and program == 'wl-paste': sys.exit(2)
if listing:
    if case == 'invalid-types': sys.stdout.buffer.write(b'\xff')
    elif case == 'oversized-types': sys.stdout.buffer.write(b'x'*32769)
    elif case == 'image': sys.stdout.write('text/plain\nimage/png\n')
    elif case == 'empty-image': sys.stdout.write('image/png\nimage/jpeg\ntext/plain\n')
    elif case in ('uris','invalid-uris'): sys.stdout.write('text/uri-list\ntext/plain\n')
    elif case == 'gnome': sys.stdout.write('x-special/gnome-copied-files\n')
    elif case == 'no-types': sys.stdout.write('application/x-owned-unknown\n')
    else: sys.stdout.write('text/plain;charset=utf-8\n')
    sys.exit(0)
mime = args[-1]
if case == 'image': sys.stdout.buffer.write(b'\x89PNG\r\nowned')
elif case == 'empty-image':
    if mime == 'image/jpeg': sys.stdout.buffer.write(b'\xff\xd8owned')
elif case == 'uris': sys.stdout.write('# owned list\nfile:///tmp/owned%20one\nfile:///tmp/owned-two\n')
elif case == 'gnome': sys.stdout.write('copy\nfile:///tmp/owned-one\n')
elif case == 'invalid-uris':
    if mime == 'text/uri-list': sys.stdout.write('https://invalid.test/private')
    else: sys.stdout.write('retained literal text')
elif case == 'invalid-text': sys.stdout.buffer.write(b'\xff')
elif case == 'oversized-text': sys.stdout.buffer.write(b'x'*65537)
elif case == 'empty-text': pass
else: sys.stdout.buffer.write('private 日本語 text'.encode())
"#;

fn calls(root: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(root.join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_helpers_retired(root: &std::path::Path) {
    for call in calls(root) {
        let pid = call["pid"].as_u64().unwrap();
        assert_eq!(call["pgid"].as_u64(), Some(pid));
        assert!(
            !same_helper(root, &call),
            "owned clipboard helper still exists"
        );
        assert!(
            !call["keys"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "CLIPBOARD_NEVER_EXPORT")
        );
    }
}

fn same_helper(root: &std::path::Path, call: &serde_json::Value) -> bool {
    let pid = call["pid"].as_u64().unwrap();
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let Ok(meta) = fs::metadata(&proc) else {
        return false;
    };
    if meta.uid() != unsafe { libc::geteuid() } {
        return false;
    }
    let Ok(stat) = fs::read_to_string(proc.join("stat")) else {
        return false;
    };
    let start = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|v| v.parse::<u64>().ok());
    if start != call["start_ticks"].as_u64() {
        return false;
    }
    if stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().next())
        == Some("Z")
    {
        return true;
    }
    if fs::read_link(proc.join("cwd")).ok().as_deref() != Some(root) {
        return false;
    }
    let Ok(argv) = fs::read(proc.join("cmdline")) else {
        return false;
    };
    let helper = root.join("bin").join(call["program"].as_str().unwrap());
    argv.split(|b| *b == 0)
        .any(|arg| arg == helper.as_os_str().as_encoded_bytes())
}

#[test]
fn linux_clipboard_routes_bounds_privacy_and_cleanup_use_owned_children() {
    for case in [
        "text",
        "read-only",
        "sandbox-refusal",
        "missing-workspace",
        "xclip",
        "image",
        "empty-image",
        "uris",
        "gnome",
        "invalid-uris",
        "empty-text",
        "no-types",
        "missing",
        "failure",
        "invalid-types",
        "oversized-types",
        "invalid-text",
        "oversized-text",
        "timeout",
        "cancel",
        "pre-cancel",
        "expired",
        "wsl-file",
        "wsl-image",
        "wsl-text",
        "wsl-empty",
        "wsl-invalid-file",
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for program in ["wl-paste", "xclip", "powershell.exe"] {
            if case == "missing" {
                continue;
            }
            let file = bin.join(program);
            let source = SCRIPT.replace("case = CASE", &format!("case = {case:?}"));
            fs::write(&file, source).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut child = ProcessCommand::new(std::env::current_exe().unwrap());
        child
            .env_clear()
            .current_dir(root.path())
            .env("HOME", root.path())
            .env("PATH", &bin)
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("XDG_DATA_HOME", root.path().join("data"))
            .env("XDG_STATE_HOME", root.path().join("state"))
            .env("XDG_CACHE_HOME", root.path().join("cache"))
            .env("XDG_RUNTIME_DIR", root.path().join("runtime"))
            .env("CLIPBOARD_OWNED_CASE", case)
            .env("CLIPBOARD_NEVER_EXPORT", "synthetic-env-secret")
            .env("LANG", "C.UTF-8")
            .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if case.starts_with("wsl-") {
            child.env("WSL_DISTRO_NAME", "owned-fixture");
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        let mut process = child.spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        loop {
            if process.try_wait().unwrap().is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                process.kill().unwrap();
                process.wait().unwrap();
                for call in calls(root.path()) {
                    if same_helper(root.path(), &call) {
                        let pid = call["pid"].as_u64().unwrap() as i32;
                        assert_eq!(call["pgid"].as_i64(), Some(pid as i64));
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                }
                // Failure evidence remains private and available for an audit.
                let _retained = root.keep();
                panic!("owned clipboard child exceeded bound: {case}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = process.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "owned clipboard scenario {case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_helpers_retired(root.path());
    }
}

#[test]
fn owned_clipboard_child() {
    let Ok(case) = std::env::var("CLIPBOARD_OWNED_CASE") else {
        return;
    };
    let root = std::env::current_dir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let cancel = CancellationToken::new();
        if case == "pre-cancel" {
            cancel.cancel();
        }
        if matches!(
            case.as_str(),
            "timeout" | "cancel" | "expired" | "pre-cancel"
        ) {
            let reader = Reader {
                policy: crate::policy::Policy::new(&crate::Config::default(), root.clone())
                    .unwrap(),
                cancel: cancel.clone(),
                deadline: Instant::now()
                    + if case == "expired" {
                        Duration::ZERO
                    } else {
                        Duration::from_millis(250)
                    },
            };
            let cancelling = async {
                if case == "cancel" {
                    let limit = Instant::now() + Duration::from_secs(2);
                    loop {
                        if !calls(&root).is_empty() {
                            break;
                        }
                        assert!(
                            Instant::now() < limit,
                            "owned cancellation helper never admitted"
                        );
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    cancel.cancel();
                }
            };
            let (result, ()) = tokio::join!(
                reader.run(root.join("bin/wl-paste").to_str().unwrap(), &[], TEXT),
                cancelling
            );
            let error = result.unwrap_err().to_string();
            assert_eq!(
                error,
                if matches!(case.as_str(), "cancel" | "pre-cancel") {
                    "Clipboard cancelled"
                } else {
                    "Clipboard timed out"
                }
            );
            if matches!(case.as_str(), "expired" | "pre-cancel") {
                assert!(calls(&root).is_empty());
            } else {
                assert_eq!(calls(&root).len(), 1);
            }
        } else {
            let mut config = crate::Config::default();
            if case == "read-only" {
                config.access = Some(crate::config::AccessMode::ReadOnly);
                config.legacy_deny_commands.push("wl-paste".into());
            }
            if case == "sandbox-refusal" {
                config.sandbox.mode = crate::sandbox::Mode::Required;
            }
            if case == "missing-workspace" {
                config.workspace = Some(root.join("absent-workspace"));
            }
            let result = read(Some(config), cancel).await;
            match case.as_str() {
                "text" | "read-only" | "xclip" | "wsl-text" => {
                    let Content::Text(text) = result.unwrap() else {
                        panic!("expected owned text");
                    };
                    assert_eq!(text, "private 日本語 text");
                }
                "image" | "wsl-image" => {
                    let Content::Image { name, bytes } = result.unwrap() else {
                        panic!("expected owned image");
                    };
                    assert_eq!(name, "clipboard.png");
                    assert_eq!(bytes, b"\x89PNG\r\nowned");
                }
                "empty-image" => {
                    let Content::Image { name, bytes } = result.unwrap() else {
                        panic!("expected fallback image");
                    };
                    assert_eq!(name, "clipboard.jpg");
                    assert_eq!(bytes, b"\xff\xd8owned");
                }
                "uris" | "wsl-file" | "gnome" => {
                    let Content::Files(paths) = result.unwrap() else {
                        panic!("expected owned paths");
                    };
                    assert_eq!(
                        paths,
                        if case == "gnome" {
                            vec![PathBuf::from("/tmp/owned-one")]
                        } else {
                            vec![
                                PathBuf::from("/tmp/owned one"),
                                PathBuf::from("/tmp/owned-two"),
                            ]
                        }
                    );
                }
                "invalid-uris" => {
                    let Content::Text(text) = result.unwrap() else {
                        panic!("invalid URI must fall back to literal text");
                    };
                    assert_eq!(text, "retained literal text");
                }
                "empty-text" | "no-types" | "wsl-empty" => {
                    assert!(matches!(result.unwrap(), Content::Empty))
                }
                "invalid-types" => {
                    assert_eq!(result.unwrap_err().to_string(), "Invalid clipboard types")
                }
                "invalid-text" => assert_eq!(
                    result.unwrap_err().to_string(),
                    "Clipboard text is not UTF-8"
                ),
                "wsl-invalid-file" => assert_eq!(
                    result.unwrap_err().to_string(),
                    "Invalid clipboard file list"
                ),
                "oversized-types" | "oversized-text" => assert_eq!(
                    result.unwrap_err().to_string(),
                    "Clipboard output exceeds limit"
                ),
                "missing" | "failure" => {
                    let error = result.unwrap_err().to_string();
                    assert!(error.starts_with("Clipboard unavailable:"));
                    assert!(!error.contains("synthetic-private-stderr"));
                }
                "sandbox-refusal" => {
                    let error = result.unwrap_err().to_string();
                    assert!(
                        error == "Clipboard policy unavailable"
                            || error == "Clipboard unavailable under process isolation"
                    );
                    assert!(calls(&root).is_empty());
                }
                "missing-workspace" => {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "Clipboard workspace unavailable"
                    );
                    assert!(calls(&root).is_empty());
                }
                _ => panic!("unknown owned fixture"),
            }
            let observed = calls(&root);
            if case == "missing" {
                assert!(observed.is_empty());
            }
            if case == "text" {
                assert_eq!(observed.len(), 2);
                assert_eq!(observed[0]["args"], serde_json::json!(["--list-types"]));
                assert_eq!(
                    observed[1]["args"],
                    serde_json::json!(["--no-newline", "--type", "text/plain;charset=utf-8"])
                );
            }
            if case == "xclip" {
                assert_eq!(observed.len(), 3);
                assert_eq!(
                    observed[1]["args"],
                    serde_json::json!(["-selection", "clipboard", "-out", "-target", "TARGETS"])
                );
            }
            if case == "image" {
                assert_eq!(observed.len(), 2);
                assert_eq!(observed[1]["args"][2], "image/png");
            }
            if case == "empty-image" {
                assert_eq!(observed.len(), 3);
                assert_eq!(observed[2]["args"][2], "image/jpeg");
            }
        }
        assert_helpers_retired(&root);
    });
}
