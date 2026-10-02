//! Exact test subprocess scope; no parent HOME/account/reservation ledger used.
use std::{
    fs,
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const ROLE: &str = "VOYAGE_OWNED_LIFETIME_CASE_353";
pub fn run(name: &str, body: impl FnOnce()) {
    // module_path includes the crate name; libtest's exact names do not. Refuse
    // a zero-test child rather than treating an empty successful runner as proof.
    let name = name.split_once("::").expect("qualified owned test name").1;
    if let Ok(selected) = std::env::var(ROLE) {
        assert_eq!(selected, name);
        assert!(unsafe { libc::geteuid() } > 0);
        assert_eq!(unsafe { libc::getpgrp() }, unsafe { libc::getpid() });
        assert!(std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").is_none());
        crate::attachment::journal::prepare_directory(crate::config::default_data_dir()).unwrap();
        body();
        return;
    }
    let root = tempfile::Builder::new()
        .prefix("lifetime-353-")
        .tempdir()
        .unwrap();
    let log = |name: &str| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.path().join(name))
            .unwrap()
    };
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(ROLE, name)
        .env("HOME", root.path())
        .env("PATH", "/usr/bin:/bin")
        .current_dir(root.path())
        .stdin(Stdio::null())
        .stdout(log("stdout-private.log"))
        .stderr(log("stderr-private.log"));
    for (key, leaf) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        let path = root.path().join(leaf);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        command.env(key, path);
    }
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o077);
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline
            || ["stdout-private.log", "stderr-private.log"]
                .iter()
                .any(|p| fs::metadata(root.path().join(p)).unwrap().len() > 1024 * 1024)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _evidence = root.keep();
            panic!("owned lifetime child deadline/output exceeded; private evidence retained");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        let _evidence = root.keep();
        panic!("owned lifetime source case failed; private evidence retained");
    }
    let summary = fs::read(root.path().join("stdout-private.log")).unwrap();
    assert!(summary.len() <= 1024 * 1024);
    let summary = String::from_utf8_lossy(&summary);
    if !summary.contains("running 1 test") || !summary.contains("1 passed; 0 failed") {
        let _evidence = root.keep();
        panic!("owned exact test entry did not execute once; private evidence retained");
    }
}
pub fn context() -> crate::tools::ToolContext {
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let mut context = crate::tools::reliability_tests::context(&root);
    context
        .environment
        .insert("HOME".into(), root.to_string_lossy().into_owned());
    context
        .environment
        .insert("PATH".into(), "/usr/bin:/bin".into());
    context
}
pub async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("bounded owned lifetime stage");
}
