//! Persist every transition before sending; resolve uncertain submissions without replay.
use super::*;
use anyhow::ensure;

pub(super) async fn advance(
    client: &Client,
    saved: &mut Saved,
) -> Result<Option<serde_json::Value>> {
    advance_mode(client, saved, false).await
}

pub(super) async fn observe(
    client: &Client,
    saved: &mut Saved,
) -> Result<Option<serde_json::Value>> {
    advance_mode(client, saved, true).await
}

async fn advance_mode(
    client: &Client,
    saved: &mut Saved,
    observe_only: bool,
) -> Result<Option<serde_json::Value>> {
    if let Some(receipt) = &saved.receipt {
        return Ok(Some(receipt.clone()));
    }
    if let Some(host) = saved.account_host {
        let caps = client.request(VesselCommand::Capabilities).await?;
        ensure!(
            caps["vessel_id"] == host.to_string(),
            "Authenticated account host changed; original draft retained"
        );
    }
    if saved.process.is_none() {
        // Recovery is observation only, not even an exact Start replay. A
        // registration may establish that the intended session now exists; an
        // absent registration cannot establish that an uncertain Start failed.
        let result = if saved.start_attempted {
            let capabilities = client.request(VesselCommand::Capabilities).await?;
            if capabilities["features"]
                .as_array()
                .is_some_and(|features| features.iter().any(|f| f == "start_resolution"))
            {
                let original = saved
                    .start
                    .as_ref()
                    .context("original creation envelope missing; recovery retained")?;
                let (command_id, resolve) = match original {
                    VesselCommand::StartAccount {
                        command_id,
                        session_id,
                        workspace,
                        account,
                        model,
                        reasoning_effort,
                        service_tier,
                    } => (
                        *command_id,
                        VesselCommand::ResolveStartAccount {
                            command_id: *command_id,
                            session_id: *session_id,
                            workspace: workspace.clone(),
                            account: account.clone(),
                            model: model.clone(),
                            reasoning_effort: reasoning_effort.clone(),
                            service_tier: service_tier.clone(),
                        },
                    ),
                    VesselCommand::Start {
                        command_id,
                        workspace,
                        ..
                    } => (
                        *command_id,
                        VesselCommand::ResolveStart {
                            command_id: *command_id,
                            session_id: saved.id,
                            workspace: workspace.clone(),
                            config_path: None,
                        },
                    ),
                    VesselCommand::StartConfigured {
                        command_id,
                        workspace,
                        config_path,
                        ..
                    } => (
                        *command_id,
                        VesselCommand::ResolveStart {
                            command_id: *command_id,
                            session_id: saved.id,
                            workspace: workspace.clone(),
                            config_path: Some(config_path.clone()),
                        },
                    ),
                    _ => anyhow::bail!("Unsupported original creation envelope; recovery retained"),
                };
                let resolution = client.request(resolve).await?;
                ensure!(
                    resolution["command_id"] == command_id.to_string()
                        && resolution["session_id"] == saved.id.to_string(),
                    "creation resolution identity mismatch"
                );
                match resolution["status"].as_str() {
                    Some("created") => Ok(resolution["process"].clone()),
                    Some("not_admitted") => {
                        // The original ID is durably fenced. Only a subsequent
                        // explicit send may allocate another creation command.
                        saved.start = None;
                        saved.start_attempted = false;
                        storage::save(saved)?;
                        anyhow::bail!("Creation was not admitted; your draft is editable again")
                    }
                    Some("unknown") => return Ok(None),
                    _ => anyhow::bail!("invalid creation resolution; original recovery retained"),
                }
            } else {
                // Older servers can confirm an existing owner but cannot prove
                // non-admission from an absent registration. Keep that uncertainty.
                client
                    .request(VesselCommand::Inspect {
                        session_id: saved.id,
                    })
                    .await
            }
        } else {
            if observe_only {
                return Ok(None);
            }
            if let Err(error) = workspaces::validate_live(client, saved).await {
                // Capabilities are read-only and no Start has been attempted.
                // Keep text editable when scope or host readiness needs repair.
                saved.start = None;
                storage::save(saved)?;
                return Err(error);
            }
            saved.start_attempted = true;
            storage::save(saved)?;
            let result = client
                .request(saved.start.clone().context("start identity missing")?)
                .await;
            if result.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<crate::process_client::transport::Refusal>()
                    .is_some()
            }) {
                // A definite first-attempt refusal is the only safe reset.
                saved.start = None;
                saved.start_attempted = false;
                storage::save(saved)?;
            }
            result
        };
        let process: ProcessInfo = serde_json::from_value(result?)?;
        ensure!(
            process.session_id == saved.id
                && (client.is_local() || process.workspace == saved.workspace),
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
                VoyageCommand::Receipt {
                    command_id: saved.turn,
                },
            )
            .await?;
        if receipt["status"] != "unknown" {
            return retain_receipt(saved, receipt);
        }
        // Resolve may durably close an unadmitted ID; it never resubmits the
        // original prompt or uploads. Older owners may refuse this operation,
        // in which case the original uncertainty remains saved.
        let resolved = client
            .voyage(
                saved.id,
                process.incarnation,
                VoyageCommand::Resolve {
                    command_id: saved.turn,
                    original: Some(Box::new(
                        saved
                            .submit
                            .clone()
                            .context("first turn envelope missing")?,
                    )),
                },
            )
            .await?;
        if resolved["status"] != "unknown" {
            return retain_receipt(saved, resolved);
        }
        // An unresolved original is never resent, even on an explicit retry.
        return Ok(None);
    }
    if observe_only {
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
        saved.submit = Some(super::super::attachments::prepare(
            VoyageCommand::Submit {
                coordination: None,
                command_id: saved.turn,
                expected_revision: snapshot["revision"]
                    .as_u64()
                    .context("snapshot revision missing")?,
                expires_at_ms: super::super::super::frontend::deadline()?,
                prompt: saved.text.clone(),
            },
            &super::super::attachments::restore_draft(
                saved.text.clone(),
                saved.markers.clone(),
                &saved.images,
            )?,
            &saved.images,
        )?);
    }
    let recovering = saved.attempted;
    saved.attempted = true;
    storage::save(saved)?;
    // attempted + frozen command and bytes were saved before any upload. A crash
    // from here recovers with Receipt only; it never repeats this sequence.
    let receipt = super::super::attachments::upload_then_submit(
        client,
        saved.id,
        process.incarnation,
        saved.turn,
        saved.submit.clone().context("first turn missing")?,
        &saved.images,
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
