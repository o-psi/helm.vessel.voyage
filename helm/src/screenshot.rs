//! Explicit Helm-host desktop capture. Never a model tool or a remote-host command.
use anyhow::{Result, bail, ensure};

/// Confirmation is supplied only by the dedicated composer confirmation action.
/// Captured pixels may contain secrets; no capture is automatically sent.
pub(crate) fn capture(confirmed: bool, invocation: Option<&crate::Config>) -> Result<Vec<u8>> {
    ensure!(
        confirmed,
        "Confirm capture of the Helm host's full display; it may contain secrets"
    );
    let config = invocation
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| crate::Config::load(None))?;
    let policy = crate::policy::Policy::new(&config, config.resolve_workspace(None)?)?;
    policy.check_current()?;
    ensure!(
        policy.access_mode() != crate::config::AccessMode::ReadOnly,
        "Screenshots are disabled by local read-only policy"
    );
    ensure!(
        policy.sandbox().settings.mode == crate::sandbox::Mode::Off,
        "Desktop capture is unavailable under required process isolation; attach a manually captured image"
    );
    #[cfg(target_os = "linux")]
    {
        let (program, args): (&str, &[&str]) = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            ("/usr/bin/grim", &["-t", "png", "-"])
        } else if std::env::var_os("DISPLAY").is_some() {
            ("/usr/bin/maim", &["-u"])
        } else {
            bail!("No local graphical display; attach an image file instead")
        };
        ensure!(
            std::path::Path::new(program).is_file(),
            "Screenshot utility unavailable; install grim for Wayland or maim for X11, or attach a file"
        );
        policy.check_command_denials(&[program])?;
        match policy.command(&format!("{} {}", program, args.join(" "))) {
            crate::policy::Decision::Deny(_) => bail!("Screenshot capture denied by local policy"),
            crate::policy::Decision::Ask(_) if !confirmed => bail!("Screenshot approval required"),
            _ => {}
        }
        policy.check_current()?;
        capture_linux(program, args, policy.workspace())
    }
    #[cfg(not(target_os = "linux"))]
    bail!(
        "Screenshot capture is currently supported on Linux only; attach a manually captured PNG, JPEG or WebP"
    )
}

#[cfg(target_os = "linux")]
fn capture_linux(program: &str, args: &[&str], cwd: &std::path::Path) -> Result<Vec<u8>> {
    use std::{
        io::Read,
        os::unix::{io::AsRawFd, process::CommandExt},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const LIMIT: usize = 2 * 1024 * 1024;
    let mut command = Command::new(program);
    // Desktop connection variables only: never pass provider credentials or
    // loader injection variables to a capture utility.
    command.env_clear();
    for name in [
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_RUNTIME_DIR",
        "XAUTHORITY",
        "HOME",
        "LANG",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    // Fixed native utility, no shell, no user-controlled argv. Limit capture work.
    unsafe {
        command.pre_exec(|| {
            for (resource, limit) in [
                (libc::RLIMIT_CPU, 4),
                (libc::RLIMIT_AS, 512 * 1024 * 1024),
                (libc::RLIMIT_FSIZE, 2 * 1024 * 1024),
            ] {
                let limits = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &limits) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| anyhow::anyhow!("Screenshot utility could not start"))?;
    let pid = child.id() as i32;
    let mut reaped = false;
    let result = (|| -> Result<Vec<u8>> {
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Screenshot output unavailable"))?;
        let fd = output.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
            "Screenshot output setup failed"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            ensure!(
                Instant::now() < deadline,
                "Screenshot timed out; attach a file instead"
            );
            match output.read(&mut buffer) {
                Ok(0) => {
                    if let Some(status) = child.try_wait()? {
                        reaped = true;
                        ensure!(
                            status.success(),
                            "Screenshot failed or was refused by the desktop"
                        );
                        ensure!(!bytes.is_empty(), "Screenshot utility produced no image");
                        return Ok(bytes);
                    }
                }
                Ok(n) => {
                    ensure!(
                        bytes.len().saturating_add(n) <= LIMIT,
                        "Screenshot exceeds 2 MiB; resize and attach manually"
                    );
                    bytes.extend_from_slice(&buffer[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => bail!("Screenshot output failed"),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    // Observe cleanup before returning, including failure/limit/timeout. No untracked child.
    if !reaped {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    child
        .wait()
        .map_err(|_| anyhow::anyhow!("Screenshot process cleanup unconfirmed"))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_confirmation_precedes_any_capture() {
        assert!(
            capture(false, None)
                .unwrap_err()
                .to_string()
                .contains("Confirm")
        );
    }
    #[test]
    fn invocation_read_only_policy_refuses_capture() {
        let mut config = crate::Config::default();
        config.access = Some(crate::config::AccessMode::ReadOnly);
        let error = capture(true, Some(&config)).unwrap_err();
        assert!(error.to_string().contains("read-only"), "{error}");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_capture_runner_bounds_output_and_ignores_diagnostics() {
        let cwd = std::env::temp_dir();
        let result = capture_linux("/usr/bin/printf", &["synthetic-pixels"], &cwd).unwrap();
        assert_eq!(result, b"synthetic-pixels");
        let error =
            capture_linux("/usr/bin/head", &["-c", "2097153", "/dev/zero"], &cwd).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_capture_runner_observes_timeout_cleanup() {
        let error = capture_linux("/usr/bin/sleep", &["30"], &std::env::temp_dir()).unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }
}
