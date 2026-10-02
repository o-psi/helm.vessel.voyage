//! Private child test-runner scope for APIs whose production data root is global.
//! Never changes the parent environment or operates another host/user/service.
use std::{
    fs::{File, OpenOptions},
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const MARKER: &str = "VOYAGE_OWNED_BROWSER_TEST_353";
const PARENT: &str = "VOYAGE_OWNED_BROWSER_PARENT_353";
const LIMIT: u64 = 1024 * 1024;

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn log(path: &std::path::Path) -> File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .unwrap()
}

pub(crate) fn run(name: &str, body: impl FnOnce()) {
    if let Ok(selected) = std::env::var(MARKER) {
        assert_eq!(selected, name, "only the exact child test is authorized");
        let parent: u32 = std::env::var(PARENT).unwrap().parse().unwrap();
        // SAFETY: these identity queries have no pointer or argument preconditions.
        assert_eq!(unsafe { libc::getppid() } as u32, parent);
        assert_eq!(unsafe { libc::getpgrp() }, unsafe { libc::getpid() });
        assert!(
            unsafe { libc::geteuid() } > 0,
            "ordinary fixture user required"
        );
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
        let data = crate::config::default_data_dir();
        assert!(home.is_absolute() && data.starts_with(&home));
        assert!(std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").is_none());
        // Normal executing-host bootstrap provides the data root. The private
        // storage API creates only its leaf and intentionally refuses missing
        // ancestors; reproduce that prerequisite inside this owned child.
        crate::attachment::journal::prepare_directory(data).unwrap();
        body();
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("browser-owned-353-")
        .tempdir()
        .unwrap();
    let stdout = root.path().join("stdout-private.log");
    let stderr = root.path().join("stderr-private.log");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env(MARKER, name)
        .env(PARENT, std::process::id().to_string())
        .env("HOME", root.path())
        .env("PATH", "/usr/bin:/bin")
        .current_dir(root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log(&stdout)))
        .stderr(Stdio::from(log(&stderr)));
    for (key, leaf) in [
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_RUNTIME_DIR", "runtime"),
    ] {
        let path = root.path().join(leaf);
        std::fs::create_dir(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        command.env(key, path);
    }
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    // SAFETY: the child only enters its own process group before executing the
    // same fixed test binary. No UID, capabilities or runtime policy is changed.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::umask(0o077);
            Ok(())
        });
    }
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(40);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline
            || std::fs::metadata(&stdout).unwrap().len() > LIMIT
            || std::fs::metadata(&stderr).unwrap().len() > LIMIT
        {
            drop(child);
            let evidence = root.keep();
            panic!("owned child deadline/output bound; retained private evidence: {evidence:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success()
        || std::fs::metadata(&stdout).unwrap().len() > LIMIT
        || std::fs::metadata(&stderr).unwrap().len() > LIMIT
    {
        let evidence = root.keep();
        panic!("owned child failed; retained private evidence: {evidence:?}");
    }
    assert!(
        child.0.try_wait().unwrap().is_some(),
        "owned child was not reaped"
    );
}
