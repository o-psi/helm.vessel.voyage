//! Persist every transition before sending; uncertain submissions are receipt-only
//! until the operator explicitly retries their original immutable command.
use super::*;
use anyhow::ensure;

pub(super) async fn advance(
    client: &Client,
    saved: &mut Saved,
    check_only: bool,
) -> Result<Option<serde_json::Value>> {
    if let Some(receipt) = &saved.receipt {
        return Ok(Some(receipt.clone()));
    }
    if saved.process.is_none() {
        // Start is exactly deduplicated by Vessel. An uncertain admission retains
        // its original identity and can never silently allocate a replacement.
        let recovering = saved.start_attempted;
        saved.start_attempted = true;
        storage::save(saved)?;
        let result = client
            .request(saved.start.clone().context("start identity missing")?)
            .await;
        if let Err(error) = &result
            && !recovering
            && error
                .downcast_ref::<crate::process_client::transport::Refusal>()
                .is_some()
        {
            saved.start = None;
            saved.start_attempted = false;
            storage::save(saved)?;
        }
        let process: ProcessInfo = serde_json::from_value(result?)?;
        ensure!(
            process.session_id == saved.id,
            "creation response identity mismatch"
        );
        saved.process = Some(process);
        storage::save(saved)?;
    }
    let process = saved.process.as_ref().context("process missing")?;
    if saved.attempted {
        let receipt = client
            .forward(
                saved.id,
                process.incarnation,
                RuntimeCommand::Receipt {
                    command_id: saved.turn,
                },
            )
            .await?;
        if receipt["status"] != "unknown" {
            return retain_receipt(saved, receipt);
        }
        if check_only {
            return Ok(None);
        }
    } else if check_only {
        return Ok(None);
    }
    if saved.submit.is_none() {
        let snapshot = client
            .forward(saved.id, process.incarnation, RuntimeCommand::Snapshot)
            .await?;
        ensure!(
            snapshot["session_id"] == saved.id.to_string(),
            "snapshot identity mismatch"
        );
        saved.submit = Some(RuntimeCommand::Submit {
            command_id: saved.turn,
            expected_revision: snapshot["revision"]
                .as_u64()
                .context("snapshot revision missing")?,
            expires_at_ms: super::super::super::frontend::deadline()?,
            prompt: saved.text.clone(),
        });
    }
    let recovering = saved.attempted;
    saved.attempted = true;
    storage::save(saved)?;
    let receipt = client
        .forward(
            saved.id,
            process.incarnation,
            saved.submit.clone().context("first turn missing")?,
        )
        .await;
    match receipt {
        Ok(value) if value["status"] == "unknown" => Ok(None),
        Ok(value) => retain_receipt(saved, value),
        Err(error)
            if !recovering
                && error
                    .downcast_ref::<crate::process_client::transport::Refusal>()
                    .is_some() =>
        {
            // A definite refusal did not admit a turn. Hand off the created owner
            // with its text intact so the user can adjust settings and send again.
            retain_receipt(
                saved,
                serde_json::json!({"status":"rejected", "command_id": saved.turn, "detail": error.to_string()}),
            )
        }
        Err(error) => Err(error),
    }
}

fn retain_receipt(
    saved: &mut Saved,
    receipt: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    ensure!(
        receipt["command_id"] == saved.turn.to_string(),
        "first-send receipt identity mismatch"
    );
    ensure!(
        receipt["status"].as_str().is_some_and(|s| s != "unknown"),
        "first-send receipt status missing"
    );
    saved.receipt = Some(receipt.clone());
    storage::save(saved)?;
    Ok(Some(receipt))
}
