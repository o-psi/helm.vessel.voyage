//! Stable signing identities; courier-provided keys are never trusted implicitly.
use super::{access::store, registry};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Serialize, Deserialize)]
struct IdentityKey {
    vessel_id: Uuid,
    pkcs8: String,
}
fn key(root: &Path) -> Result<(VesselIdentity, Ed25519KeyPair)> {
    let directory = root.join("identity");
    registry::private_directory(&directory)?;
    let path = directory.join("key.json");
    let saved: IdentityKey = if path.exists() {
        store::load(&path)?
    } else {
        let document = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .map_err(|_| anyhow::anyhow!("identity generation failed"))?;
        let saved = IdentityKey {
            vessel_id: Uuid::new_v4(),
            pkcs8: STANDARD.encode(document.as_ref()),
        };
        store::save(&path, &saved)?;
        saved
    };
    let bytes = STANDARD.decode(&saved.pkcs8)?;
    let key = Ed25519KeyPair::from_pkcs8(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid signing identity"))?;
    let public = VesselIdentity {
        vessel_id: saved.vessel_id,
        public_key: STANDARD.encode(key.public_key().as_ref()),
    };
    store::save(&directory.join("public.json"), &public)?;
    Ok((public, key))
}
pub(super) fn public(root: &Path) -> Result<VesselIdentity> {
    Ok(key(root)?.0)
}
pub(super) fn sign<T: Serialize>(root: &Path, payload: T) -> Result<SignedArtifact<T>> {
    let (_, key) = key(root)?;
    let mut bytes = b"voyage/process-transfer/v1\0".to_vec();
    bytes.extend(serde_json::to_vec(&payload)?);
    Ok(SignedArtifact {
        payload,
        signature: STANDARD.encode(key.sign(&bytes).as_ref()),
    })
}
pub(super) fn pin(root: &Path, identity: &VesselIdentity) -> Result<()> {
    ensure!(
        !identity.vessel_id.is_nil() && STANDARD.decode(&identity.public_key)?.len() == 32,
        "invalid peer signing identity"
    );
    let directory = root.join("trusted-vessels");
    registry::private_directory(&directory)?;
    let path = directory.join(format!("{}.json", identity.vessel_id));
    if path.exists() {
        let prior: VesselIdentity = store::load(&path)?;
        ensure!(
            prior == *identity,
            "peer signing key already pinned differently"
        );
        return Ok(());
    }
    store::save(&path, identity)
}
pub(super) fn verify<T: Serialize>(
    root: &Path,
    peer: Uuid,
    artifact: &SignedArtifact<T>,
) -> Result<()> {
    let identity: VesselIdentity =
        store::load(&root.join("trusted-vessels").join(format!("{peer}.json")))?;
    ensure!(identity.vessel_id == peer, "trusted identity mismatch");
    let mut bytes = b"voyage/process-transfer/v1\0".to_vec();
    bytes.extend(serde_json::to_vec(&artifact.payload)?);
    UnparsedPublicKey::new(&ED25519, STANDARD.decode(identity.public_key)?)
        .verify(&bytes, &STANDARD.decode(&artifact.signature)?)
        .map_err(|_| anyhow::anyhow!("transfer signature rejected"))
}
pub(super) fn digest<T: Serialize>(value: &T) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
