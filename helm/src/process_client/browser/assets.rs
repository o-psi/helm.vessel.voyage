//! Versioned local adapter distribution. Dependencies are installed only by explicit human setup.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, time::Duration};
use crate::attachment::local_actor::storage::Directory;

const FILES: &[(&str, &str)] = &[
    ("helper.mjs", include_str!("../../../browser/helper.mjs")),
    ("security.mjs", include_str!("../../../browser/security.mjs")),
    ("index.html", include_str!("../../../browser/index.html")),
    ("app.js", include_str!("../../../browser/app.js")),
    ("app.css", include_str!("../../../browser/app.css")),
    ("package.json", include_str!("../../../browser/package.json")),
    ("package-lock.json", include_str!("../../../browser/package-lock.json")),
];

pub(super) fn root() -> Result<PathBuf> {
    let base = crate::process_client::cli::default_directory();
    let parent = base.parent().context("browser storage parent unavailable")?;
    std::fs::create_dir_all(parent)?;
    let root = parent.join("helm-browser");
    Directory::open(&root)?;
    Ok(root)
}
fn version() -> String {
    let mut hash = Sha256::new();
    for (name, text) in FILES { hash.update(name.as_bytes()); hash.update([0]); hash.update(text.as_bytes()); }
    hex::encode(hash.finalize())[..20].to_owned()
}
pub(super) fn distribution() -> Result<PathBuf> {
    let path = root()?.join(format!("adapter-{}", version()));
    let private = Directory::open(&path)?;
    let _lock = private.lock()?;
    for (name, content) in FILES {
        match private.read_bounded(name, 65_536)? {
            Some(bytes) => ensure!(bytes == content.as_bytes(), "Installed browser adapter differs from the bundled version; refusing replacement"),
            None => private.publish_new(name, content.as_bytes())?,
        }
    }
    private.verify()?;
    Ok(path)
}
pub(super) fn executable() -> Result<PathBuf> {
    // Explicit local-only setting. It never travels to Voyage or comes from a tool argument.
    if let Some(path) = std::env::var_os("HELM_BROWSER_CHROMIUM") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute() && path.is_file(), "HELM_BROWSER_CHROMIUM must name an absolute local Chromium executable");
        return Ok(path);
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if !dir.is_absolute() { continue; }
        for name in ["chromium", "chromium-browser", "google-chrome"] {
            let path = dir.join(name);
            if path.is_file() { return Ok(path); }
        }
    }
    anyhow::bail!("Install Chromium locally or set HELM_BROWSER_CHROMIUM to its absolute executable path")
}
pub(super) fn installed(path: &Path) -> bool {
    path.join("node_modules/playwright-core/package.json").is_file() && path.join("installed.json").is_file()
}
pub(super) async fn setup() -> Result<()> {
    let path = distribution()?;
    let private = Directory::open_existing(&path)?;
    let _lock = private.lock()?;
    println!("Installing the bundled, lockfile-pinned local browser adapter dependency. Browser and provider credentials are not involved.");
    let mut command = tokio::process::Command::new("npm");
    command.args(["ci", "--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund"])
        .current_dir(&path).kill_on_drop(true);
    #[cfg(unix)]
    { command.process_group(0); unsafe { command.pre_exec(|| { libc::umask(0o077); Ok(()) }); } }
    let mut child = command.spawn().context("Install Node.js and npm before running browser setup")?;
    let status = match tokio::time::timeout(Duration::from_secs(120), child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = child.id() { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }
            let _ = child.start_kill();
            tokio::time::timeout(Duration::from_secs(5), child.wait()).await.context("Browser dependency installation cleanup not observed")??;
            anyhow::bail!("Browser dependency installation timed out; no browser was shared");
        }
    };
    ensure!(status.success(), "Browser dependency installation failed; no browser was shared");
    private.publish("installed.json", br#"{"version":1}"#)?;
    println!("Local browser adapter installed. {}", if executable().is_ok() { "Chromium found. In Helm select a voyage and press F6 or use /browser." } else { "Install Chromium or set HELM_BROWSER_CHROMIUM to an absolute path before use." });
    Ok(())
}
