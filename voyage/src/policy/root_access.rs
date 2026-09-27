//! Non-destructive access preflight in the actual runtime identity and namespace.
//! This is a directory check, not authority to access every descendant or a promise
//! that future operations will succeed. Kernel access checks include ACLs and MAC;
//! EACCES alone cannot tell us which mechanism refused the operation.
use crate::tools::roots::RootPermission;
use anyhow::{Result, ensure};
use std::{
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
};

pub(super) fn verify(path: &Path, permission: RootPermission, identity: (u64, u64)) -> Result<()> {
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(access_error)?;
    let metadata = directory.metadata()?;
    ensure!(
        (metadata.dev(), metadata.ino()) == identity,
        "root identity changed during access preflight"
    );
    // Write grants include read, and both need directory search permission. Pin the
    // inode across the check. Do not use access(2), which checks real IDs, or the
    // older libc faccessat emulation, which may ignore ACLs. An unavailable kernel
    // check is a refusal, never evidence of access.
    let mode = libc::R_OK
        | libc::X_OK
        | if permission == RootPermission::Write {
            libc::W_OK
        } else {
            0
        };
    let result = unsafe {
        libc::syscall(
            libc::SYS_faccessat2,
            directory.as_raw_fd(),
            c"".as_ptr(),
            mode,
            libc::AT_EMPTY_PATH | libc::AT_EACCESS,
        )
    };
    if result != 0 {
        return Err(access_error(std::io::Error::last_os_error()));
    }
    // The descriptor protects what we checked; the policy still names a path.
    // Revalidate that name too. Later tool operations retain their own checks.
    let current = std::fs::symlink_metadata(path)?;
    ensure!(
        (current.dev(), current.ino()) == identity,
        "root identity changed during access preflight"
    );
    Ok(())
}

pub(super) fn access_error(error: std::io::Error) -> anyhow::Error {
    // Do not guess Unix mode bits vs ACL vs mandatory policy from EACCES.
    let cause = match error.raw_os_error() {
        Some(libc::EROFS) => "filesystem_read_only: the filesystem is mounted read-only",
        Some(libc::EACCES | libc::EPERM) => {
            "os_access_denied: operating-system permissions, ACLs or security policy deny this access"
        }
        Some(libc::ENOSYS | libc::EINVAL) => {
            "access_check_unavailable: this host cannot perform the required effective-identity access check"
        }
        _ => "access_check_failed: the operating-system access check could not complete",
    };
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    anyhow::anyhow!(
        "{cause} (execution UID {uid}, GID {gid}); filesystem consent cannot change OS permissions"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn effective_identity_directory_check_is_non_destructive_and_exact() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("directory");
        std::fs::create_dir(&path).unwrap();
        let m = std::fs::metadata(&path).unwrap();
        let identity = (m.dev(), m.ino());
        verify(&path, RootPermission::Write, identity).unwrap();
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
        assert!(verify(&path, RootPermission::Write, (m.dev(), m.ino() + 1)).is_err());
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(verify(&link, RootPermission::Read, identity).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        verify(&path, RootPermission::Read, identity).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            let error = verify(&path, RootPermission::Write, identity).unwrap_err();
            assert!(error.to_string().contains("os_access_denied"), "{error}");
            assert!(
                error
                    .to_string()
                    .contains("filesystem consent cannot change OS permissions")
            );
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn readonly_mount_refuses_write_access_without_a_write_probe() {
        const CHILD: &str = "VOYAGE_ROOT_ACCESS_READONLY_FIXTURE";
        if let Some(path) = std::env::var_os(CHILD) {
            let path = Path::new(&path);
            let m = std::fs::metadata(path).unwrap();
            verify(path, RootPermission::Read, (m.dev(), m.ino())).unwrap();
            let error = verify(path, RootPermission::Write, (m.dev(), m.ino())).unwrap_err();
            assert!(
                error.to_string().contains("filesystem_read_only"),
                "{error}"
            );
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new("bwrap")
            .args(["--die-with-parent", "--unshare-user", "--unshare-pid", "--ro-bind", "/", "/", "--proc", "/proc", "--"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "policy::roots::access::tests::readonly_mount_refuses_write_access_without_a_write_probe", "--nocapture"])
            .env(CHILD, root.path())
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}
