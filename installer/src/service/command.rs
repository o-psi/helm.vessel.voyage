//! Bounded child execution, including output collection and service-manager failures.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
pub(crate) fn run(program: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    run_with_leases(program, args, input, &[])
}
pub(crate) fn run_with_leases(
    program: &Path,
    args: &[&str],
    input: Option<&[u8]>,
    leases: &[&std::fs::File],
) -> Result<Vec<u8>> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    };
    let mut inherited = Vec::new();
    for lease in leases {
        let descriptor = unsafe { libc::fcntl(lease.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 64) };
        ensure!(descriptor >= 0, "Cannot duplicate helper ownership lease");
        inherited.push(unsafe { OwnedFd::from_raw_fd(descriptor) });
    }
    let descriptors = inherited.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>();
    let parent = std::process::id() as libc::pid_t;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !descriptors.is_empty() {
        command.env(
            "LEGACY_UPDATE_LOCK_FDS",
            descriptors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );
        // Child inherits exact OFD leases. Parent death kills it before another
        // supervisor can proceed; leases remain held until observed child exit.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0
                    || libc::getppid() != parent
                {
                    return Err(std::io::Error::other("Helper parent no longer owned"));
                }
                for descriptor in &descriptors {
                    if libc::fcntl(*descriptor, libc::F_SETFD, 0) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("Cannot execute {}", program.display()))?;
    drop(inherited);
    let stdout = child.stdout.take().context("child stdout missing")?;
    let stderr = child.stderr.take().context("child stderr missing")?;
    let (send, receive) = mpsc::channel();
    for (is_stdout, reader, limit) in [
        (
            true,
            Box::new(stdout) as Box<dyn Read + Send>,
            4 * 1024 * 1024 + 4,
        ),
        (false, Box::new(stderr) as Box<dyn Read + Send>, 65536),
    ] {
        let send = send.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = send.send((is_stdout, limit, result));
        });
    }
    if let Some(input) = input {
        let result = child
            .stdin
            .take()
            .context("child stdin missing")?
            .write_all(input);
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} exceeded its ten-second deadline; service outcome must be inspected",
                program.display()
            )
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    for _ in 0..2 {
        let (stdout, limit, result) = receive
            .recv_timeout(Duration::from_secs(1))
            .context("child output did not close")?;
        let bytes = result?;
        ensure!(
            bytes.len() <= limit as usize,
            "child output exceeded its bound"
        );
        if stdout {
            output = bytes
        } else {
            diagnostics = bytes
        }
    }
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&diagnostics)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(400)
            .collect::<String>();
        bail!("{} failed ({status}): {diagnostic}", program.display())
    }
    Ok(output)
}
pub(super) fn systemctl(args: &[&str]) -> Result<String> {
    #[cfg(test)]
    return crate::fixture_tests::systemctl(args);
    #[cfg(not(test))]
    let mut all = vec!["--user", "--no-pager"];
    #[cfg(not(test))]
    all.extend_from_slice(args);
    #[cfg(not(test))]
    Ok(
        String::from_utf8(run(Path::new("/usr/bin/systemctl"), &all, None)?)?
            .trim()
            .into(),
    )
}
pub(super) fn query(property: &str) -> Result<String> {
    systemctl(&["show", super::unit::NAME, "--value", "--property", property])
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
