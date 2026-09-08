//! Persist every transition before sending; resolve uncertain submissions without replay.
use super::*;
use anyhow::ensure;

pub(super) async fn advance(
    client: &Client,
    saved: &mut Saved,
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
    let process = saved.process.clone().context("process missing")?;
    if saved.attempted {
        let receipt = client
            .voyage(
                saved.id,
                process.incarnation,
                VoyageCommand::Resolve {
                    command_id: saved.turn,
                    original: saved.submit.clone().map(Box::new),
                },
            )
            .await?;
        if receipt["status"] != "unknown" {
            return retain_receipt(saved, receipt);
        }
        // An unresolved original is never resent, even on an explicit retry.
        return Ok(None);
    }
    if saved.submit.is_none() {
        let snapshot = client
            .voyage(saved.id, process.incarnation, VoyageCommand::Snapshot)
            .await?;
        ensure!(
            snapshot["session_id"] == saved.id.to_string(),
            "snapshot identity mismatch"
        );
        saved.submit = Some(super::super::attachments::prepare(VoyageCommand::Submit {
            command_id: saved.turn,
            expected_revision: snapshot["revision"]
                .as_u64()
                .context("snapshot revision missing")?,
            expires_at_ms: super::super::super::frontend::deadline()?,
            prompt: saved.text.clone(),
        }, &saved.images)?);
    }
    let recovering = saved.attempted;
    saved.attempted = true;
    storage::save(saved)?;
    // attempted + frozen command and bytes were saved before any upload. A crash
    // from here recovers with Resolve only; it never repeats this sequence.
    let receipt = super::super::attachments::upload_then_submit(
        client, saved.id, process.incarnation, saved.turn,
        saved.submit.clone().context("first turn missing")?, &saved.images,
    ).await;
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
