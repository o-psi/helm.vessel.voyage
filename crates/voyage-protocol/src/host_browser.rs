//! Executing-host browser control. Distinct from the opt-in local Browser executor.
use crate::process::ProcessRight;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_SDP_BYTES: usize = 64 * 1024;
pub const MAX_ICE_BYTES: usize = 8 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

/// Private transport provenance; never accepted inside a public VoyageCommand.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostBrowserSocket {
    pub socket_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostBrowserBinding {
    pub incarnation: Uuid,
    pub browser_id: Uuid,
    /// Caller supplies a fresh non-nil ID on Attach; IDs never confer authority.
    pub attachment_id: Uuid,
    pub tab_id: Uuid,
    pub document_epoch: u64,
    pub viewport_epoch: u64,
    pub controller_epoch: u64,
    pub capture_epoch: u64,
}
impl HostBrowserBinding {
    pub fn valid(&self) -> bool {
        !self.incarnation.is_nil()
            && !self.browser_id.is_nil()
            && !self.attachment_id.is_nil()
            && !self.tab_id.is_nil()
            && self.document_epoch > 0
            && self.viewport_epoch > 0
            && self.controller_epoch > 0
            && self.capture_epoch > 0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostBrowserSignal {
    RequestOffer,
    Answer {
        sdp: String,
    },
    Ice {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
}
impl HostBrowserSignal {
    pub fn valid(&self) -> bool {
        match self {
            Self::RequestOffer => true,
            Self::Answer { sdp } => !sdp.is_empty() && sdp.len() <= MAX_SDP_BYTES,
            Self::Ice {
                candidate, sdp_mid, ..
            } => {
                candidate.len() <= MAX_ICE_BYTES && sdp_mid.as_ref().is_none_or(|s| s.len() <= 256)
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostBrowserButton {
    Left,
    Middle,
    Right,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostBrowserInput {
    Pointer {
        x: u32,
        y: u32,
        button: Option<HostBrowserButton>,
        pressed: bool,
    },
    Key {
        key: String,
        pressed: bool,
    },
    Text {
        text: String,
    },
    Scroll {
        delta_x: i32,
        delta_y: i32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Dialog {
        accept: bool,
        text: Option<String>,
    },
    Navigate {
        url: String,
    },
    Tab {
        operation: HostBrowserTabOperation,
        tab_id: Option<Uuid>,
    },
}
impl HostBrowserInput {
    pub fn valid(&self) -> bool {
        match self {
            Self::Pointer { x, y, .. } => *x <= 16384 && *y <= 16384,
            Self::Key { key, .. } => {
                !key.is_empty() && key.len() <= 128 && !key.chars().any(char::is_control)
            }
            Self::Text { text } => !text.is_empty() && text.len() <= MAX_TEXT_BYTES,
            Self::Resize { width, height } => {
                (320..=3840).contains(width) && (240..=2160).contains(height)
            }
            Self::Dialog { text, .. } => text.as_ref().is_none_or(|s| s.len() <= MAX_TEXT_BYTES),
            Self::Navigate { url } => {
                url.len() <= 8192
                    && !url.chars().any(char::is_control)
                    && (url.starts_with("https://") || url.starts_with("http://"))
            }
            Self::Tab { operation, tab_id } => match operation {
                HostBrowserTabOperation::New => tab_id.is_none(),
                HostBrowserTabOperation::Select | HostBrowserTabOperation::Close => {
                    tab_id.is_some_and(|id| !id.is_nil())
                }
            },
            Self::Scroll { delta_x, delta_y } => {
                delta_x.unsigned_abs() <= 16384 && delta_y.unsigned_abs() <= 16384
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostBrowserControlMode {
    Agent,
    Human,
    Private,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostBrowserTabOperation {
    New,
    Select,
    Close,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostBrowserOperation {
    Status,
    Start {
        command_id: Uuid,
        expected_revision: u64,
        incarnation: Uuid,
    },
    Attach {
        command_id: Uuid,
        binding: HostBrowserBinding,
    },
    Detach {
        command_id: Uuid,
        binding: HostBrowserBinding,
    },
    Control {
        command_id: Uuid,
        binding: HostBrowserBinding,
        mode: HostBrowserControlMode,
    },
    Signal {
        command_id: Uuid,
        binding: HostBrowserBinding,
        signal: HostBrowserSignal,
    },
    Input {
        command_id: Uuid,
        binding: HostBrowserBinding,
        sequence: u64,
        input: HostBrowserInput,
    },
    Receipt {
        command_id: Uuid,
    },
    Close {
        command_id: Uuid,
        binding: HostBrowserBinding,
    },
}
impl HostBrowserOperation {
    pub fn required_right(&self) -> ProcessRight {
        match self {
            Self::Start { .. } | Self::Control { .. } | Self::Input { .. } | Self::Close { .. } => {
                ProcessRight::Execute
            }
            _ => ProcessRight::Observe,
        }
    }
    /// Exact IDs for effects; Signal/Input receipts belong to a bounded live
    /// attachment ledger, never the process lifetime durable tombstone store.
    pub fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Status | Self::Receipt { .. } => None,
            Self::Start { command_id, .. }
            | Self::Attach { command_id, .. }
            | Self::Detach { command_id, .. }
            | Self::Control { command_id, .. }
            | Self::Signal { command_id, .. }
            | Self::Input { command_id, .. }
            | Self::Close { command_id, .. } => Some(*command_id),
        }
    }
    pub fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Signal { .. } | Self::Input { .. })
    }
    pub fn binding(&self) -> Option<&HostBrowserBinding> {
        match self {
            Self::Attach { binding, .. }
            | Self::Detach { binding, .. }
            | Self::Control { binding, .. }
            | Self::Signal { binding, .. }
            | Self::Input { binding, .. }
            | Self::Close { binding, .. } => Some(binding),
            _ => None,
        }
    }
    /// Structural validation only. Runtime must also validate live ownership,
    /// principal/socket/epoch identity and sequence before every effect.
    pub fn valid(&self) -> bool {
        if self.mutation_id().is_some_and(|id| id.is_nil())
            || self.binding().is_some_and(|b| !b.valid())
        {
            return false;
        }
        match self {
            Self::Start { incarnation, .. } => !incarnation.is_nil(),
            Self::Receipt { command_id } => !command_id.is_nil(),
            Self::Signal { signal, .. } => signal.valid(),
            Self::Input {
                sequence, input, ..
            } => *sequence > 0 && input.valid(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_and_private_identity() {
        assert!(
            !HostBrowserInput::Scroll {
                delta_x: i32::MIN,
                delta_y: 0
            }
            .valid()
        );
        assert!(
            !HostBrowserInput::Text {
                text: "a".repeat(MAX_TEXT_BYTES + 1)
            }
            .valid()
        );
        assert!(
            !HostBrowserSignal::Answer {
                sdp: "a".repeat(MAX_SDP_BYTES + 1)
            }
            .valid()
        );
        assert!(serde_json::from_str::<crate::vessel::VoyageCommand>(r#"{"op":"host_browser","operation":{"action":"status"},"socket":{"socket_id":"00000000-0000-0000-0000-000000000001"}}"#).is_err());
        assert!(
            serde_json::from_str::<HostBrowserOperation>(
                r#"{"action":"status","socket_id":"spoof"}"#
            )
            .is_err()
        );
    }
    #[test]
    fn rights_ids_and_round_trip() {
        let id = Uuid::new_v4();
        let op = HostBrowserOperation::Start {
            command_id: id,
            expected_revision: 0,
            incarnation: Uuid::new_v4(),
        };
        assert!(op.valid());
        assert_eq!(op.required_right(), ProcessRight::Execute);
        assert_eq!(op.mutation_id(), Some(id));
        assert_eq!(
            serde_json::from_value::<HostBrowserOperation>(serde_json::to_value(&op).unwrap())
                .unwrap(),
            op
        );
        assert_eq!(
            HostBrowserOperation::Receipt { command_id: id }.mutation_id(),
            None
        );
        assert_eq!(
            HostBrowserOperation::Status.required_right(),
            ProcessRight::Observe
        );
    }
    #[test]
    fn attachment_fences_and_strict_input_shapes() {
        let id = Uuid::new_v4();
        let binding = HostBrowserBinding {
            incarnation: id,
            browser_id: id,
            attachment_id: id,
            tab_id: id,
            document_epoch: 1,
            viewport_epoch: 1,
            controller_epoch: 1,
            capture_epoch: 1,
        };
        let mut invalid = binding.clone();
        invalid.attachment_id = Uuid::nil();
        assert!(
            !HostBrowserOperation::Attach {
                command_id: id,
                binding: invalid
            }
            .valid()
        );
        for mode in [
            HostBrowserControlMode::Agent,
            HostBrowserControlMode::Human,
            HostBrowserControlMode::Private,
        ] {
            let op = HostBrowserOperation::Control {
                command_id: id,
                binding: binding.clone(),
                mode,
            };
            assert!(op.valid());
            assert_eq!(op.required_right(), ProcessRight::Execute);
            assert_eq!(
                serde_json::from_value::<HostBrowserOperation>(serde_json::to_value(&op).unwrap())
                    .unwrap(),
                op
            );
        }
        assert!(
            !HostBrowserOperation::Input {
                command_id: id,
                binding,
                sequence: 0,
                input: HostBrowserInput::Text { text: "x".into() }
            }
            .valid()
        );
        assert!(
            HostBrowserInput::Text {
                text: "x".repeat(MAX_TEXT_BYTES)
            }
            .valid()
        );
        assert!(
            !HostBrowserInput::Resize {
                width: 3841,
                height: 240
            }
            .valid()
        );
        assert!(
            !HostBrowserInput::Tab {
                operation: HostBrowserTabOperation::New,
                tab_id: Some(id)
            }
            .valid()
        );
        assert!(
            !HostBrowserInput::Navigate {
                url: "file:///etc/passwd".into()
            }
            .valid()
        );
        assert!(
            serde_json::from_str::<HostBrowserSignal>(r#"{"type":"offer","sdp":"x"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<HostBrowserInput>(r#"{"type":"text","text":"x","script":"x"}"#)
                .is_err()
        );
    }
}
