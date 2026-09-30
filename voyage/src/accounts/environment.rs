//! Resolve only the executing user's named environment credential. Values stay
//! in that identity and never enter root IPC, diagnostics or configuration.
use anyhow::{Result, ensure};

pub(super) fn resolve(name: &str) -> Result<String> {
    ensure!(valid_name(name), "invalid environment credential name");
    match std::env::var(name) {
        Ok(value) => return checked(value),
        Err(std::env::VarError::NotPresent) => (),
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!("invalid environment credential"),
    }
    #[cfg(target_os = "linux")]
    if let Some(value) = linux::lookup(name)? {
        return checked(value);
    }
    anyhow::bail!(
        "bound environment credential unavailable; configure it in the executing identity"
    )
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.as_bytes()[0].is_ascii_digit()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
fn checked(value: String) -> Result<String> {
    ensure!(
        !value.is_empty() && value.len() <= 8192 && !value.chars().any(char::is_control),
        "invalid environment credential"
    );
    Ok(value)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::BTreeMap;
    use std::{
        io::Read,
        os::unix::{
            fs::{FileTypeExt, MetadataExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    const MAX_BYTES: usize = 256 * 1024;
    struct Snapshot {
        uid: u32,
        time: Instant,
        values: BTreeMap<String, String>,
    }
    static CACHE: OnceLock<Mutex<Option<Snapshot>>> = OnceLock::new();

    pub(super) fn lookup(name: &str) -> Result<Option<String>> {
        let uid = unsafe { libc::geteuid() };
        // Root's login/user-manager state is not administrator account scope.
        // A root runtime needs an explicitly provisioned private environment or
        // stored account; no ambient root fallback is manufactured here.
        if uid == 0 || unsafe { libc::getuid() } != uid {
            return Ok(None);
        }
        // Explicit private configuration is always read afresh. Presence,
        // replacement and parse errors cannot be hidden by the manager cache.
        if let Some(values) = private_file(uid)? {
            return Ok(values.get(name).cloned());
        }
        let mut cache = CACHE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| anyhow::anyhow!("executing environment unavailable"))?;
        if let Some(snapshot) = cache.as_ref() {
            if snapshot.uid == uid && snapshot.time.elapsed() < Duration::from_secs(1) {
                return Ok(snapshot.values.get(name).cloned());
            }
        }
        let values = manager(uid).unwrap_or_default();
        let value = values.get(name).cloned();
        *cache = Some(Snapshot {
            uid,
            time: Instant::now(),
            values,
        });
        Ok(value)
    }

    fn private_file(uid: u32) -> Result<Option<BTreeMap<String, String>>> {
        let root = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("executing configuration unavailable"))?
            .join("helm");
        read_private_file(&root, uid)
    }

    fn read_private_file(root: &Path, uid: u32) -> Result<Option<BTreeMap<String, String>>> {
        match std::fs::symlink_metadata(root.join("account-environment.json")) {
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => anyhow::bail!("executing environment file unavailable"),
        }
        let directory = crate::attachment::local_actor::storage::Directory::open_existing(root)?;
        let bytes = directory
            .read_bounded("account-environment.json", 65536)?
            .ok_or_else(|| anyhow::anyhow!("private environment file changed"))?;
        ensure!(
            unsafe { libc::geteuid() } == uid,
            "executing identity changed"
        );
        let values: BTreeMap<String, String> = serde_json::from_slice(&bytes)?;
        ensure!(
            values.len() <= 256 && values.keys().all(|name| valid_name(name)),
            "private environment bounds exceeded"
        );
        for value in values.values() {
            checked(value.clone())?;
        }
        directory.verify()?;
        Ok(Some(values))
    }

    fn trusted_executable(path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor)?;
            ensure!(
                !metadata.file_type().is_symlink()
                    && metadata.uid() == 0
                    && metadata.mode() & 0o022 == 0,
                "environment reader is not protected"
            );
        }
        ensure!(path.is_file(), "environment reader unavailable");
        Ok(())
    }
    fn decode(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
        ensure!(bytes.len() <= MAX_BYTES, "environment reply exceeds bounds");
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Reply {
            #[serde(rename = "type")]
            kind: String,
            data: serde_json::Value,
        }
        let reply: Reply = serde_json::from_slice(bytes)?;
        ensure!(reply.kind == "v", "environment reply has wrong type");
        let arguments = reply
            .data
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("environment data unavailable"))?;
        ensure!(arguments.len() == 1, "environment argument bound exceeded");
        let property: Reply = serde_json::from_value(arguments[0].clone())?;
        ensure!(property.kind == "as", "environment property has wrong type");
        let entries = property
            .data
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("environment data unavailable"))?;
        ensure!(entries.len() <= 1024, "environment entry bound exceeded");
        let mut values = BTreeMap::new();
        for entry in entries {
            let entry = entry
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("environment entry invalid"))?;
            let (name, value) = entry
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("environment assignment invalid"))?;
            ensure!(
                valid_name(name) && value.len() <= 8192,
                "environment assignment exceeds bounds"
            );
            ensure!(
                values.insert(name.to_owned(), value.to_owned()).is_none(),
                "duplicate environment assignment"
            );
        }
        Ok(values)
    }
    fn manager(uid: u32) -> Result<BTreeMap<String, String>> {
        let runtime = PathBuf::from(format!("/run/user/{uid}"));
        let metadata = std::fs::symlink_metadata(&runtime)?;
        ensure!(
            metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
            "user manager directory unavailable"
        );
        let bus = runtime.join("bus");
        let socket = std::fs::symlink_metadata(&bus)?;
        ensure!(
            socket.file_type().is_socket() && socket.uid() == uid,
            "user manager peer unavailable"
        );
        let binary = Path::new("/usr/bin/busctl");
        trusted_executable(binary)?;
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("XDG_RUNTIME_DIR", &runtime)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}", bus.display()),
            )
            .args([
                "--user",
                "--json=short",
                "--no-pager",
                "--auto-start=no",
                "--allow-interactive-authorization=no",
                "--timeout=1s",
                "call",
                "org.freedesktop.systemd1",
                "/org/freedesktop/systemd1",
                "org.freedesktop.DBus.Properties",
                "Get",
                "ss",
                "org.freedesktop.systemd1.Manager",
                "Environment",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                for (resource, limit) in
                    [(libc::RLIMIT_AS, 128 * 1024 * 1024), (libc::RLIMIT_CPU, 1)]
                {
                    let value = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &value) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("environment reader unavailable"))?;
        use std::os::fd::AsRawFd;
        let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(output.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                != 0
        {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("environment reader unavailable");
        }
        let result = (|| -> Result<Vec<u8>> {
            let deadline = Instant::now() + Duration::from_millis(1500);
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                ensure!(
                    Instant::now() < deadline,
                    "environment reader deadline exceeded"
                );
                match output.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        bytes.extend_from_slice(&buffer[..size]);
                        ensure!(
                            bytes.len() <= MAX_BYTES,
                            "environment reader output exceeds bounds"
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => anyhow::bail!("environment reader unavailable"),
                }
            }
            loop {
                if let Some(status) = child.try_wait()? {
                    ensure!(status.success(), "environment reader unavailable");
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "environment reader deadline exceeded"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => decode(&bytes),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("environment reader unavailable")
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn explicit_private_environment_distinguishes_absence_from_refusal() {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("helm");
            let uid = unsafe { libc::geteuid() };
            assert!(read_private_file(&root, uid).unwrap().is_none());
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let path = root.join("account-environment.json");
            std::fs::write(&path, br#"{"API_KEY":"opaque=a $(not-executed)"}"#).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let values = read_private_file(&root, uid).unwrap().unwrap();
            assert_eq!(values.get("API_KEY").unwrap(), "opaque=a $(not-executed)");
            assert!(!values.contains_key("UNPROVISIONED"));
            std::fs::write(&path, b"{broken").unwrap();
            assert!(read_private_file(&root, uid).is_err());
            std::fs::write(&path, br#"{}"#).unwrap();
            assert!(read_private_file(&root, uid).unwrap().unwrap().is_empty());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            assert!(read_private_file(&root, uid).is_err());
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750)).unwrap();
            assert!(read_private_file(&root, uid).is_err());
        }

        #[test]
        fn explicit_environment_rejects_links_growth_and_other_identity() {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("helm");
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let path = root.join("account-environment.json");
            let backing = fixture.path().join("private.json");
            std::fs::write(&backing, br#"{"API_KEY":"opaque"}"#).unwrap();
            std::fs::set_permissions(&backing, std::fs::Permissions::from_mode(0o600)).unwrap();
            let uid = unsafe { libc::geteuid() };
            std::os::unix::fs::symlink(&backing, &path).unwrap();
            assert!(read_private_file(&root, uid).is_err());
            std::fs::remove_file(&path).unwrap();
            std::fs::hard_link(&backing, &path).unwrap();
            assert!(read_private_file(&root, uid).is_err());
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, vec![b'x'; 65537]).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(read_private_file(&root, uid).is_err());
            std::fs::write(&path, br#"{"API_KEY":"opaque"}"#).unwrap();
            assert!(read_private_file(&root, uid.wrapping_add(1)).is_err());
            assert!(read_private_file(&root, uid).is_ok());
        }

        #[test]
        fn property_decoder_preserves_values_without_shell_evaluation() {
            let value = serde_json::json!({"type":"v","data":[{"type":"as","data":["API_KEY=a=b $(never execute)","OTHER=opaque"]}]});
            let decoded = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(decoded.get("API_KEY").unwrap(), "a=b $(never execute)");
            assert_eq!(decoded.get("OTHER").unwrap(), "opaque");
            assert!(
                decode(br#"{"type":"v","data":[{"type":"as","data":["KEY=x","KEY=y"]}]}"#).is_err()
            );
            assert!(decode(br#"{"type":"s","data":"private"}"#).is_err());
            assert!(decode(&vec![b'x'; MAX_BYTES + 1]).is_err());
        }
        #[test]
        fn manager_reply_requires_exact_variant_and_named_assignments() {
            for reply in [
                serde_json::json!({"type":"v","data":[]}),
                serde_json::json!({"type":"v","data":[{"type":"s","data":"KEY=private"}]}),
                serde_json::json!({"type":"v","data":[{"type":"as","data":["KEY=x"]},{"type":"as","data":[]}]}),
                serde_json::json!({"type":"v","data":[{"type":"as","data":["9KEY=private"]}]}),
                serde_json::json!({"type":"v","data":[{"type":"as","data":["missing-assignment"]}]}),
                serde_json::json!({"type":"v","data":[{"type":"as","data":[42]}]}),
                serde_json::json!({"type":"v","data":[{"type":"as","data":["KEY=x"],"extra":"private"}]}),
            ] {
                assert!(decode(&serde_json::to_vec(&reply).unwrap()).is_err());
            }
            assert!(checked(String::new()).is_err());
            assert!(checked("private\nextra".to_owned()).is_err());
            assert!(checked("x".repeat(8193)).is_err());
        }
    }
}
