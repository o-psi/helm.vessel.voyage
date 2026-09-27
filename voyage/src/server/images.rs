//! Binary acquisition is separately idempotent; no image bytes enter command journals.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub(super) async fn upload(
    state: &Arc<State>,
    authorization: super::authorization::Authorization,
    id: Uuid,
    name: String,
    data: String,
) -> Result<Value> {
    ensure!(!id.is_nil(), "invalid image upload identity");
    ensure!(
        data.len() <= (crate::images::MAX_BYTES).div_ceil(3) * 4,
        "image upload exceeds limit"
    );
    let _admission = state.admission.lock().await;
    ensure!(!state.shutdown.is_cancelled(), "runtime stopping");
    if let Some(authority) = &authorization.authority {
        authority.check()?;
    }
    let config = state.config.read().await.clone();
    crate::policy::Policy::new(&config, state.registration.workspace.clone())?.check_current()?;
    let status = state.owner.snapshot().await?;
    ensure!(
        status.session.id == state.registration.session_id,
        "image session mismatch"
    );
    let bytes = STANDARD
        .decode(&data)
        .map_err(|_| anyhow::anyhow!("image upload is not canonical base64"))?;
    ensure!(
        bytes.len() <= crate::images::MAX_BYTES && STANDARD.encode(&bytes) == data,
        "image upload is not bounded canonical base64"
    );
    let attachment = state
        .owner
        .put_image(
            authorization.actor.principal_id,
            id,
            name,
            bytes,
            authorization.authority,
        )
        .await?;
    Ok(serde_json::to_value(attachment)?)
}
