//! Durable bounded device enrollment. No detached tasks, automatic effect replay or secret receipts.
//! The supervisor owns calling `drive`; disconnect need not cancel that worker.
use super::*;
use crate::provider::{ChatGptOauthProvider, DeviceAuthorization, OAuthEndpoints, ProviderError};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Must consult CURRENT authority, not merely a cached admission decision.
/// Called under the publication transaction: must be fast, synchronous, and not reenter Registry.
pub type Authorize = Arc<dyn Fn(&EnrollmentActor, Uuid) -> bool + Send + Sync>;
#[derive(Clone)]
pub struct DeviceService {
    registry: Registry,
    authorize: Authorize,
    #[cfg(test)]
    test_endpoints: Option<OAuthEndpoints>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Record {
    request: EnrollmentRequest,
    status: EnrollmentStatus,
    device: Option<DeviceAuthorization>,
    next_poll: u64,
    cancellations: Vec<Uuid>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl DeviceService {
    pub fn new(registry: Registry, authorize: Authorize) -> Self {
        Self {
            registry,
            authorize,
            #[cfg(test)]
            test_endpoints: None,
        }
    }
    fn authorized(&self, actor: &EnrollmentActor, connection: Uuid) -> Result<()> {
        ensure!(
            (self.authorize)(actor, connection),
            "enrollment authority unavailable or revoked"
        );
        Ok(())
    }
    fn provider(&self) -> ChatGptOauthProvider {
        // Store is never read or written: only unpublished device methods are used.
        #[cfg(test)]
        if let Some(endpoints) = &self.test_endpoints {
            return ChatGptOauthProvider::from_store(
                ChatGptTokenStore::new(PathBuf::new()),
                endpoints.clone(),
            );
        }
        ChatGptOauthProvider::from_store(
            ChatGptTokenStore::new(PathBuf::new()),
            OAuthEndpoints::default(),
        )
    }
    /// Exact observation/negative-admission fence. A late start under this
    /// envelope sees the retained terminal record and cannot contact the provider.
    pub fn resolve(&self, request: EnrollmentRequest) -> Result<EnrollmentStatus> {
        ensure!(
            !request.command_id.is_nil()
                && !request.enrollment_id.is_nil()
                && request.command_id != request.enrollment_id,
            "invalid enrollment identity"
        );
        text(&request.alias)?;
        text(&request.label)?;
        self.authorized(&request.actor, request.connection_id)?;
        self.registry.transaction(|db| {
            if let Some(r) = db.enrollments.iter_mut().find(|r| {
                [request.command_id, request.enrollment_id]
                    .iter()
                    .any(|id| {
                        *id == r.request.command_id
                            || *id == r.request.enrollment_id
                            || r.cancellations.contains(id)
                    })
            }) {
                ensure!(r.request == request, "enrollment command identity conflict");
                expire(r);
                return Ok(r.status.clone());
            }
            ensure!(
                db.enrollments.len() < 64,
                "retained enrollment capacity reached; owner recovery required"
            );
            ensure!(
                db.connections.iter().any(|c| c.id == request.connection_id
                    && c.transports == [Transport::ChatgptOauth]
                    && c.endpoint == "https://chatgpt.com/backend-api/codex"),
                "unsupported enrollment connection"
            );
            let status = EnrollmentStatus {
                enrollment_id: request.enrollment_id,
                state: EnrollmentState::Cancelled,
                account_id: None,
                expires_at: now(),
                effects_may_have_occurred: false,
            };
            db.enrollments.push(Record {
                request,
                status: status.clone(),
                device: None,
                next_poll: 0,
                cancellations: Vec::new(),
            });
            Ok(status)
        })
    }

    pub async fn start(&self, request: EnrollmentRequest) -> Result<EnrollmentStatus> {
        ensure!(
            !request.command_id.is_nil()
                && !request.enrollment_id.is_nil()
                && request.command_id != request.enrollment_id,
            "enrollment requires distinct non-nil command and enrollment UUIDs"
        );
        text(&request.alias)?;
        text(&request.label)?;
        ensure!(
            !request.actor.principal.is_empty()
                && request.actor.principal.len() <= 256
                && !request.actor.workspace.is_empty()
                && request.actor.workspace.len() <= 4096,
            "invalid enrollment actor"
        );
        self.authorized(&request.actor, request.connection_id)?;
        let (status, fresh) = self.registry.transaction(|db| {
            for r in &mut db.enrollments {
                expire(r);
            }
            if let Some(r) = db.enrollments.iter().find(|r| {
                [request.command_id, request.enrollment_id]
                    .iter()
                    .any(|id| {
                        *id == r.request.command_id
                            || *id == r.request.enrollment_id
                            || r.cancellations.contains(id)
                    })
            }) {
                ensure!(r.request == request, "enrollment command identity conflict");
                return Ok((r.status.clone(), false));
            }
            let c = db
                .connections
                .iter()
                .find(|c| c.id == request.connection_id)
                .ok_or_else(|| anyhow::anyhow!("unknown connection"))?;
            ensure!(
                c.transports == [Transport::ChatgptOauth]
                    && c.endpoint == "https://chatgpt.com/backend-api/codex",
                "device sign-in supports only the native ChatGPT connection"
            );
            ensure!(
                db.enrollments.len() < 64,
                "retained enrollment capacity reached; owner recovery required"
            );
            ensure!(
                db.enrollments
                    .iter()
                    .filter(|r| matches!(
                        r.status.state,
                        EnrollmentState::Starting
                            | EnrollmentState::Pending
                            | EnrollmentState::Exchanging
                    ))
                    .count()
                    < 8,
                "concurrent enrollment capacity reached"
            );
            ensure!(
                !db.accounts
                    .iter()
                    .any(|a| a.descriptor.alias == request.alias)
                    && !db
                        .enrollments
                        .iter()
                        .any(|r| r.request.alias == request.alias
                            && matches!(
                                r.status.state,
                                EnrollmentState::Starting
                                    | EnrollmentState::Pending
                                    | EnrollmentState::Exchanging
                                    | EnrollmentState::Uncertain
                            )),
                "alias already reserved"
            );
            let status = EnrollmentStatus {
                enrollment_id: request.enrollment_id,
                state: EnrollmentState::Starting,
                account_id: None,
                expires_at: now() + 600,
                effects_may_have_occurred: true,
            };
            db.enrollments.push(Record {
                request: request.clone(),
                status: status.clone(),
                device: None,
                next_poll: 0,
                cancellations: Vec::new(),
            });
            Ok((status, true))
        })?;
        if !fresh {
            return Ok(status);
        }
        if !(self.authorize)(&request.actor, request.connection_id) {
            return self.registry.transaction(|db| {
                let r = record(db, request.enrollment_id, &request.actor)?;
                r.status.state = EnrollmentState::Cancelled;
                r.device = None;
                Ok(r.status.clone())
            });
        }
        // Starting is durable before the first network effect. Lost responses are never replayed.
        let result =
            tokio::time::timeout(Duration::from_secs(30), self.provider().begin_device()).await;
        self.registry.transaction(|db| {
            let r = record(db, request.enrollment_id, &request.actor)?;
            if r.status.state != EnrollmentState::Starting {
                return Ok(r.status.clone());
            }
            if !(self.authorize)(&request.actor, request.connection_id) {
                r.status.state = EnrollmentState::Cancelled;
            } else if let Ok(Ok(device)) = result {
                // The provider website, not arbitrary remote content, is the only allowed URL.
                if device.verification_uri != "https://auth.openai.com/codex/device"
                    || device.user_code.len() > 64
                    || device.user_code.is_empty()
                    || !device
                        .user_code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || device.device_auth_id.is_empty()
                    || device.device_auth_id.len() > 4096
                    || device.interval == 0
                    || device.interval > 600
                    || device.expires_in == Some(0)
                {
                    r.status.state = EnrollmentState::Uncertain;
                } else {
                    if let Some(expiry) = device.expires_in {
                        r.status.expires_at = r.status.expires_at.min(now().saturating_add(expiry));
                    }
                    r.next_poll = now() + device.interval;
                    r.device = Some(device);
                    r.status.state = EnrollmentState::Pending;
                }
            } else {
                r.status.state = EnrollmentState::Uncertain;
            }
            Ok(r.status.clone())
        })
    }
    pub fn status(&self, id: Uuid, actor: &EnrollmentActor) -> Result<PrivateEnrollmentStatus> {
        self.registry.transaction(|db| {
            let r = record(db, id, actor)?;
            self.authorized(actor, r.request.connection_id)?;
            expire(r);
            let device = if r.status.state == EnrollmentState::Pending {
                r.device.as_ref()
            } else {
                None
            };
            Ok(PrivateEnrollmentStatus {
                status: r.status.clone(),
                user_code: device.map(|d| d.user_code.clone()),
                verification_uri: device.map(|d| d.verification_uri.clone()),
            })
        })
    }
    pub fn cancel(
        &self,
        command_id: Uuid,
        id: Uuid,
        actor: &EnrollmentActor,
    ) -> Result<EnrollmentStatus> {
        self.registry.transaction(|db| {
            ensure!(
                !command_id.is_nil()
                    && !db
                        .enrollments
                        .iter()
                        .any(|r| r.request.command_id == command_id
                            || r.request.enrollment_id == command_id
                            || (r.request.enrollment_id != id
                                && r.cancellations.contains(&command_id))),
                "cancel command identity conflict"
            );
            let r = record(db, id, actor)?;
            self.authorized(actor, r.request.connection_id)?;
            if !r.cancellations.contains(&command_id) {
                ensure!(
                    r.cancellations.len() < 16,
                    "cancel receipt capacity reached"
                );
                r.cancellations.push(command_id);
            }
            if r.status.state != EnrollmentState::Succeeded {
                r.status.state = EnrollmentState::Cancelled;
                r.device = None;
            }
            Ok(r.status.clone())
        })
    }
    /// Host-supervisor recovery input, NOT a remote enumeration endpoint. Bound by the
    /// registry's active-attempt limit. `run` rechecks each retained actor's current authority.
    /// Starting/exchanging attempts are observed until expiry, never replayed after a crash.
    pub fn resume_candidates(&self) -> Result<Vec<(Uuid, EnrollmentActor)>> {
        self.registry.transaction(|db| {
            for r in &mut db.enrollments {
                expire(r);
            }
            Ok(db
                .enrollments
                .iter()
                .filter(|r| {
                    matches!(
                        r.status.state,
                        EnrollmentState::Starting
                            | EnrollmentState::Pending
                            | EnrollmentState::Exchanging
                    )
                })
                .map(|r| (r.request.enrollment_id, r.request.actor.clone()))
                .collect())
        })
    }

    /// Host-owned bounded worker, suitable for a supervised task retained across Helm disconnect.
    /// Dropping this future can leave a durable uncertain effect, never an automatic replay.
    pub async fn run(&self, id: Uuid, actor: &EnrollmentActor) -> Result<EnrollmentStatus> {
        loop {
            let status = self.drive(id, actor).await?;
            if !matches!(
                status.state,
                EnrollmentState::Starting | EnrollmentState::Pending | EnrollmentState::Exchanging
            ) {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    /// One bounded host tick. The caller schedules ticks (at most once a second),
    /// retaining the original ID. In-flight/crashed effects are not reissued.
    pub async fn drive(&self, id: Uuid, actor: &EnrollmentActor) -> Result<EnrollmentStatus> {
        let (status, device) = self.registry.transaction(|db| {
            let r = record(db, id, actor)?;
            if !(self.authorize)(actor, r.request.connection_id) {
                if r.status.state != EnrollmentState::Succeeded {
                    r.status.state = EnrollmentState::Cancelled;
                    r.device = None;
                }
                return Ok((r.status.clone(), None));
            }
            expire(r);
            if r.status.state != EnrollmentState::Pending || now() < r.next_poll {
                return Ok((r.status.clone(), None));
            }
            r.status.state = EnrollmentState::Exchanging;
            Ok((r.status.clone(), r.device.clone()))
        })?;
        let Some(device) = device else {
            return Ok(status);
        };
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            let provider = self.provider();
            let grant = provider.poll_device_grant(&device).await?;
            // Poll and exchange are separate external effects. A late poll result must
            // not start a token exchange after cancellation, expiry or revocation.
            // Keep Exchanging durable throughout: a crash never replays either effect.
            let proceed = self
                .registry
                .transaction(|db| {
                    let r = record(db, id, actor)?;
                    if r.status.state != EnrollmentState::Exchanging {
                        return Ok(false);
                    }
                    if !(self.authorize)(actor, r.request.connection_id) {
                        r.status.state = EnrollmentState::Cancelled;
                    } else if now() >= r.status.expires_at {
                        r.status.state = EnrollmentState::Expired;
                    } else {
                        return Ok(true);
                    }
                    r.device = None;
                    Ok(false)
                })
                .map_err(|_| {
                    ProviderError::Authentication("device publication state unavailable".into())
                })?;
            if !proceed {
                return Err(ProviderError::Authentication(
                    "device authorization no longer active".into(),
                ));
            }
            provider.exchange_device_grant(grant).await
        })
        .await;
        self.registry.transaction(|db| {
            let r = record(db, id, actor)?;
            if r.status.state != EnrollmentState::Exchanging {
                return Ok(r.status.clone());
            }
            if !(self.authorize)(actor, r.request.connection_id) {
                r.status.state = EnrollmentState::Cancelled;
                r.device = None;
                return Ok(r.status.clone());
            }
            match result.map(|value| value.map_err(ProviderError::into_semantic)) {
                Ok(Ok(tokens)) => {
                    // Cancellation and publication use the SAME lock and atomic checkpoint.
                    let request = r.request.clone();
                    if now() >= r.status.expires_at {
                        r.status.state = EnrollmentState::Expired;
                        r.device = None;
                        return Ok(r.status.clone());
                    }
                    let a = insert(
                        db,
                        request.connection_id,
                        request.alias,
                        request.label,
                        Credential::OAuth(tokens),
                        Some(id),
                    )?;
                    let r = record(db, id, actor)?;
                    r.status.account_id = Some(a.id);
                    r.status.state = EnrollmentState::Succeeded;
                    r.device = None;
                }
                Ok(Err(ProviderError::Unavailable(message))) => {
                    if message == "device authorization slow_down"
                        && let Some(d) = &mut r.device
                    {
                        d.interval = d.interval.saturating_add(5).min(600);
                    }
                    r.next_poll = now() + r.device.as_ref().map(|d| d.interval).unwrap_or(5);
                    r.status.state = EnrollmentState::Pending;
                    expire(r);
                }
                Ok(Err(ProviderError::Authentication(message)))
                    if message == "device authorization denied"
                        || message == "device authorization expired" =>
                {
                    r.status.state = if message.ends_with("expired") {
                        EnrollmentState::Expired
                    } else {
                        EnrollmentState::Denied
                    };
                    r.device = None;
                }
                _ => {
                    r.status.state = EnrollmentState::Uncertain;
                    r.device = None;
                }
            }
            Ok(record(db, id, actor)?.status.clone())
        })
    }
}
fn record<'a>(db: &'a mut Database, id: Uuid, actor: &EnrollmentActor) -> Result<&'a mut Record> {
    db.enrollments
        .iter_mut()
        .find(|r| r.request.enrollment_id == id && &r.request.actor == actor)
        .ok_or_else(|| anyhow::anyhow!("enrollment unavailable"))
}
fn expire(r: &mut Record) {
    if now() >= r.status.expires_at {
        match r.status.state {
            EnrollmentState::Starting | EnrollmentState::Exchanging => {
                r.status.state = EnrollmentState::Uncertain;
                r.device = None;
            }
            EnrollmentState::Pending => {
                r.status.state = EnrollmentState::Expired;
                r.device = None;
            }
            _ => {}
        }
    }
}

pub(super) fn alias_reserved(db: &Database, alias: &str, own_enrollment: Option<Uuid>) -> bool {
    db.enrollments.iter().any(|r| {
        r.request.alias == alias
            && Some(r.request.enrollment_id) != own_enrollment
            && matches!(
                r.status.state,
                EnrollmentState::Starting
                    | EnrollmentState::Pending
                    | EnrollmentState::Exchanging
                    | EnrollmentState::Uncertain
            )
    })
}

pub(super) fn enrollment_actor(db: &Database, account: Uuid) -> Option<EnrollmentActor> {
    db.enrollments
        .iter()
        .find(|r| {
            r.status.state == EnrollmentState::Succeeded && r.status.account_id == Some(account)
        })
        .map(|r| r.request.actor.clone())
}

// Fixtures cannot select a remote host, even in a test binary. No runtime configuration seam.
#[cfg(test)]
impl DeviceService {
    pub(super) fn loopback_fixture(mut self, address: std::net::SocketAddr) -> Self {
        assert!(address.ip().is_loopback());
        let base = format!("http://{address}");
        self.test_endpoints = Some(OAuthEndpoints {
            authorize: format!("{base}/unused-authorize"),
            token: format!("{base}/token"),
            device_user_code: format!("{base}/device"),
            device_token: format!("{base}/poll"),
            responses: format!("{base}/unused-responses"),
            models: format!("{base}/unused-models"),
        });
        self
    }

    // Advance only retained scheduling metadata: actual HTTP parsing/exchange/publication runs.
    pub(super) fn fixture_due(&self, id: Uuid, expire_now: bool) -> Result<u64> {
        self.registry.transaction(|db| {
            let r = db
                .enrollments
                .iter_mut()
                .find(|r| r.request.enrollment_id == id)
                .unwrap();
            r.next_poll = 0;
            if expire_now {
                r.status.expires_at = 0;
            }
            Ok(r.device.as_ref().map(|d| d.interval).unwrap_or(0))
        })
    }
}
