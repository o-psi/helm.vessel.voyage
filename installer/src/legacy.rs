//! Quiescent legacy migration: exact snapshot evidence, held ownership, no effect replay.
use crate::install::files;
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Proof {
    pub state: PathBuf,
    pub accounts: PathBuf,
    pub stage: PathBuf,
    pub evidence: serde_json::Value,
}
pub(crate) struct Guard {
    pub proof: Proof,
    _sessions: Vec<File>,
    supervisor: Option<File>,
}
fn inspect(action: &str, state: &Path, accounts: &Path, stage: &Path) -> Result<serde_json::Value> {
    for path in [state, accounts, stage] {
        files::safe(path)?;
    }
    let output = crate::service::command::run(
        Path::new("/usr/bin/python3"),
        &[
            "-I",
            "-c",
            include_str!("legacy_update.py"),
            action,
            state.to_str().context("State path encoding")?,
            accounts.to_str().context("Account path encoding")?,
            stage.to_str().context("Stage path encoding")?,
        ],
        None,
    )?;
    let evidence: serde_json::Value = serde_json::from_slice(&output)?;
    ensure!(
        evidence["sessions"]
            .as_array()
            .is_some_and(|sessions| sessions.len() <= 4096),
        "Legacy snapshot identity unavailable"
    );
    Ok(evidence)
}
pub(crate) fn accounts() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(root) = crate::fixture_tests::root() {
        return Ok(root.join("accounts"));
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME unavailable")?);
    Ok(std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(home.join(".local/share"))
        .join("helm"))
}
pub(crate) fn accounts_for_process(pid: u32) -> Result<PathBuf> {
    #[cfg(test)]
    {
        let _ = pid;
        return accounts();
    }
    #[cfg(not(test))]
    {
        let bytes = files::read(&PathBuf::from(format!("/proc/{pid}/environ")), 1024 * 1024)?;
        let value = |key: &[u8]| {
            bytes.split(|byte| *byte == 0).find_map(|entry| {
                entry.strip_prefix(key).map(|value| {
                    <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(value)
                        .to_os_string()
                })
            })
        };
        let home = value(b"HOME=").context("Supervisor HOME unavailable")?;
        Ok(value(b"XDG_DATA_HOME=")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from(home).join(".local/share"))
            .join("helm"))
    }
}

pub(crate) fn eligible(state: &Path, accounts: &Path, stage: &Path) -> Result<()> {
    inspect("inspect", state, accounts, stage).map(|_| ())
}
pub(crate) fn begin(state: &Path, accounts: &Path, stage: &Path) -> Result<Guard> {
    let supervisor = files::lock(&state.join("supervisor.lock"))?;
    let first = inspect("inspect", state, accounts, stage)?;
    let mut sessions = Vec::new();
    for session in first["sessions"]
        .as_array()
        .context("Legacy session inventory missing")?
    {
        let session = session
            .as_str()
            .context("Legacy session identity missing")?;
        let id: uuid::Uuid = session.parse()?;
        ensure!(!id.is_nil(), "Nil legacy session");
        let directory = state.join("sessions").join(id.to_string());
        sessions.push(files::lock(&directory.join("startup.lock"))?);
        sessions.push(files::lock(
            &directory
                .join("journal")
                .join(format!("{id}.execution.lock")),
        )?);
    }
    let mut evidence = inspect("snapshot", state, accounts, stage)?;
    ensure!(
        first["sessions"] == evidence["sessions"],
        "Legacy ownership inventory changed"
    );
    evidence
        .as_object_mut()
        .context("Legacy evidence missing")?
        .remove("sessions");
    Ok(Guard {
        proof: Proof {
            state: state.to_owned(),
            accounts: accounts.to_owned(),
            stage: stage.to_owned(),
            evidence,
        },
        _sessions: sessions,
        supervisor: Some(supervisor),
    })
}
impl Guard {
    pub fn permit_supervisor(&mut self) {
        self.supervisor.take();
    }
    pub fn verify(&self) -> Result<()> {
        verify(&self.proof)
    }
    pub fn restore(&mut self) -> Result<()> {
        if self.supervisor.is_none() {
            self.supervisor = Some(files::lock(&self.proof.state.join("supervisor.lock"))?);
        }
        restore(&self.proof)
    }
}
pub(crate) fn verify(proof: &Proof) -> Result<()> {
    let observed = inspect("verify", &proof.state, &proof.accounts, &proof.stage)?;
    for key in [
        "canonical_sha256",
        "state_sha256",
        "accounts_sha256",
        "session_count",
    ] {
        ensure!(
            observed[key] == proof.evidence[key],
            "Legacy snapshot proof changed"
        );
    }
    Ok(())
}
pub(crate) fn restore(proof: &Proof) -> Result<()> {
    inspect("restore", &proof.state, &proof.accounts, &proof.stage).map(|_| ())
}
pub(crate) fn fence(state: &Path, operation: &str, previous: &str, candidate: &str) -> Result<()> {
    let path = state.join("update-quarantine.json");
    ensure!(
        !path.try_exists()?,
        "Existing update quarantine requires reconciliation"
    );
    files::atomic_json(
        &path,
        &serde_json::json!({"schema_version":1,"operation_id":operation,"previous_release":previous,"candidate_release":candidate}),
    )
}
pub(crate) fn clear(state: &Path, operation: &str) -> Result<()> {
    let path = state.join("update-quarantine.json");
    let value: serde_json::Value = serde_json::from_slice(&files::read(&path, 4096)?)?;
    ensure!(
        value["operation_id"] == operation,
        "Quarantine identity changed"
    );
    std::fs::remove_file(&path)?;
    files::sync(&path)
}

#[cfg(test)]
#[path = "legacy_tests.rs"]
mod tests;
