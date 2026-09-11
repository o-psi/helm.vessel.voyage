//! Bounded Vessel requests. An I/O error never causes a command retry.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use voyage_protocol::vessel::{
    VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VesselEventSubscription,
    VoyageCommand, VoyageReply, VoyageRequest,
};

#[derive(Clone, Debug)]
pub struct Client {
    pub directory: PathBuf,
    pub access_file: Option<PathBuf>,
    id: super::connections::ConnectionId,
    generation: u64,
    managed: Option<super::connections::Connection>,
    socket: std::sync::Arc<super::duplex::Slot>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refusal(pub String);

impl Client {
    pub fn local(directory: PathBuf) -> Self {
        let route = super::connections::LegacyRoute {
            directory: directory.clone(),
            access_file: None,
        };
        Self {
            directory,
            access_file: None,
            id: route.id(),
            socket: super::duplex::slot(route.id(), 0),
            generation: 0,
            managed: None,
        }
    }
    pub fn access(path: PathBuf) -> Self {
        let route = super::connections::LegacyRoute {
            directory: PathBuf::new(),
            access_file: Some(path.clone()),
        };
        Self {
            directory: route.directory.clone(),
            access_file: Some(path),
            id: route.id(),
            socket: super::duplex::slot(route.id(), 0),
            generation: 0,
            managed: None,
        }
    }
    /// Preserve the exact legacy directory slot for explicitly configured clients.
    pub fn access_with_directory(path: PathBuf, directory: PathBuf) -> Self {
        let route = super::connections::LegacyRoute {
            directory: directory.clone(),
            access_file: Some(path.clone()),
        };
        Self {
            directory,
            access_file: Some(path),
            id: route.id(),
            socket: super::duplex::slot(route.id(), 0),
            generation: 0,
            managed: None,
        }
    }
    pub(super) fn from_connection(
        connection: super::connections::Connection,
        path: PathBuf,
    ) -> Self {
        Self {
            directory: PathBuf::new(),
            access_file: Some(path),
            id: connection.id,
            socket: super::duplex::slot(connection.id, 0),
            generation: 0,
            managed: Some(connection),
        }
    }
    pub fn id(&self) -> super::connections::ConnectionId {
        self.id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn with_generation(mut self, generation: u64) -> Self {
        if self.generation != generation {
            self.socket = super::duplex::slot(self.id, generation);
        }
        self.generation = generation;
        self
    }
    pub fn managed(&self) -> Option<&super::connections::Connection> {
        self.managed.as_ref()
    }
    pub fn legacy_route(&self) -> super::connections::LegacyRoute {
        self.managed
            .as_ref()
            .and_then(|c| c.legacy_route.clone())
            .unwrap_or_else(|| super::connections::LegacyRoute {
                directory: self.directory.clone(),
                access_file: self.access_file.clone(),
            })
    }
    fn pin(&self) -> Option<uuid::Uuid> {
        self.managed.as_ref().map(|c| c.vessel_id)
    }
    fn read_access(&self, path: &std::path::Path) -> Result<super::access::Credential> {
        let credential = super::access::credential(path)?;
        if let Some(connection) = &self.managed {
            super::connections::validate_credential(connection, &credential)?;
        }
        Ok(credential)
    }

    pub fn is_local(&self) -> bool {
        self.access_file.is_none()
    }
    pub fn label(&self) -> String {
        if let Some(c) = &self.managed {
            return if c.alias.is_empty() {
                c.endpoint.clone()
            } else {
                c.alias.clone()
            };
        }
        self.access_file
            .as_ref()
            .map(|p| {
                format!(
                    "grant:{}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
            })
            .unwrap_or_else(|| "local".into())
    }
    pub async fn request(&self, command: VesselCommand) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(15), self.exchange(command))
            .await
            .context("Vessel response deadline elapsed; command delivery may be unknown")?
    }

    pub async fn events(
        &self,
        subscriptions: Vec<VesselEventSubscription>,
    ) -> Result<futures_util::stream::BoxStream<'static, Result<VesselEvent>>> {
        let request = VesselEventRequest {
            protocol: VESSEL_API_VERSION,
            subscriptions,
        };
        self.socket.events(self, request).await
    }

    pub async fn supports_events(&self) -> bool {
        self.request(VesselCommand::Capabilities)
            .await
            .ok()
            .and_then(|value| value.get("features").and_then(Value::as_array).cloned())
            .is_some_and(|features| features.iter().any(|feature| feature == "duplex_socket"))
    }

    async fn exchange(&self, command: VesselCommand) -> Result<Value> {
        self.socket.exchange(self, command).await
    }

    pub fn connection_state(&self) -> tokio::sync::watch::Receiver<super::duplex::ConnectionState> {
        self.socket.state()
    }
    pub async fn reverse_requests(
        &self,
    ) -> Result<tokio::sync::mpsc::Receiver<super::duplex::IncomingReverseRequest>> {
        self.socket.reverse_requests()
    }
    /// Permanently retire this activation. Pending deliveries stay uncertain;
    /// explicit reconnection must use a fresh activation generation.
    pub fn disconnect(&self) {
        self.socket.disconnect();
    }

    pub(super) fn socket_request(
        &self,
    ) -> Result<(
        tokio_tungstenite::tungstenite::http::Request<()>,
        Option<uuid::Uuid>,
    )> {
        if let Some(path) = &self.access_file {
            let credential = self.read_access(path)?;
            let pin = self.pin().or(credential.vessel_id());
            let request = super::duplex::request(
                super::access::endpoint(
                    credential.endpoint(),
                    voyage_protocol::duplex::SOCKET_PATH,
                )?,
                credential.token(),
                Some(credential.grant_id()),
                pin,
            )?;
            Ok((request, pin))
        } else {
            let credential = super::local::credential(&self.directory)?;
            let request = super::duplex::request(
                super::local::endpoint(&credential, voyage_protocol::duplex::SOCKET_PATH)?,
                &credential.token,
                None,
                None,
            )?;
            Ok((request, None))
        }
    }

    pub async fn voyage(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: VoyageCommand,
    ) -> Result<Value> {
        Ok(self
            .voyage_observed(session_id, incarnation, command)
            .await?
            .0)
    }

    pub(super) async fn voyage_observed(
        &self,
        session_id: uuid::Uuid,
        incarnation: uuid::Uuid,
        command: VoyageCommand,
    ) -> Result<(Value, uuid::Uuid)> {
        let exact_owner = command.requires_incarnation();
        let reply: VoyageReply = serde_json::from_value(
            self.request(VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation: exact_owner.then_some(incarnation),
                command,
            }))
            .await?,
        )?;
        ensure!(
            reply.session_id == session_id && (!exact_owner || reply.incarnation == incarnation),
            "Vessel response identity mismatch"
        );
        Ok((reply.result, reply.incarnation))
    }
}
