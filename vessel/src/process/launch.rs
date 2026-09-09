use anyhow::Result;
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};
use voyage_protocol::process::ProcessRegistration;

pub fn launch(binary: &Path, directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .arg("serve")
        .arg("--directory")
        .arg(directory)
        .arg("--session")
        .arg(registration.session_id.to_string())
        .arg("--incarnation")
        .arg(registration.incarnation.to_string())
        .arg("--workspace")
        .arg(&registration.workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = &registration.config_path {
        command.arg("--config").arg(path);
    }
    // The supervisor's lifetime and terminal must not become the runtime lifetime.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
