//! Bounded, peer-authenticated local transport for the system Vessel gateway.
//! Only typed public operations cross this pipe; supervisor secrets stay root-owned.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{os::linux::net::SocketAddrExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};
use uuid::Uuid;
use voyage_protocol::vessel::MAX_VESSEL_BODY;
use voyage_protocol::vessel::{VesselCommand, VesselResponse};

/// A principal's existing grant, forwarded without a supervisor loopback token.
/// Deliberately no Debug: this contains a live secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantAuth {
    pub expected_vessel_id: Option<Uuid>,
    pub grant_id: Uuid,
    pub token: String,
}

/// The root side constructs private envelopes after peer authentication. The
/// gateway cannot submit one in this wire schema or choose an OS identity.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayRequest {
    Command {
        auth: GrantAuth,
        command: VesselCommand,
    },
    PairPreflight,
    PairRedeem {
        expected_vessel_id: Option<Uuid>,
        request: super::pairing::PairRequest,
    },
    /// One long-lived pipe per external browser controller. The root allocates
    /// its own socket identity and retires it on EOF.
    SocketOpen {
        auth: GrantAuth,
    },
    SocketCommand {
        auth: GrantAuth,
        command: VesselCommand,
    },
}

const DEADLINE: Duration = Duration::from_secs(30);
const MAX_NAME: usize = 80;

fn address(name: &str) -> Result<tokio::net::unix::SocketAddr> {
    ensure!(
        !name.is_empty()
            && name.len() <= MAX_NAME
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "invalid system gateway socket name"
    );
    std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())
        .map(Into::into)
        .context("invalid system gateway socket")
}

/// An abstract socket has no writable filesystem pathname to race or unlink.
/// The root service must bind; an occupied name is a hard startup failure.
pub fn bind_root(name: &str, gateway_uid: u32) -> Result<UnixListener> {
    ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
    ensure!(
        gateway_uid != 0,
        "gateway must have a separate unprivileged UID"
    );
    Ok(UnixListener::bind_addr(&address(name)?)?)
}

/// The kernel credential is checked before a byte of request data is read.
pub async fn accept_gateway(
    listener: &UnixListener,
    gateway_uid: u32,
) -> Result<Option<UnixStream>> {
    let (stream, _) = listener.accept().await?;
    Ok((stream.peer_cred()?.uid() == gateway_uid).then_some(stream))
}

/// The gateway verifies the root peer before sending grants or invitation codes.
pub async fn connect_root(name: &str) -> Result<UnixStream> {
    connect_peer(name, 0).await
}

async fn connect_peer(name: &str, expected_uid: u32) -> Result<UnixStream> {
    let stream = timeout(DEADLINE, UnixStream::connect_addr(&address(name)?)).await??;
    ensure!(
        stream.peer_cred()?.uid() == expected_uid,
        "system supervisor peer identity refused"
    );
    Ok(stream)
}

/// Length-prefix each JSON message. Refuse oversized or partial frames without
/// decoding their content. Callers keep one connection for a browser socket so
/// EOF can retire its exact controller state in the supervisor.
pub async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_VESSEL_BODY,
        "system gateway frame exceeds limit"
    );
    timeout(DEADLINE, async {
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&bytes).await?;
        stream.flush().await
    })
    .await??;
    Ok(())
}

pub async fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<Option<T>> {
    read_frame_inner(stream, true).await
}

/// After SocketOpen, idle is allowed; once a frame starts, its remaining bytes
/// must arrive within the same bounded deadline as other frames.
pub async fn read_frame_idle<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<Option<T>> {
    read_frame_inner(stream, false).await
}

async fn read_frame_inner<T: DeserializeOwned>(
    stream: &mut UnixStream,
    first_deadline: bool,
) -> Result<Option<T>> {
    let mut length = [0u8; 4];
    let first = if first_deadline {
        timeout(DEADLINE, stream.read(&mut length[..1])).await??
    } else {
        stream.read(&mut length[..1]).await?
    };
    if first == 0 {
        return Ok(None);
    }
    timeout(DEADLINE, stream.read_exact(&mut length[1..])).await??;
    let size = u32::from_be_bytes(length) as usize;
    ensure!(
        (1..=MAX_VESSEL_BODY).contains(&size),
        "system gateway frame exceeds limit"
    );
    let mut bytes = vec![0u8; size];
    timeout(DEADLINE, stream.read_exact(&mut bytes)).await??;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// One-shot command and pairing calls keep their own connection. A failed read
/// after sending a mutation leaves the outcome unknown to the caller.
pub async fn exchange(name: &str, request: &GatewayRequest) -> Result<VesselResponse> {
    let mut stream = connect_root(name).await?;
    write_frame(&mut stream, request).await?;
    read_frame(&mut stream)
        .await?
        .ok_or_else(|| anyhow::anyhow!("system supervisor reply unavailable"))
}

/// Browser controllers retain this stream for their whole external socket life.
pub async fn open_socket(name: &str, auth: GrantAuth) -> Result<(UnixStream, Uuid)> {
    let mut stream = connect_root(name).await?;
    write_frame(&mut stream, &GatewayRequest::SocketOpen { auth }).await?;
    let response: VesselResponse = read_frame(&mut stream)
        .await?
        .ok_or_else(|| anyhow::anyhow!("system supervisor socket reply unavailable"))?;
    ensure!(
        response.error.is_none() && !response.outcome_unknown,
        "system supervisor socket refused"
    );
    let socket_id: Uuid = serde_json::from_value(response.result["socket_id"].clone())?;
    ensure!(!socket_id.is_nil(), "invalid supervisor socket identity");
    Ok((stream, socket_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Message {
        id: Uuid,
        text: String,
    }

    #[tokio::test]
    async fn validates_both_kernel_peers_and_round_trips_frames() {
        let name = format!("voyage-gateway-test-{}", Uuid::new_v4().simple());
        let listener = UnixListener::bind_addr(&address(&name).unwrap()).unwrap();
        let current = unsafe { libc::geteuid() };
        let mut client = connect_peer(&name, current).await.unwrap();
        let mut server = accept_gateway(&listener, current).await.unwrap().unwrap();
        let message = Message {
            id: Uuid::new_v4(),
            text: "private input stays within the authenticated pipe".into(),
        };
        write_frame(&mut client, &message).await.unwrap();
        assert_eq!(
            read_frame::<Message>(&mut server).await.unwrap(),
            Some(message)
        );
        client.shutdown().await.unwrap();
        assert_eq!(read_frame::<Message>(&mut server).await.unwrap(), None);
        assert!(connect_peer(&name, current.wrapping_add(1)).await.is_err());
        let wrong = UnixStream::connect_addr(&address(&name).unwrap())
            .await
            .unwrap();
        assert_eq!(wrong.peer_cred().unwrap().uid(), current);
        assert!(
            accept_gateway(&listener, current.wrapping_add(1))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn refuses_oversized_partial_and_malformed_frames() {
        let name = format!("voyage-gateway-test-{}", Uuid::new_v4().simple());
        let listener = UnixListener::bind_addr(&address(&name).unwrap()).unwrap();
        let mut client = UnixStream::connect_addr(&address(&name).unwrap())
            .await
            .unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        client
            .write_all(&((MAX_VESSEL_BODY as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        assert!(read_frame::<Message>(&mut server).await.is_err());
        let mut second = UnixStream::connect_addr(&address(&name).unwrap())
            .await
            .unwrap();
        let (mut receiver, _) = listener.accept().await.unwrap();
        second.write_all(&[0, 0]).await.unwrap();
        second.shutdown().await.unwrap();
        assert!(read_frame::<Message>(&mut receiver).await.is_err());
        let mut third = UnixStream::connect_addr(&address(&name).unwrap())
            .await
            .unwrap();
        let (mut receiver, _) = listener.accept().await.unwrap();
        third.write_all(&3u32.to_be_bytes()).await.unwrap();
        third.write_all(b"bad").await.unwrap();
        assert!(read_frame::<Message>(&mut receiver).await.is_err());
        assert!(address("../gateway").is_err());
        assert!(address("GATEWAY").is_err());
        assert!(address("").is_err());
    }
}
