//! Explicitly shared local browser. This protocol carries no JavaScript, CDP or host paths.
//! Browser operations use the authenticated command connection, never a second transport.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Full outstanding/payload-bearing entries, not a lifetime receipt count.
pub const MAX_BROWSER_REQUESTS: usize = 128;
pub const MAX_BROWSER_RESULT_BYTES: usize = 3 * 1024 * 1024;
pub const MAX_BROWSER_LEASE_MS: u64 = 60_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserBinding {
    pub session_id: Uuid,
    pub incarnation: Uuid,
    /// None is an idle offer; dispatched requests always carry Some(current run).
    pub run_id: Option<Uuid>,
    pub browser_id: Uuid,
    pub resource_id: Uuid,
    pub executor_id: Uuid,
    pub controller_epoch: u64,
    pub capture_epoch: u64,
    pub expires_at_ms: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserControl {
    Shared,
    Human,
    Private,
    Disconnected,
    Revoked,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserTarget {
    pub page_id: Uuid,
    pub observation_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAction {
    Inspect {
        page_id: Option<Uuid>,
    },
    Navigate {
        target: BrowserTarget,
        url: String,
    },
    Click {
        target: BrowserTarget,
        element: String,
    },
    Fill {
        target: BrowserTarget,
        element: String,
        text: String,
    },
    Scroll {
        target: BrowserTarget,
        delta_x: i32,
        delta_y: i32,
    },
    Tabs {
        operation: BrowserTabs,
    },
    Screenshot {
        target: BrowserTarget,
    },
    Upload {
        target: BrowserTarget,
        element: String,
        grant_id: Uuid,
    },
    /// Explicit remote disclosure to private local staging, not a browser upload yet.
    UploadPrepare {
        transfer_id: Uuid,
        name: String,
        mime_type: String,
        data_base64: String,
    },
    Download {
        target: BrowserTarget,
        download_id: Uuid,
    },
}
impl BrowserAction {
    pub fn observation_only(&self) -> bool {
        matches!(
            self,
            Self::Inspect { .. }
                | Self::Screenshot { .. }
                | Self::Tabs {
                    operation: BrowserTabs::List {}
                }
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserTabs {
    List {},
    Open { url: String },
    Select { target: BrowserTarget },
    Close { target: BrowserTarget },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRequest {
    pub request_id: Uuid,
    pub binding: BrowserBinding,
    pub action: BrowserAction,
    pub action_sha256: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRequestState {
    Pending,
    Dispatched,
    Completed,
    Refused,
    Cancelled,
    Unresolved,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserReceipt {
    pub request_id: Uuid,
    pub action_sha256: String,
    pub state: BrowserRequestState,
    pub cleanup_pending: bool,
}
/// Observations are untrusted web content, not instructions. Raster data uses the existing
/// tool-result ingestion pipeline; never accept arbitrary server filesystem references.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserResult {
    pub request_id: Uuid,
    pub action_sha256: String,
    pub state: BrowserRequestState,
    pub text: String,
    pub page_id: Option<Uuid>,
    pub observation_id: Option<Uuid>,
    pub image: Option<BrowserImage>,
    #[serde(default)]
    pub file: Option<BrowserFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserFile {
    pub name: String,
    pub mime_type: String,
    pub data_base64: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserImage {
    pub mime_type: String,
    pub data_base64: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserOperation {
    Offer {
        command_id: Uuid,
        binding: BrowserBinding,
    },
    Pending {
        binding: BrowserBinding,
        limit: u16,
    },
    Claim {
        command_id: Uuid,
        binding: BrowserBinding,
        request_id: Uuid,
        action_sha256: String,
    },
    Result {
        command_id: Uuid,
        binding: BrowserBinding,
        result: BrowserResult,
    },
    Control {
        command_id: Uuid,
        binding: BrowserBinding,
        control: BrowserControl,
    },
    Receipt {
        binding: BrowserBinding,
        request_id: Uuid,
    },
    Cleanup {
        command_id: Uuid,
        binding: BrowserBinding,
        request_id: Uuid,
        observed: bool,
    },
}
impl BrowserOperation {
    pub fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Offer { command_id, .. }
            | Self::Claim { command_id, .. }
            | Self::Result { command_id, .. }
            | Self::Control { command_id, .. }
            | Self::Cleanup { command_id, .. } => Some(*command_id),
            _ => None,
        }
    }
    pub fn binding(&self) -> &BrowserBinding {
        match self {
            Self::Offer { binding, .. }
            | Self::Pending { binding, .. }
            | Self::Claim { binding, .. }
            | Self::Result { binding, .. }
            | Self::Control { binding, .. }
            | Self::Receipt { binding, .. }
            | Self::Cleanup { binding, .. } => binding,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserStatus {
    pub binding: BrowserBinding,
    pub control: BrowserControl,
    pub available: bool,
    pub cleanup_pending: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserReply {
    Status { status: BrowserStatus },
    Pending { requests: Vec<BrowserRequest> },
    Receipt { receipt: BrowserReceipt },
}
