//! Version-two enrollment proofs. This module authenticates a proof, not replay
//! state: callers must atomically consume challenge IDs and enforce stored epochs,
//! invitations, and transaction state after verification.
//!
//! Signing format: `SIGNING_DOMAIN` followed by compact serde JSON of the typed
//! `Challenge`, in the declaration order below (including operation fields).
//! UUIDs use serde's lowercase hyphenated representation; keys are byte arrays.
//! This is a fixed-schema canonical encoding, not general-purpose JSON JCS.
//! Decode untrusted JSON with `decode` to enforce the wire limit and redact errors.

use ring::{
    rand::SystemRandom,
    signature::{self, Ed25519KeyPair, KeyPair},
};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

pub const PROOF_VERSION: u16 = 2;
pub const MAX_PROOF_BYTES: usize = 16 * 1024;
pub const MAX_ORIGIN_BYTES: usize = 2048;
pub const MAX_CHALLENGE_LIFETIME_MS: i64 = 60_000;
pub const SIGNING_DOMAIN: &[u8] = b"voyage:enrollment-proof:v2\0";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub version: u16,
    pub id: Uuid,
    pub origin: String,
    pub expires_at_ms: i64,
    /// Opaque server MAC; the client signature binds it but does not validate it.
    pub server_tag: Vec<u8>,
    pub operation: ProofOperation,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProofOperation {
    Enroll {
        transaction_id: Uuid,
        invitation_id: Uuid,
        machine_id: Uuid,
        public_key: [u8; 32],
    },
    Connect {
        machine_id: Uuid,
        epoch: u64,
    },
    Rotate {
        machine_id: Uuid,
        epoch: u64,
        transaction_id: Uuid,
        new_public_key: [u8; 32],
    },
    Revoke {
        machine_id: Uuid,
        epoch: u64,
        transaction_id: Uuid,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedChallenge {
    pub challenge: Challenge,
    pub signature: Vec<u8>,
    pub new_signature: Option<Vec<u8>>,
}

// Never include attacker-controlled content or cryptographic material in Debug.
impl fmt::Debug for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Challenge { .. }")
    }
}
impl fmt::Debug for ProofOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProofOperation { .. }")
    }
}
impl fmt::Debug for SignedChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignedChallenge { .. }")
    }
}

/// Deliberately content-free; no wrapped serde or crypto errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofError {
    TooLarge,
    InvalidJson,
    InvalidVersion,
    InvalidOrigin,
    InvalidExpiry,
    InvalidId,
    InvalidEpoch,
    InvalidSignature,
    InvalidNewSignature,
    InvalidKey,
    KeyGeneration,
    Encoding,
}
impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "proof exceeds size limit",
            Self::InvalidJson => "invalid proof JSON",
            Self::InvalidVersion => "unsupported proof version",
            Self::InvalidOrigin => "invalid proof origin",
            Self::InvalidExpiry => "invalid proof expiry",
            Self::InvalidId => "invalid proof identifier",
            Self::InvalidEpoch => "invalid proof epoch",
            Self::InvalidSignature => "invalid proof signature",
            Self::InvalidNewSignature => "invalid new-key signature",
            Self::InvalidKey => "invalid signing key",
            Self::KeyGeneration => "signing key generation failed",
            Self::Encoding => "proof encoding failed",
        })
    }
}
impl std::error::Error for ProofError {}

fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, ProofError> {
    if bytes.len() > MAX_PROOF_BYTES {
        return Err(ProofError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ProofError::InvalidJson)
}

impl Challenge {
    /// Strict structural decode; contextual validation is a separate step.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProofError> {
        decode_json(bytes)
    }

    /// Does not validate time or origin: these require the verifier's context.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, ProofError> {
        let mut bytes = SIGNING_DOMAIN.to_vec();
        serde_json::to_writer(&mut bytes, self).map_err(|_| ProofError::Encoding)?;
        if bytes.len() - SIGNING_DOMAIN.len() > MAX_PROOF_BYTES {
            return Err(ProofError::TooLarge);
        }
        Ok(bytes)
    }

    pub fn validate(&self, expected_origin: &str, now_ms: i64) -> Result<(), ProofError> {
        if self.server_tag.len() != 32 {
            return Err(ProofError::InvalidSignature);
        }
        if self.version != PROOF_VERSION {
            return Err(ProofError::InvalidVersion);
        }
        if self.origin.is_empty()
            || self.origin.len() > MAX_ORIGIN_BYTES
            || self.origin != expected_origin
        {
            return Err(ProofError::InvalidOrigin);
        }
        if now_ms < 0 {
            return Err(ProofError::InvalidExpiry);
        }
        let remaining = self
            .expires_at_ms
            .checked_sub(now_ms)
            .ok_or(ProofError::InvalidExpiry)?;
        if !(1..=MAX_CHALLENGE_LIFETIME_MS).contains(&remaining) {
            return Err(ProofError::InvalidExpiry);
        }
        nonnil(self.id)?;
        match &self.operation {
            ProofOperation::Enroll {
                transaction_id,
                invitation_id,
                machine_id,
                ..
            } => {
                nonnil(*transaction_id)?;
                nonnil(*invitation_id)?;
                nonnil(*machine_id)?;
            }
            ProofOperation::Connect { machine_id, epoch } => {
                nonnil(*machine_id)?;
                valid_epoch(*epoch)?;
            }
            ProofOperation::Rotate {
                machine_id,
                epoch,
                transaction_id,
                ..
            }
            | ProofOperation::Revoke {
                machine_id,
                epoch,
                transaction_id,
            } => {
                nonnil(*machine_id)?;
                nonnil(*transaction_id)?;
                valid_epoch(*epoch)?;
            }
        }
        Ok(())
    }
}
fn nonnil(id: Uuid) -> Result<(), ProofError> {
    if id.is_nil() {
        Err(ProofError::InvalidId)
    } else {
        Ok(())
    }
}
fn valid_epoch(epoch: u64) -> Result<(), ProofError> {
    if (1..=(i64::MAX as u64 - 1)).contains(&epoch) {
        Ok(())
    } else {
        Err(ProofError::InvalidEpoch)
    }
}

impl SignedChallenge {
    pub fn decode(bytes: &[u8]) -> Result<Self, ProofError> {
        decode_json(bytes)
    }

    /// `public_key` must come from trusted state (or the invitation-bound enroll
    /// operation). Enrollment also requires it to match the proposed key.
    pub fn verify(
        &self,
        public_key: &[u8; 32],
        expected_origin: &str,
        now_ms: i64,
    ) -> Result<(), ProofError> {
        self.challenge.validate(expected_origin, now_ms)?;
        if let ProofOperation::Enroll {
            public_key: proposed,
            ..
        } = &self.challenge.operation
            && proposed != public_key
        {
            return Err(ProofError::InvalidKey);
        }
        let bytes = self.challenge.signing_bytes()?;
        verify_signature(public_key, &bytes, &self.signature)
            .map_err(|_| ProofError::InvalidSignature)?;
        match (&self.challenge.operation, &self.new_signature) {
            (ProofOperation::Rotate { new_public_key, .. }, Some(sig)) => {
                verify_signature(new_public_key, &bytes, sig)
                    .map_err(|_| ProofError::InvalidNewSignature)
            }
            (ProofOperation::Rotate { .. }, None) | (_, Some(_)) => {
                Err(ProofError::InvalidNewSignature)
            }
            (_, None) => Ok(()),
        }
    }
}
fn verify_signature(key: &[u8; 32], bytes: &[u8], sig: &[u8]) -> Result<(), ()> {
    if sig.len() != 64 {
        return Err(());
    }
    signature::UnparsedPublicKey::new(&signature::ED25519, key)
        .verify(bytes, sig)
        .map_err(|_| ())
}

/// Private PKCS#8 material. Intentionally implements neither Debug nor Serialize.
/// Callers are responsible for secure storage of exported private bytes.
pub struct SigningKey {
    pkcs8: Vec<u8>,
}
impl SigningKey {
    pub fn generate() -> Result<Self, ProofError> {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
            .map_err(|_| ProofError::KeyGeneration)?;
        Self::from_pkcs8(document.as_ref())
    }
    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, ProofError> {
        Ed25519KeyPair::from_pkcs8(bytes).map_err(|_| ProofError::InvalidKey)?;
        Ok(Self {
            pkcs8: bytes.to_vec(),
        })
    }
    pub fn as_pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }
    pub fn public_key(&self) -> [u8; 32] {
        // Private, immutable bytes are validated at construction.
        let pair = Ed25519KeyPair::from_pkcs8(&self.pkcs8).expect("validated signing key");
        pair.public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public key length")
    }
    /// Returns a detached signature. For rotation call on both old and new keys
    /// with the identical challenge and populate both SignedChallenge fields.
    pub fn sign(&self, challenge: &Challenge) -> Result<Vec<u8>, ProofError> {
        let pair = Ed25519KeyPair::from_pkcs8(&self.pkcs8).map_err(|_| ProofError::InvalidKey)?;
        Ok(pair.sign(&challenge.signing_bytes()?).as_ref().to_vec())
    }
}

impl Drop for SigningKey {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.pkcs8.zeroize();
    }
}

#[cfg(test)]
mod tests;

pub mod inspection;
