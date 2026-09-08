//! Linux PTY spawn that retains only sandbox setup descriptors until bwrap consumes them.
use portable_pty::{Child, CommandBuilder, MasterPty};
use std::{
    fs::File,
    os::{fd::FromRawFd, unix::process::CommandExt},
    path::Path,
    process::Stdio,
};
pub(super) fn spawn(
    master: &dyn MasterPty,
    builder: &CommandBuilder,
    policy: &crate::policy::Policy,
) -> anyhow::Result<Box<dyn Child + Send + Sync>> {
    let cwd = builder
        .get_cwd()
        .map(Path::new)
        .unwrap_or(policy.workspace());
    let argv = builder.get_argv();
    anyhow::ensure!(!argv.is_empty(), "isolated terminal command unavailable");
    let mut command = policy.process_command(&argv[0], cwd)?;
    command
        .args(&argv[1..])
        .env_clear()
        .envs(builder.iter_full_env_as_str());
    let master = master
        .as_raw_fd()
        .ok_or_else(|| anyhow::anyhow!("isolated terminal descriptor unavailable"))?;
    // TIOCGPTPEER returns the already-open master's slave, without following a pathname.
    let raw = unsafe {
        libc::ioctl(
            master,
            libc::TIOCGPTPEER,
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    anyhow::ensure!(raw >= 0, "isolated terminal peer unavailable");
    let slave = unsafe { File::from_raw_fd(raw) };
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: stdio has been assigned before pre_exec, and both calls are child-only.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    policy.isolate_process(&mut command, cwd)?;
    Ok(Box::new(command.spawn()?))
}
