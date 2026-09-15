//! Offline fixtures shared by process regression tests. Never consult host accounts.
use super::{access::store, database, identity, registry};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::process::*;

pub(super) struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        // Leave room for sessions/<UUID>/runtime.sock in Unix sockaddr_un.
        let path = std::env::temp_dir().join(format!("vr-{}", Uuid::new_v4()));
        registry::private_directory(&path).unwrap();
        registry::private_directory(&path.join("sessions")).unwrap();
        for kind in ["grants", "connections"] {
            registry::private_directory(&path.join("access").join(kind)).unwrap();
        }
        Self(std::fs::canonicalize(path).unwrap())
    }
    pub fn registration(&self) -> ProcessRegistration {
        ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: None,
            token: "offline-fixture-not-a-credential".into(),
            workspace: self.0.clone(),
            state: ProcessState::Suspended,
            name: Some("Offline voyage".into()),
            executable: None,
        }
    }
    pub fn session(&self) -> ProcessGrant {
        ProcessGrant {
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            workspace: self.0.clone(),
            revision: 3,
            rights: vec![
                ProcessRight::AccountEnroll,
                ProcessRight::AccountUse,
                ProcessRight::Observe,
            ],
            accounts: vec![Uuid::new_v4()],
            enrollment_connections: vec![Uuid::new_v4()],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "unused".into(),
            parent_grant: None,
            connection_binding: None,
            participant_binding: None,
        }
    }
    pub fn connection(&self) -> ConnectionGrant {
        ConnectionGrant {
            schema_version: 1,
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            vessel_id: identity::public(&self.0).unwrap().vessel_id,
            revision: 7,
            rights: vec![
                ProcessRight::AccountEnroll,
                ProcessRight::AccountUse,
                ProcessRight::Observe,
            ],
            accounts: vec![Uuid::new_v4()],
            enrollment_connections: vec![Uuid::new_v4()],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "unused".into(),
            workspaces: vec![ApprovedWorkspace {
                id: Uuid::new_v4(),
                name: "fixture".into(),
                path: self.0.clone(),
                provider_ready: None,
            }],
        }
    }
    pub fn save_session(&self, grant: &ProcessGrant) {
        store::save(&store::grant_path(&self.0, grant.grant_id), grant).unwrap();
    }
    pub fn save_connection(&self, grant: &ConnectionGrant) {
        store::save(&store::connection_path(&self.0, grant.grant_id), grant).unwrap();
    }
    pub async fn supervisor(&self) -> super::service::Supervisor {
        use std::{collections::HashMap, sync::Arc};
        use tokio::sync::{Mutex, Semaphore};
        database::initialize(&self.0).await.unwrap();
        // Explicit temp registry: DeviceService construction does not start a worker.
        super::service::Supervisor {
            directory: self.0.clone(),
            binary: self.0.join("new-voyage"),
            model_slots: Arc::new(Semaphore::new(1)),
            devices: voyage_runtime::accounts::device::DeviceService::new(
                voyage_runtime::accounts::Registry::new(self.0.join("accounts")),
                Arc::new(|_, _| false),
            ),
            enrollment_workers: Mutex::new(HashMap::new()),
            assignment_locks: Mutex::new(HashMap::new()),
            lifecycle_locks: Mutex::new(HashMap::new()),
            registrations: database::Registrations::new(self.0.clone()),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
