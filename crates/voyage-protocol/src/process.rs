//! Versioned private process transport. Authentication remains adapter-owned.
mod codec;
mod types;
pub use codec::*;
pub use types::*;

impl RuntimeCommand {
    /// Durable reads may use an exclusively fenced helper after an unclean exit.
    /// This deliberately excludes controls, decisions and mutating resolution.
    pub fn observes_saved(&self) -> bool {
        matches!(
            self,
            Self::Snapshot
                | Self::History { .. }
                | Self::MessageChunk { .. }
                | Self::RunOutput { .. }
                | Self::ReadArtifact { .. }
                | Self::Receipt { .. }
                | Self::Events { .. }
                | Self::NotificationEvents { .. }
        )
    }
}

mod lifecycle;
pub use lifecycle::*;

mod access;
pub use access::*;
mod transfer;
pub use transfer::*;
mod participation;
pub use participation::*;

// Executing-host management adapters also use the public service API.
pub use crate::vessel::{
    VesselCommand, VesselEvent, VesselEventRequest, VesselEventSubscription, VesselRequest,
    VesselResponse,
};

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn notification_read_is_private_bounded_shape_and_saved_observation() {
        let command: RuntimeCommand =
            serde_json::from_str(r#"{"op":"notification_events","after":9,"limit":128}"#).unwrap();
        assert!(command.observes_saved());
        assert!(command.observes_suspended());
        assert!(command.mutation_id().is_none());
        assert!(required_process_right(&command).is_none());
        assert!(
            serde_json::from_str::<RuntimeCommand>(
                r#"{"op":"notification_events","after":0,"limit":1,"recipient":"foreign"}"#
            )
            .is_err()
        );
        let encoded = serde_json::to_value(command).unwrap();
        assert_eq!(encoded["op"], "notification_events");
        assert_eq!(encoded["after"], 9);
    }
    #[tokio::test]
    async fn notification_command_uses_existing_private_frame_codec() {
        let (mut sender, mut receiver) = tokio::io::duplex(4096);
        let command = RuntimeCommand::NotificationEvents {
            after: 9,
            limit: 128,
        };
        write_frame(&mut sender, &command).await.unwrap();
        let decoded: RuntimeCommand = read_frame(&mut receiver).await.unwrap();
        assert!(matches!(
            decoded,
            RuntimeCommand::NotificationEvents {
                after: 9,
                limit: 128
            }
        ));
    }
}
