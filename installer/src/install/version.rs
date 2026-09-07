use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    os::fd::AsRawFd,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn inspect(bin: &Path) -> Result<String> {
    let mut version = None;
    for name in super::release::BINARIES {
        let mut child = Command::new(bin.join(name))
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot inspect local binary version")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Local binary {name} version inspection timed out")
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        ensure!(
            status.success(),
            "Local binary {name} version inspection failed"
        );
        let mut output = child.stdout.take().context("Missing version output")?;
        let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
        ensure!(
            flags >= 0
                && unsafe {
                    libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
                } == 0,
            "Cannot bound binary version output"
        );
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 4097];
        loop {
            ensure!(
                Instant::now() < deadline,
                "Local binary {name} version output timed out"
            );
            match output.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    ensure!(bytes.len() <= 4096, "Oversized binary version output");
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(bytes.len() <= 4096, "Oversized binary version output");
        let text = std::str::from_utf8(&bytes)?;
        let mut words = text.split_whitespace();
        ensure!(
            words.next() == Some(name),
            "Unexpected binary identity for {name}"
        );
        let value = words.next().context("Missing binary version")?.to_owned();
        ensure!(
            words.next().is_none() && value.len() <= 128 && !value.chars().any(char::is_control),
            "Invalid binary version"
        );
        if let Some(expected) = &version {
            ensure!(
                expected == &value,
                "Local release contains mixed binary versions"
            );
        } else {
            version = Some(value);
        }
    }
    version.context("Missing release version")
}
