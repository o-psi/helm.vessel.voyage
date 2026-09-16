//! Authenticate the endpoint and preserve previously observed live runtime identities.
use super::command;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
pub(super) fn catalogue(bin: &Path, state: &Path) -> Result<Vec<Value>> {
    // Reuse the public HTTP client: it validates private discovery, bearer
    // authentication and the wire version without exposing credentials here.
    // Observation must never implicitly start a missing supervisor.
    let output = command::run(
        &bin.join("helm"),
        &[
            "connect",
            "--no-start",
            "--directory",
            state.to_str().context("state path requires UTF-8")?,
            "list",
        ],
        None,
    )?;
    let entries: Value = serde_json::from_slice(&output)?;
    Ok(entries
        .as_array()
        .context("invalid Vessel catalogue")?
        .clone())
}
pub(super) fn wait(
    bin: &Path,
    state: &Path,
    prior: &[Value],
    prior_invocation: Option<&str>,
) -> Result<()> {
    wait_for(bin, state, prior, prior_invocation, Duration::from_secs(15))
}
fn wait_for(
    bin: &Path,
    state: &Path,
    prior: &[Value],
    prior_invocation: Option<&str>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if super::command::query("ActiveState")? == "active"
            && super::command::query("MainPID")
                .ok()
                .and_then(|pid| pid.parse::<u32>().ok())
                .and_then(|pid| {
                    {
                        #[cfg(test)]
                        {
                            crate::fixture_tests::executable(pid)
                        }
                        #[cfg(not(test))]
                        {
                            std::fs::read_link(format!("/proc/{pid}/exe"))
                        }
                    }
                    .ok()
                })
                .is_some_and(|path| path == bin.join("vessel"))
            && prior_invocation.is_none_or(|prior| {
                super::command::query("InvocationID")
                    .is_ok_and(|current| !current.is_empty() && current != prior)
            })
            && let Ok(current) = catalogue(bin, state)
        {
            let retained = prior
                .iter()
                .filter(|v| v["state"] == "live")
                .all(|previous| {
                    current.iter().any(|now| {
                        now["session_id"] == previous["session_id"]
                            && now["incarnation"] == previous["incarnation"]
                            && matches!(now["state"].as_str(), Some("live" | "suspended"))
                    })
                });
            if retained {
                return Ok(());
            }
        }
        ensure!(
            Instant::now() < deadline,
            "Service did not expose an authenticated ready Vessel endpoint with every original incarnation live or cleanly suspended; inspect journalctl --user -u voyage-vessel.service"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
