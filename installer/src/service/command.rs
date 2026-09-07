//! Bounded child execution, including output collection and service-manager failures.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
pub(super) fn run(program: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Cannot execute {}", program.display()))?;
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
    let mut all = vec!["--user", "--no-pager"];
    all.extend_from_slice(args);
    Ok(
        String::from_utf8(run(Path::new("/usr/bin/systemctl"), &all, None)?)?
            .trim()
            .into(),
    )
}
pub(super) fn query(property: &str) -> Result<String> {
    systemctl(&["show", super::unit::NAME, "--value", "--property", property])
}
