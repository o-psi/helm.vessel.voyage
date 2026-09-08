use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum AdminCommand {
    /// Print this Vessel's public identity for an independent trust exchange.
    Identity,
    /// Pin an independently verified peer identity from a JSON file.
    Trust { identity_file: PathBuf },
    /// Transfer an idle canonical owner using pinned signatures and a durable courier journal.
    Move(MoveArgs),
    /// Accept an explicit subordinate binding from a reviewed JSON file.
    AcceptParticipant {
        binding_file: PathBuf,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Stop admission and explicitly drain or cancel existing subordinate work.
    RemoveParticipant {
        binding_id: Uuid,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        cancel: bool,
    },
    /// Reconcile one canonical parent obligation, including after parent restart.
    Assignment {
        session: Uuid,
        #[arg(long)]
        run: Uuid,
        #[arg(long)]
        assignment: Uuid,
        #[arg(long)]
        participant: String,
        #[arg(long)]
        cancel: bool,
    },
    /// Recover an unavailable owner under its OS fence; attest only exact resource identities.
    Recover {
        session: Uuid,
        #[arg(long)]
        incarnation: Uuid,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        acknowledge_cleanup: Option<Uuid>,
        #[arg(long)]
        reconcile_tools: Option<Uuid>,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long = "acknowledge-resource")]
        acknowledge_resources: Vec<Uuid>,
    },
}

#[derive(Args, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MoveArgs {
    pub session: Uuid,
    #[arg(long)]
    pub incarnation: Uuid,
    #[arg(long)]
    pub expected_revision: u64,
    /// Absolute state directory of a second local Vessel.
    #[arg(long)]
    pub destination_directory: PathBuf,
    /// Destination-local workspace; no source filesystem paths are imported.
    #[arg(long)]
    pub workspace: PathBuf,
    #[arg(long)]
    pub config_path: Option<PathBuf>,
    /// Private local operation journal. Repeat this exact command to resume it.
    #[arg(long)]
    pub journal: PathBuf,
}
