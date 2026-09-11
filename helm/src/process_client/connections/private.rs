//! Descriptor-relative private storage. A lock covers read/merge/atomic commit.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use voyage_storage::credentials;

fn purpose(name: &str) -> Vec<u8> {
    format!("helm-connection:{name}").into_bytes()
}
fn secret_record(name: &str) -> bool {
    name.ends_with(".credential") || name.ends_with(".redemption") || name.ends_with(".tmp")
}

pub const CREDENTIAL_LIMIT: usize = 16 * 1024;
pub const REGISTRY_LIMIT: usize = 1024 * 1024;

#[cfg(unix)]
fn check(file: &File, directory: bool, limit: usize) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    ensure!(
        m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0
            && if directory {
                m.is_dir()
            } else {
                m.is_file() && m.nlink() == 1 && m.len() <= limit as u64
            },
        "private connection storage ownership, permissions or size is invalid"
    );
    Ok(())
}

pub fn read_path(path: &Path, limit: usize) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .context("cannot open private access file")?;
        check(&file, false, limit + credentials::OVERHEAD)?;
        let bytes = read(file, limit + credentials::OVERHEAD)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("invalid private access filename")?;
        let bytes = credentials::open(&purpose(name), &bytes)?;
        ensure!(
            bytes.len() <= limit,
            "private connection file exceeds size limit"
        );
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, limit);
        anyhow::bail!("private connections unsupported on this platform")
    }
}
fn read(file: File, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "private connection file exceeds size limit"
    );
    Ok(bytes)
}

pub struct Directory(File);
impl Directory {
    pub fn open(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            // Only this last component is created; callers select an existing parent.
            match std::fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(path)
                .context("cannot open private connection directory")?;
            check(&file, true, 0)?;
            Ok(Self(file))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    #[cfg(unix)]
    fn open_file(&self, name: &str, flags: i32, limit: usize) -> Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        ensure!(
            !name.contains('/') && name != "." && name != "..",
            "invalid private storage reference"
        );
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(&file, false, limit)?;
        Ok(file)
    }
    fn raw(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        #[cfg(unix)]
        {
            match self.open_file(name, libc::O_RDONLY, limit + credentials::OVERHEAD) {
                Ok(file) => Ok(Some(read(file, limit + credentials::OVERHEAD)?)),
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (name, limit);
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    pub fn read(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some(bytes) = self.raw(name, limit)? else {
            return Ok(None);
        };
        let bytes = credentials::open(&purpose(name), &bytes)?;
        ensure!(
            bytes.len() <= limit,
            "private connection file exceeds size limit"
        );
        Ok(Some(bytes))
    }
    pub fn lock(&self) -> Result<File> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = self.open_file("registry.lock", libc::O_RDWR | libc::O_CREAT, 0)?;
            ensure!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
                "connection registry busy in another Helm window; retry"
            );
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    pub fn write(&self, name: &str, bytes: &[u8], limit: usize) -> Result<()> {
        ensure!(
            bytes.len() <= limit,
            "private connection file exceeds size limit"
        );
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Refuse unsafe destinations, even though rename would replace a symlink.
            self.read(name, limit)?;
            if secret_record(name) {
                const CHECK: &str = "protection.check";
                const VALUE: &[u8] = b"helm-connection-key-check-v1";
                if let Some(check) = self.raw(CHECK, 256)? {
                    ensure!(
                        credentials::encrypted(&check),
                        "invalid connection protection marker"
                    );
                    ensure!(
                        credentials::open(&purpose(CHECK), &check)? == VALUE,
                        "connection protection key mismatch"
                    );
                } else {
                    let check = credentials::seal(&purpose(CHECK), VALUE)?;
                    self.write(CHECK, &check, 256)?;
                }
            }
            let sealed = if secret_record(name) {
                Some(credentials::seal(&purpose(name), bytes)?)
            } else {
                None
            };
            let bytes = sealed.as_deref().unwrap_or(bytes);
            let temporary = format!("{}.tmp", uuid::Uuid::new_v4());
            let mut file = self.open_file(
                &temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                limit + credentials::OVERHEAD,
            )?;
            let tmp = std::ffi::CString::new(temporary)?;
            let destination = std::ffi::CString::new(name)?;
            let result = (|| -> Result<()> {
                file.write_all(bytes)?;
                file.sync_all()?;
                ensure!(
                    unsafe {
                        libc::renameat(
                            self.0.as_raw_fd(),
                            tmp.as_ptr(),
                            self.0.as_raw_fd(),
                            destination.as_ptr(),
                        )
                    } == 0,
                    "cannot commit private connection file"
                );
                self.0.sync_all()?;
                Ok(())
            })();
            if result.is_err() {
                unsafe {
                    libc::unlinkat(self.0.as_raw_fd(), tmp.as_ptr(), 0);
                }
            }
            result
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
}

/// Explicit idempotent migration; no network, identity changes or automatic key creation.
pub fn protect(root: &Path) -> Result<usize> {
    ensure!(
        root.is_absolute() && root.is_dir() && std::fs::canonicalize(root)? == root,
        "protection requires an existing canonical private connection directory"
    );
    credentials::check_key()?;
    let dir = Directory::open(root)?;
    let _lock = dir.lock()?;
    let mut names = Vec::new();
    for (i, entry) in std::fs::read_dir(root)?.enumerate() {
        ensure!(i < 16384, "connection migration entry limit exceeded");
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("invalid connection filename"))?;
        if secret_record(&name) {
            ensure!(
                entry.file_type()?.is_file(),
                "unsafe connection migration record"
            );
            if name.ends_with(".tmp")
                && dir
                    .raw(&name, REGISTRY_LIMIT)?
                    .is_some_and(|bytes| credentials::encrypted(&bytes))
            {
                continue;
            }
            names.push(name);
        }
    }
    names.sort();
    // Preflight every record before rewriting any. A crash can still leave a mixed
    // directory; both formats remain readable and rerunning preserves exact bytes.
    for name in &names {
        dir.read(name, REGISTRY_LIMIT)?
            .context("connection record disappeared")?;
    }
    for name in &names {
        let bytes = dir
            .read(name, REGISTRY_LIMIT)?
            .context("connection record disappeared")?;
        dir.write(name, &bytes, REGISTRY_LIMIT)?;
    }
    Ok(names.len())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    #[test]
    fn encrypted_connection_files() {
        if std::env::var_os("VOYAGE_TEST_CONNECTION_CHILD").is_none() {
            let root = tempfile::tempdir().unwrap();
            let keys = tempfile::Builder::new()
                .prefix("voyage-connection-key-")
                .tempdir_in("/dev/shm")
                .unwrap();
            let key = keys.path().join("key");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&key)
                .unwrap();
            file.write_all(&[11; 32]).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process_client::connections::private::tests::encrypted_connection_files",
                    "--nocapture",
                ])
                .env("VOYAGE_TEST_CONNECTION_CHILD", root.path())
                .env("VOYAGE_CREDENTIAL_KEY_FILE", &key)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let root =
            std::path::PathBuf::from(std::env::var_os("VOYAGE_TEST_CONNECTION_CHILD").unwrap());
        let key = std::path::PathBuf::from(std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").unwrap());
        let dir = Directory::open(&root).unwrap();
        let name = format!("{}.credential", uuid::Uuid::new_v4());
        let secret = b"{\"token\":\"synthetic-private-token\",\"command\":\"same-command\"}";
        dir.write(&name, secret, CREDENTIAL_LIMIT).unwrap();
        assert!(credentials::encrypted(
            &std::fs::read(root.join(&name)).unwrap()
        ));
        assert_eq!(dir.read(&name, CREDENTIAL_LIMIT).unwrap().unwrap(), secret);
        assert_eq!(
            read_path(&root.join(&name), CREDENTIAL_LIMIT).unwrap(),
            secret
        );
        let sealed = std::fs::read(root.join(&name)).unwrap();
        std::fs::rename(&key, key.with_extension("locked")).unwrap();
        assert!(dir.read(&name, CREDENTIAL_LIMIT).is_err());
        assert!(
            dir.write("new.redemption", secret, CREDENTIAL_LIMIT)
                .is_err()
        );
        assert!(!root.join("new.redemption").exists());
        assert_eq!(std::fs::read(root.join(&name)).unwrap(), sealed);
        std::fs::rename(key.with_extension("locked"), &key).unwrap();
        std::fs::write(&key, [12; 32]).unwrap();
        assert!(dir.read(&name, CREDENTIAL_LIMIT).is_err());
        std::fs::write(&key, [11; 32]).unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(dir.read(&name, CREDENTIAL_LIMIT).is_err());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        // Legacy state is unchanged until explicit migration, including an orphan temp.
        for name in ["old.redemption", "old.tmp"] {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(root.join(name))
                .unwrap();
            file.write_all(secret).unwrap();
        }
        dir.write("principal.json", b"public-principal", 1024)
            .unwrap();
        assert_eq!(protect(&root).unwrap(), 3);
        assert_eq!(
            std::fs::read(root.join("principal.json")).unwrap(),
            b"public-principal"
        );
        for name in [name.as_str(), "old.redemption", "old.tmp"] {
            assert!(credentials::encrypted(
                &std::fs::read(root.join(name)).unwrap()
            ));
            assert_eq!(dir.read(name, CREDENTIAL_LIMIT).unwrap().unwrap(), secret);
        }
        // A ciphertext orphan from a future interrupted write is already protected.
        std::fs::copy(root.join(&name), root.join("interrupted.tmp")).unwrap();
        let orphan = std::fs::read(root.join("interrupted.tmp")).unwrap();
        protect(&root).unwrap();
        assert_eq!(std::fs::read(root.join("interrupted.tmp")).unwrap(), orphan);
        let mut corrupt = std::fs::read(root.join(&name)).unwrap();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        std::fs::write(root.join(&name), &corrupt).unwrap();
        assert!(protect(&root).is_err());
        assert_eq!(std::fs::read(root.join(&name)).unwrap(), corrupt);
        // An existing symlink is refused rather than replaced.
        std::os::unix::fs::symlink(root.join("principal.json"), root.join("unsafe.redemption"))
            .unwrap();
        assert!(
            dir.write("unsafe.redemption", secret, CREDENTIAL_LIMIT)
                .is_err()
        );
    }
}
