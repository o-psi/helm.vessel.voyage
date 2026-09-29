use anyhow::{Result, ensure};
use std::{
    ffi::{CStr, CString},
    os::unix::ffi::OsStrExt,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};
use voyage_protocol::execution_identity::{AuthorityClass, ConfiguredExecutionIdentity};
use voyage_protocol::process::ProcessRegistration;

/// Resolve the reviewed identity against the host immediately before launch.
/// A saved numeric UID alone can name a different account after host changes.
#[cfg(target_os = "linux")]
#[allow(dead_code)] // Activated with the protected system-install execution path.
pub fn validate_identity(identity: &ConfiguredExecutionIdentity) -> Result<()> {
    ensure!(
        matches!(
            (identity.authority, identity.uid),
            (AuthorityClass::Administrator, 0) | (AuthorityClass::Ordinary, 1..=u32::MAX)
        ),
        "invalid execution authority and UID"
    );
    let user = CString::new(identity.user_name.as_bytes())?;
    let mut password = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    let code = unsafe {
        libc::getpwnam_r(
            user.as_ptr(),
            password.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    ensure!(
        code == 0 && !found.is_null(),
        "execution user is unavailable"
    );
    let password = unsafe { password.assume_init() };
    let home = unsafe { CStr::from_ptr(password.pw_dir) };
    ensure!(
        password.pw_uid == identity.uid
            && password.pw_gid == identity.gid
            && home.to_bytes() == identity.home.as_os_str().as_bytes(),
        "execution account changed since configuration"
    );
    let mut groups = [0 as libc::gid_t; 128];
    let mut count = groups.len() as libc::c_int;
    let result =
        unsafe { libc::getgrouplist(user.as_ptr(), identity.gid, groups.as_mut_ptr(), &mut count) };
    ensure!(
        result >= 0 && count >= 0 && count as usize <= groups.len(),
        "execution group set is unavailable or exceeds limit"
    );
    let mut actual = groups[..count as usize]
        .iter()
        .copied()
        .filter(|group| *group != identity.gid)
        .collect::<Vec<_>>();
    actual.sort_unstable();
    actual.dedup();
    let mut configured = identity.supplementary_groups.clone();
    configured.sort_unstable();
    configured.dedup();
    ensure!(actual == configured, "execution group membership changed");
    Ok(())
}

#[cfg(target_os = "linux")]
#[allow(dead_code)] // Activated with the protected system-install execution path.
pub fn configure_identity(
    command: &mut Command,
    identity: &ConfiguredExecutionIdentity,
) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "privileged supervisor required"
    );
    ensure!(
        identity.enabled
            && !identity.identity.id.is_nil()
            && !identity.account_context.id.is_nil()
            && identity.home.is_absolute()
            && identity.supplementary_groups.len() <= 64
            && !identity.supplementary_groups.contains(&identity.gid),
        "invalid configured execution identity"
    );
    validate_identity(identity)?;
    command.env_clear();
    command.env("HOME", &identity.home);
    command.env("USER", &identity.user_name);
    command.env("LOGNAME", &identity.user_name);
    command.env(
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    let uid = identity.uid;
    let gid = identity.gid;
    let groups = identity.supplementary_groups.clone();
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(groups.len(), groups.as_ptr()) != 0
                || libc::setresgid(gid, gid, gid) != 0
                || libc::prctl(libc::PR_SET_KEEPCAPS, 0, 0, 0, 0) != 0
                || libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_CLEAR_ALL,
                    0,
                    0,
                    0,
                ) != 0
                || libc::setresuid(uid, uid, uid) != 0
                || (uid != 0 && libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0)
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

/// A privileged helper may execute only an administrator-controlled program.
/// Every ancestor must exclude ordinary writes; symlinks are refused explicitly.
#[cfg(target_os = "linux")]
pub(super) fn protected_binary(binary: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    ensure!(binary.is_absolute(), "helper binary must be absolute");
    let mut path = std::path::PathBuf::from("/");
    let parts: Vec<_> = binary.components().collect();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => path.push(name),
            _ => anyhow::bail!("helper binary path must be normalized"),
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.uid() == 0
                && metadata.mode() & 0o022 == 0
                && !metadata.file_type().is_symlink(),
            "helper binary has an unsafe owner, ancestor or mode"
        );
        if index + 1 == parts.len() {
            ensure!(
                metadata.is_file()
                    && metadata.nlink() == 1
                    && metadata.mode() & 0o111 != 0
                    && metadata.mode() & 0o6000 == 0,
                "helper executable must be a regular single-link executable without set-ID bits"
            );
        } else {
            ensure!(
                metadata.is_dir(),
                "helper binary ancestor is not a directory"
            );
        }
    }
    Ok(())
}

pub fn launch(binary: &Path, directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    ensure!(
        registration.peer_uids.is_none(),
        "cross-identity launch is unavailable until protected runtime storage and recovery are active"
    );
    let mut command = Command::new(binary);
    command
        .arg("supervise")
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

#[cfg(all(test, target_os = "linux"))]
#[path = "launch_tests.rs"]
mod tests;
