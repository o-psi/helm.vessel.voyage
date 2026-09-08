/// Only a closed classification crosses from preparation into display.
/// Raw HTTP/storage diagnostics are never retained in the panel.
pub(super) enum SetupFailure {
    Expired,
    Revoked,
    Unavailable,
    Identity,
    Version,
    Offline,
    Redirect,
    Invalid,
    Timeout,
}
impl SetupFailure {
    pub(super) fn from_error(error: &anyhow::Error) -> Self {
        use crate::process_client::access::ConnectionFailure;
        match error.downcast_ref::<ConnectionFailure>() {
            Some(ConnectionFailure::Unavailable) => Self::Unavailable,
            Some(ConnectionFailure::Expired) => Self::Expired,
            Some(ConnectionFailure::Revoked) => Self::Revoked,
            Some(ConnectionFailure::Identity) => Self::Identity,
            Some(ConnectionFailure::Version) => Self::Version,
            Some(ConnectionFailure::Offline) => Self::Offline,
            Some(ConnectionFailure::Redirect) => Self::Redirect,
            None => Self::Invalid,
        }
    }
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Unavailable => {
                "Access unavailable: the server combined expiry and revocation. Ask the owner to inspect and renew access."
            }
            Self::Expired => {
                "Access or invitation expired. Ask the executing account owner for renewed access."
            }
            Self::Revoked => {
                "Access revoked or refused. Ask the owner to review the grant; importing cannot broaden authority."
            }
            Self::Identity => {
                "Vessel or principal identity changed. Review a new owner-issued invitation; old pending commands remain separate."
            }
            Self::Version => {
                "Unsupported server or credential version. Upgrade the executing Vessel; conversation grants remain limited."
            }
            Self::Offline => {
                "Vessel offline or secure connection failed. Check DNS, trusted TLS certificate and tunnel/service availability. No TLS bypass is offered."
            }
            Self::Redirect => "HTTPS redirect refused. Use the approved direct Vessel endpoint.",
            Self::Invalid => {
                "Invalid invitation, access file, metadata or private storage. Use invitationUUID.secret, an owner-private access file and HTTPS. Check storage permissions."
            }
            Self::Timeout => {
                "Setup timed out; outcome may be unknown. Recover pending pairing rather than redeeming again."
            }
        }
    }
}
