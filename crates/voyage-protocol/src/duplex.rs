//! Public full-duplex Vessel transport. Never contains private runtime envelopes.
//! Correlation IDs are socket-local and are NOT durable command identities.
use crate::vessel::{VesselEvent, VesselEventRequest, VesselRequest, VesselResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const SOCKET_PATH: &str = "/v1/vessel/socket";
pub const SUBPROTOCOL: &str = "voyage.vessel.v1";
pub const MAX_FRAME_BYTES: usize = crate::vessel::MAX_VESSEL_BODY;
pub const MAX_IN_FLIGHT: usize = 32;
pub const MAX_SUBSCRIPTIONS: usize = 256;
pub const HEARTBEAT_SECONDS: u64 = 5;
pub const DEADLINE_SECONDS: u64 = 15;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientFrame {
    Command {
        request_id: Uuid,
        request: VesselRequest,
    },
    Subscribe {
        request_id: Uuid,
        request: VesselEventRequest,
    },
    Unsubscribe {
        subscription_id: Uuid,
    },
    ReverseReply {
        request_id: Uuid,
        reply: ReverseReply,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerFrame {
    Hello {
        protocol: u32,
        socket_id: Uuid,
        vessel_id: Uuid,
    },
    Reply {
        request_id: Uuid,
        response: VesselResponse,
    },
    Subscribed {
        request_id: Uuid,
    },
    Event {
        subscription_id: Uuid,
        event: VesselEvent,
    },
    ReverseRequest {
        request_id: Uuid,
        request: ReverseRequest,
    },
}

/// Notification and acknowledgement only. Browser actions remain explicit public
/// commands with runtime-owned admission, claim and receipt semantics. This is
/// not an arbitrary reverse tool executor, remote browser endpoint or relay.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReverseRequest {
    BrowserWork { session_id: Uuid, incarnation: Uuid },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReverseReply {
    Accepted,
    Unavailable,
}
