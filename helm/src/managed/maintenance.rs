//! Explicit bounded maintenance runs in the voyage executable, never an executor in Helm.
use super::*;
use tokio::io::AsyncReadExt;

pub(super) async fn upgrade(directory: &std::path::Path) -> Result<Value> {
    let args = vec![
        "upgrade-journal".into(),
        "--directory".into(),
        directory.join("journal").into_os_string(),
    ];
    Ok(json!({"event":"journal_upgraded","result":invoke(args).await?}))
}

pub(super) async fn recover(
    directory: &std::path::Path,
    session: Uuid,
    cleanup: Option<Uuid>,
    tools: Option<Uuid>,
    revision: Option<u64>,
) -> Result<Value> {
    let mut args = vec![
        "legacy-recover".into(),
        "--directory".into(),
        directory.as_os_str().to_owned(),
        "--session".into(),
        session.to_string().into(),
    ];
    for (flag, value) in [
        ("--acknowledge-cleanup", cleanup.map(|id| id.to_string())),
        ("--reconcile-tools", tools.map(|id| id.to_string())),
        ("--expected-revision", revision.map(|id| id.to_string())),
    ] {
        if let Some(value) = value {
            args.push(flag.into());
            args.push(value.into());
        }
    }
    invoke(args).await
}

pub(crate) async fn invoke(args: Vec<std::ffi::OsString>) -> Result<Value> {
    let binary = std::env::current_exe()?
        .parent()
        .context("Helm executable directory missing")?
        .join("voyage");
    let mut child = tokio::process::Command::new(binary)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .context("maintenance output missing")?
        .take(65537);
    let mut bytes = Vec::new();
    let operation = async {
        stdout.read_to_end(&mut bytes).await?;
        ensure!(bytes.len() <= 65536, "maintenance response too large");
        ensure!(
            child.wait().await?.success(),
            "maintenance refused; inspect exact legacy session and ensure its previous owner has stopped"
        );
        Ok::<_, anyhow::Error>(())
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), operation).await {
        Ok(result) => result?,
        Err(_) => {
            child.kill().await?;
            anyhow::bail!("maintenance timed out; inspect storage before retrying");
        }
    }
    if bytes.is_empty() {
        Ok(json!({"status":"completed"}))
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}
