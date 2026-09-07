//! Authenticate the endpoint and preserve previously observed live runtime identities.
use super::command;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn request(bin: &Path, state: &Path, command: Value) -> Result<Value> {
    let bytes = serde_json::to_vec(&json!({"protocol":1,"command":command}))?;
    let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
    frame.extend(bytes);
    let output = command::run(
        &bin.join("vessel"),
        &[
            "local-request",
            "--directory",
            state.to_str().context("state path requires UTF-8")?,
        ],
        Some(&frame),
    )?;
    ensure!(output.len() >= 4, "Vessel returned no frame");
    let length = u32::from_be_bytes(output[..4].try_into()?) as usize;
    ensure!(
        length > 0 && length <= 4 * 1024 * 1024 && output.len() == length + 4,
        "invalid Vessel readiness frame"
    );
    let reply: Value = serde_json::from_slice(&output[4..])?;
    ensure!(
        reply["protocol"] == 1 && reply["error"].is_null(),
        "Vessel process protocol incompatible or unavailable"
    );
    Ok(reply["result"].clone())
}
pub(super) fn catalogue(bin: &Path, state: &Path) -> Result<Vec<Value>> {
    let capabilities = request(bin, state, json!({"op":"capabilities"}))?;
    ensure!(
        capabilities["protocol"] == 1,
        "unsupported existing process protocol; explicit migration required"
    );
    let entries = request(bin, state, json!({"op":"catalogue"}))?;
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
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if super::command::query("ActiveState")? == "active"
            && super::command::query("MainPID")
                .ok()
                .and_then(|pid| pid.parse::<u32>().ok())
                .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
                .is_some_and(|path| path == bin.join("vessel"))
            && prior_invocation.is_none_or(|prior| {
                super::command::query("InvocationID")
                    .is_ok_and(|current| !current.is_empty() && current != prior)
            })
            && let Ok(current) = catalogue(bin, state)
        {
            for previous in prior.iter().filter(|v| v["state"] == "live") {
                ensure!(
                    current
                        .iter()
                        .any(|now| now["session_id"] == previous["session_id"]
                            && now["incarnation"] == previous["incarnation"]
                            && matches!(now["state"].as_str(), Some("live" | "suspended"))),
                    "A previously live voyage did not confirm its original incarnation live or cleanly suspended after supervisor upgrade; no voyage restart was attempted"
                );
            }
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Service did not expose an authenticated ready Vessel endpoint; inspect journalctl --user -u voyage-vessel.service"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
