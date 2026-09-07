use anyhow::Result;
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};
use voyage_protocol::process::ProcessRegistration;

pub fn launch(binary: &Path, directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    if matches!(
        registration.initialize,
        Some(voyage_protocol::process::RuntimeInitialization::Outbound { .. })
    ) {
        let mut relay = Command::new(binary);
        relay
            .arg("outbound-relay")
            .arg(directory)
            .arg("--voyage-binary")
            .arg(binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            relay.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = relay.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !relay_ready(directory) {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "outbound relay initialization unconfirmed"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
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

fn relay_ready(directory: &Path) -> bool {
    let Ok(pid) = std::fs::read_to_string(directory.join("outbound-relay.pid")) else {
        return false;
    };
    let Ok(pid) = pid.trim().parse::<u32>() else {
        return false;
    };
    if pid <= 1
        || !directory.join("outbound-identity.json").is_file()
        || !directory.join("outbound-relay.sock").exists()
    {
        return false;
    }
    let Ok(command) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<_> = command.split(|b| *b == 0).collect();
    args.get(1).is_some_and(|arg| *arg == b"outbound-relay")
        && args
            .get(2)
            .is_some_and(|arg| *arg == directory.as_os_str().as_encoded_bytes())
        && std::fs::read_link(format!("/proc/{pid}/exe"))
            .is_ok_and(|path| path.file_name().is_some_and(|name| name == "voyage"))
}
