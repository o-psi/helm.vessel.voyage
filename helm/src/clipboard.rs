//! Explicit, local clipboard acquisition. Payloads must never enter diagnostics.
use anyhow::{Result, bail, ensure};
use std::{fmt, path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command, time::Instant};
use tokio_util::sync::CancellationToken;

const IMAGE: usize = 2 * 1024 * 1024;
const TEXT: usize = 64 * 1024;
const TYPES: usize = 32 * 1024;

pub(crate) enum Content {
    Image { name: String, bytes: Vec<u8> },
    Files(Vec<PathBuf>),
    Text(String),
    Empty,
}
impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Image { .. } => "Image(<private>)",
            Self::Files(_) => "Files(<private>)",
            Self::Text(_) => "Text(<private>)",
            Self::Empty => "Empty",
        })
    }
}

struct Reader {
    policy: crate::policy::Policy,
    cancel: CancellationToken,
    deadline: Instant,
}
impl Reader {
    // None means a missing utility or unsuccessful native request, not a timeout.
    async fn run(&self, program: &str, args: &[&str], limit: usize) -> Result<Option<Vec<u8>>> {
        self.policy
            .check_current()
            .map_err(|_| anyhow::anyhow!("Clipboard policy unavailable"))?;
        self.policy
            .check_command_denials(&[program])
            .map_err(|_| anyhow::anyhow!("Clipboard command denied"))?;
        ensure!(!self.cancel.is_cancelled(), "Clipboard cancelled");
        ensure!(Instant::now() < self.deadline, "Clipboard timed out");
        let mut command = Command::new(program);
        command.env_clear();
        for key in [
            "PATH",
            "LANG",
            "LC_CTYPE",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "WSL_INTEROP",
            "WSL_DISTRO_NAME",
            "WAYLAND_DISPLAY",
            "DISPLAY",
            "XDG_RUNTIME_DIR",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .args(args)
            .current_dir(self.policy.workspace())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
            // Bound regular-file output too (AppleScript PNG staging).
            unsafe {
                command.pre_exec(|| {
                    let limit = libc::rlimit {
                        rlim_cur: IMAGE as _,
                        rlim_max: IMAGE as _,
                    };
                    if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => return Ok(None),
        };
        let pid = child.id();
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Clipboard output unavailable"))?;
        let mut reaped = false;
        let operation = async {
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = output
                    .read(&mut buffer)
                    .await
                    .map_err(|_| anyhow::anyhow!("Clipboard read failed"))?;
                if n == 0 {
                    break;
                }
                ensure!(
                    n <= limit.saturating_sub(bytes.len()),
                    "Clipboard output exceeds limit"
                );
                bytes.extend_from_slice(&buffer[..n]);
            }
            let status = child
                .wait()
                .await
                .map_err(|_| anyhow::anyhow!("Clipboard helper failed"))?;
            reaped = true;
            Ok(if status.success() { Some(bytes) } else { None })
        };
        let result = tokio::select! { biased;
            _ = self.cancel.cancelled() => Err(anyhow::anyhow!("Clipboard cancelled")),
            _ = tokio::time::sleep_until(self.deadline) => Err(anyhow::anyhow!("Clipboard timed out")),
            result = operation => result,
        };
        if !reaped {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            let _ = pid;
            let _ = child.start_kill();
            child
                .wait()
                .await
                .map_err(|_| anyhow::anyhow!("Clipboard helper cleanup failed"))?;
        }
        result
    }
}

pub(crate) async fn read(
    config: Option<crate::Config>,
    cancel: CancellationToken,
) -> Result<Content> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let config = config
        .map(Ok)
        .unwrap_or_else(|| crate::Config::load(None))
        .map_err(|_| anyhow::anyhow!("Clipboard configuration unavailable"))?;
    let policy = crate::policy::Policy::new(
        &config,
        config
            .resolve_workspace(None)
            .map_err(|_| anyhow::anyhow!("Clipboard workspace unavailable"))?,
    )
    .map_err(|_| anyhow::anyhow!("Clipboard policy unavailable"))?;
    policy
        .check_current()
        .map_err(|_| anyhow::anyhow!("Clipboard policy unavailable"))?;
    // Reading user input is allowed in read-only mode; do not apply shell-write policy.
    ensure!(
        policy.sandbox().settings.mode == crate::sandbox::Mode::Off,
        "Clipboard unavailable under process isolation"
    );
    let reader = Reader {
        policy,
        cancel,
        deadline,
    };
    #[cfg(target_os = "macos")]
    {
        return macos(&reader).await;
    }
    #[cfg(target_os = "windows")]
    {
        return windows(&reader).await;
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::env::var_os("WSL_INTEROP").is_some()
        {
            return windows(&reader).await;
        }
        return linux(&reader).await;
    }
    #[allow(unreachable_code)]
    {
        let _ = reader;
        bail!("Clipboard unsupported on this platform")
    }
}

fn text(bytes: Vec<u8>) -> Result<Content> {
    ensure!(bytes.len() <= TEXT, "Clipboard text exceeds limit");
    let value =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("Clipboard text is not UTF-8"))?;
    Ok(if value.is_empty() {
        Content::Empty
    } else {
        Content::Text(value)
    })
}

#[cfg(target_os = "linux")]
async fn linux(r: &Reader) -> Result<Content> {
    let mut available = false;
    for (program, list_args, wayland) in [
        ("wl-paste", vec!["--list-types"], true),
        (
            "xclip",
            vec!["-selection", "clipboard", "-out", "-target", "TARGETS"],
            false,
        ),
    ] {
        let Some(types) = r.run(program, &list_args, TYPES).await? else {
            continue;
        };
        available = true;
        let types =
            std::str::from_utf8(&types).map_err(|_| anyhow::anyhow!("Invalid clipboard types"))?;
        for (mime, extension) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/webp", "webp"),
            ("text/uri-list", ""),
            ("x-special/gnome-copied-files", ""),
            ("text/plain;charset=utf-8", ""),
            ("UTF8_STRING", ""),
            ("text/plain", ""),
            ("STRING", ""),
        ] {
            if !types.lines().any(|line| line.trim() == mime) {
                continue;
            }
            let args = if wayland {
                vec!["--no-newline", "--type", mime]
            } else {
                vec!["-selection", "clipboard", "-out", "-target", mime]
            };
            let Some(bytes) = r
                .run(
                    program,
                    &args,
                    if extension.is_empty() { TEXT } else { IMAGE },
                )
                .await?
            else {
                continue;
            };
            if !extension.is_empty() {
                if bytes.is_empty() {
                    continue;
                }
                return Ok(Content::Image {
                    name: format!("clipboard.{extension}"),
                    bytes,
                });
            }
            if matches!(mime, "text/uri-list" | "x-special/gnome-copied-files") {
                let value = std::str::from_utf8(&bytes)
                    .map_err(|_| anyhow::anyhow!("Invalid clipboard file list"))?;
                let value = value
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.is_empty() && *l != "copy" && *l != "cut")
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(paths) = pasted_paths(&value) {
                    return Ok(Content::Files(paths));
                }
                continue;
            }
            return text(bytes);
        }
    }
    ensure!(
        available,
        "Clipboard unavailable: check wl-paste/xclip, desktop access and copied contents; or paste an image path"
    );
    Ok(Content::Empty)
}

mod paths;
pub(crate) fn pasted_paths(text: &str) -> Option<Vec<PathBuf>> {
    paths::parse(text)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
async fn windows(r: &Reader) -> Result<Content> {
    // Constant scripts only. No clipboard content is ever interpreted as code.
    const FILES: &str = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; $v=[Windows.Forms.Clipboard]::GetFileDropList(); if($v.Count -eq 0){exit 0}; $s=[string]::Join("`n",$v); $e=[Text.UTF8Encoding]::new($false); if($e.GetByteCount($s) -gt 65536){exit 2}; $b=$e.GetBytes($s); [Console]::OpenStandardOutput().Write($b,0,$b.Length)"#;
    const PNG: &str = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; Add-Type -TypeDefinition 'public class ClipboardBoundedStream : System.IO.MemoryStream { public override void Write(byte[] b,int o,int n) { if(n>2097152-Length) throw new System.IO.IOException(); base.Write(b,o,n); } public override void WriteByte(byte b) { if(Length>=2097152) throw new System.IO.IOException(); base.WriteByte(b); } }'; $i=[Windows.Forms.Clipboard]::GetImage(); if($null -eq $i){exit 0}; $w=[long]$i.Width; $h=[long]$i.Height; if($w -le 0 -or $h -le 0 -or $w -gt 8192 -or $h -gt 8192 -or ($w*$h) -gt 16777216){$i.Dispose(); exit 2}; $s=[ClipboardBoundedStream]::new(); try { $i.Save($s,[Drawing.Imaging.ImageFormat]::Png); $s.Position=0; $s.CopyTo([Console]::OpenStandardOutput()) } finally { $s.Dispose(); $i.Dispose() }"#;
    const STRING: &str = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; $s=[Windows.Forms.Clipboard]::GetText(); $e=[Text.UTF8Encoding]::new($false); if($e.GetByteCount($s) -gt 65536){exit 2}; $b=$e.GetBytes($s); [Console]::OpenStandardOutput().Write($b,0,$b.Length)"#;
    for (script, limit, kind) in [(FILES, TEXT, 0), (PNG, IMAGE, 1), (STRING, TEXT, 2)] {
        let Some(bytes) = r
            .run(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-STA",
                    "-Command",
                    script,
                ],
                limit,
            )
            .await?
        else {
            bail!("Windows clipboard helper unavailable or failed");
        };
        if bytes.is_empty() {
            continue;
        }
        if kind == 1 {
            return Ok(Content::Image {
                name: "clipboard.png".into(),
                bytes,
            });
        }
        if kind == 0 {
            let value = std::str::from_utf8(&bytes)
                .map_err(|_| anyhow::anyhow!("Invalid clipboard file list"))?;
            return Ok(Content::Files(
                pasted_paths(value)
                    .ok_or_else(|| anyhow::anyhow!("Invalid clipboard file list"))?,
            ));
        }
        return text(bytes);
    }
    Ok(Content::Empty)
}

#[cfg(target_os = "macos")]
async fn macos(r: &Reader) -> Result<Content> {
    // A unique 0700 directory contains a pre-created 0600 file, not a shared
    // predictable /tmp pathname. AppleScript receives the path as argv data.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir()
        .map_err(|_| anyhow::anyhow!("Clipboard temporary storage unavailable"))?;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
        .map_err(|_| anyhow::anyhow!("Clipboard temporary storage unavailable"))?;
    let file = tempfile::NamedTempFile::new_in(dir.path())
        .map_err(|_| anyhow::anyhow!("Clipboard temporary storage unavailable"))?;
    const SCRIPT: &str = "on run argv\nset dest to POSIX file (item 1 of argv)\nset imageData to the clipboard as «class PNGf»\nset handle to open for access dest with write permission\ntry\nset eof handle to 0\nwrite imageData to handle\nclose access handle\non error\nclose access handle\nerror \"Clipboard image unavailable\"\nend try\nend run";
    let path = file
        .path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Clipboard temporary storage unavailable"))?;
    if r.run("osascript", &["-e", SCRIPT, path], TYPES)
        .await?
        .is_some()
    {
        let mut input = tokio::fs::File::open(file.path())
            .await
            .map_err(|_| anyhow::anyhow!("Clipboard image unavailable"))?;
        let operation = async {
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let n = input
                    .read(&mut buffer)
                    .await
                    .map_err(|_| anyhow::anyhow!("Clipboard image unavailable"))?;
                if n == 0 {
                    break;
                }
                ensure!(
                    n <= IMAGE.saturating_sub(bytes.len()),
                    "Clipboard image exceeds limit"
                );
                bytes.extend_from_slice(&buffer[..n]);
            }
            Ok(bytes)
        };
        let bytes: Vec<u8> = tokio::select! { biased;
            _ = r.cancel.cancelled() => bail!("Clipboard cancelled"),
            _ = tokio::time::sleep_until(r.deadline) => bail!("Clipboard timed out"),
            result = operation => result?,
        };
        if !bytes.is_empty() {
            return Ok(Content::Image {
                name: "clipboard.png".into(),
                bytes,
            });
        }
    }
    match r.run("pbpaste", &[], TEXT).await? {
        Some(bytes) => text(bytes),
        None => bail!("Clipboard helper unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_is_private() {
        for value in [
            Content::Image {
                name: "SECRET".into(),
                bytes: b"SECRET".to_vec(),
            },
            Content::Text("SECRET".into()),
            Content::Files(vec![PathBuf::from("SECRET")]),
        ] {
            assert!(!format!("{value:?}").contains("SECRET"));
        }
    }
    #[test]
    fn text_is_bounded() {
        assert!(text(vec![b'x'; TEXT + 1]).is_err());
    }
}

#[cfg(all(test, unix))]
mod helper_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    // No desktop utility or inherited clipboard fixture state is used.
    #[tokio::test]
    async fn synthetic_helper_bounds_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("fake-clipboard");
        std::fs::write(&helper, "#!/bin/sh\nprintf 123456789\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = crate::Config::default();
        let reader = Reader {
            policy: crate::policy::Policy::new(&config, dir.path().to_path_buf()).unwrap(),
            cancel: CancellationToken::new(),
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let program = helper.to_str().unwrap();
        assert!(reader.run(program, &[], 8).await.is_err());
        assert_eq!(
            reader.run(program, &[], 9).await.unwrap().unwrap(),
            b"123456789"
        );
        std::fs::write(&helper, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        let token = reader.cancel.clone();
        let cancel = async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            token.cancel();
        };
        let (result, ()) = tokio::join!(reader.run(program, &[], 9), cancel);
        assert!(result.is_err());
    }
}

#[cfg(all(test, unix))]
mod policy_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn explicit_read_only_input_obeys_hard_command_denial() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("clipboard-policy-fixture");
        std::fs::write(&path, "#!/bin/sh\nprintf fixture").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = crate::Config::default();
        config.access = Some(crate::config::AccessMode::ReadOnly);
        let reader = Reader {
            policy: crate::policy::Policy::new(&config, root.path().into()).unwrap(),
            cancel: CancellationToken::new(),
            deadline: Instant::now() + Duration::from_secs(1),
        };
        assert_eq!(
            reader
                .run(path.to_str().unwrap(), &[], 10)
                .await
                .unwrap()
                .unwrap(),
            b"fixture"
        );
        config.deny_commands.push("clipboard-policy-fixture".into());
        let reader = Reader {
            policy: crate::policy::Policy::new(&config, root.path().into()).unwrap(),
            cancel: CancellationToken::new(),
            deadline: Instant::now() + Duration::from_secs(1),
        };
        assert!(
            reader
                .run(path.to_str().unwrap(), &[], 10)
                .await
                .unwrap_err()
                .to_string()
                .contains("denied")
        );
    }
    #[tokio::test]
    async fn timeout_reaps_a_synthetic_helper() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("clipboard-timeout-fixture");
        std::fs::write(&path, "#!/bin/sh\nwhile :; do :; done").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let reader = Reader {
            policy: crate::policy::Policy::new(&crate::Config::default(), root.path().into())
                .unwrap(),
            cancel: CancellationToken::new(),
            deadline: Instant::now() + Duration::from_millis(30),
        };
        assert!(
            reader
                .run(path.to_str().unwrap(), &[], 10)
                .await
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }
}
