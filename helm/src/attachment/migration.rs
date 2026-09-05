//! Explicit local transfer foundation. No CLI, automatic import, or remote entry.
use crate::{completion::runtime::Coordinator, session::SessionStore};
use anyhow::Result;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct TransferRequest {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub expected_revision: u64,
    pub source_sha256: String,
}

#[derive(Debug)]
pub struct TransferReceipt {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub journal_revision: u64,
    pub duplicate: bool,
}

pub async fn transfer(
    _store: SessionStore,
    _coordinator: Coordinator,
    _journal_directory: PathBuf,
    _request: TransferRequest,
    _cancel: CancellationToken,
) -> Result<TransferReceipt> {
    anyhow::bail!("transfer foundation not implemented")
}

#[cfg(test)]
mod tests;
