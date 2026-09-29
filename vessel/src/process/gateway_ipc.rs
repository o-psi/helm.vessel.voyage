//! Bounded, peer-authenticated local transport for a future system gateway.
//! This module does not enable a system installation or expose supervisor state.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{os::linux::net::SocketAddrExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};
use voyage_protocol::vessel::MAX_VESSEL_BODY;

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
pub(super) fn bind_root(name: &str, gateway_uid: u32) -> Result<UnixListener> {
    ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
    ensure!(
        gateway_uid != 0,
        "gateway must have a separate unprivileged UID"
    );
    Ok(UnixListener::bind_addr(&address(name)?)?)
}

/// The kernel credential is checked before a byte of request data is read.
pub(super) async fn accept_gateway(
    listener: &UnixListener,
    gateway_uid: u32,
) -> Result<UnixStream> {
    let (stream, _) = listener.accept().await?;
    ensure!(
        stream.peer_cred()?.uid() == gateway_uid,
        "system gateway peer identity refused"
    );
    Ok(stream)
}

/// The gateway verifies the root peer before sending grants or invitation codes.
pub(super) async fn connect_root(name: &str) -> Result<UnixStream> {
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
pub(super) async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
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

pub(super) async fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<Option<T>> {
    let mut length = [0u8; 4];
    let first = timeout(DEADLINE, stream.read(&mut length[..1])).await??;
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
        let mut server = accept_gateway(&listener, current).await.unwrap();
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
                .is_err()
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
