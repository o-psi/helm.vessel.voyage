use super::{registry, routing};
use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response, sse::Event, sse::KeepAlive, sse::Sse},
    routing::post,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore},
};
use uuid::Uuid;
use voyage_protocol::process::*;
use voyage_protocol::vessel::{
    MAX_VESSEL_BODY, VESSEL_API_VERSION, VoyageCommand, VoyageReply, VoyageRequest,
};

pub(super) struct Supervisor {
    pub(super) directory: PathBuf,
    pub(super) binary: PathBuf,
    pub(super) model_slots: Arc<Semaphore>,
    pub(super) devices: voyage_runtime::accounts::device::DeviceService,
    pub(super) enrollment_workers: Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>,
    pub(super) assignment_locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
    pub(super) lifecycle_locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
    pub(super) registrations: super::database::Registrations,
}

#[derive(Clone)]
struct HttpState {
    supervisor: Arc<Supervisor>,
    token_hash: [u8; 32],
    capacity: Arc<Semaphore>,
    event_capacity: Arc<Semaphore>,
    socket_capacity: Arc<Semaphore>,
}

pub async fn serve(directory: PathBuf, binary: PathBuf) -> Result<()> {
    registry::private_directory(&directory)?;
    let _lock = registry::lock(&directory)?;
    super::identity::public(&directory)?;
    let sessions = directory.join("sessions");
    registry::private_directory(&sessions)?;
    let registrations = super::database::initialize(&directory).await?;
    for registration in registrations.values() {
        registry::publish(
            &registry::directory(&directory, registration.session_id),
            registration,
        )?;
    }
    let supervisor = Arc::new(Supervisor {
        directory: directory.clone(),
        binary,
        model_slots: Arc::new(Semaphore::new(4)),
        devices: super::accounts::device_service(directory.clone())?,
        enrollment_workers: Mutex::new(HashMap::new()),
        registrations: super::database::Registrations::new(directory.clone()),
        assignment_locks: Mutex::new(HashMap::new()),
        lifecycle_locks: Mutex::new(HashMap::new()),
    });
    supervisor.resume_enrollments().await?;
    let notification_delivery = supervisor.start_notification_delivery();
    let catalogue_refresh = supervisor.start_catalogue_refresh();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    registry::save_local_access(
        &directory,
        &LocalAccessCredential {
            endpoint: format!("http://{address}"),
            token: token.clone(),
        },
    )?;
    let state = HttpState {
        supervisor,
        token_hash: Sha256::digest(token.as_bytes()).into(),
        capacity: Arc::new(Semaphore::new(64)),
        event_capacity: Arc::new(Semaphore::new(16)),
        socket_capacity: Arc::new(Semaphore::new(64)),
    };
    let app = Router::new()
        .route(
            voyage_protocol::duplex::SOCKET_PATH,
            axum::routing::get(local_socket),
        )
        .route(voyage_protocol::vessel::COMMAND_PATH, post(local_command))
        .route(voyage_protocol::vessel::EVENTS_PATH, post(local_events))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_VESSEL_BODY))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_boundary,
        ))
        .with_state(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    notification_delivery.abort();
    catalogue_refresh.abort();
    let _ = notification_delivery.await;
    // Runtime processes own their lifetimes; service shutdown only detaches routing.
    let access = directory.join("process-http.json");
    if registry::load_local_access(&directory).is_ok_and(|current| current.token == token) {
        std::fs::remove_file(access)?;
        std::fs::File::open(&directory)?.sync_all()?;
    }
    Ok(())
}

/// Local account authority is authenticated by local_boundary before upgrade.
struct LocalSocketBackend {
    supervisor: Arc<Supervisor>,
    token_hash: [u8; 32],
    vessel_id: Uuid,
}
impl crate::duplex::Backend for LocalSocketBackend {
    fn command(&self, request: VesselRequest) -> crate::duplex::BackendFuture<VesselResponse> {
        let supervisor = self.supervisor.clone();
        Box::pin(async move { super::api::response(supervisor.handle(request.command).await) })
    }
    fn authorize(&self, _session: Option<Uuid>) -> crate::duplex::BackendFuture<bool> {
        let directory = self.supervisor.directory.clone();
        let expected = self.token_hash;
        let vessel_id = self.vessel_id;
        Box::pin(async move {
            if !super::identity::public(&directory)
                .is_ok_and(|identity| identity.vessel_id == vessel_id)
            {
                return false;
            }
            registry::load_local_access(&directory).is_ok_and(|access| {
                let actual: [u8; 32] = Sha256::digest(access.token.as_bytes()).into();
                bool::from(actual.ct_eq(&expected))
            })
        })
    }
}
async fn local_socket(
    State(state): State<HttpState>,
    headers: axum::http::HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if !crate::duplex::supports(&headers) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(permit) = state.socket_capacity.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(identity) = super::identity::public(&state.supervisor.directory) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let backend = Arc::new(LocalSocketBackend {
        supervisor: state.supervisor,
        token_hash: state.token_hash,
        vessel_id: identity.vessel_id,
    });
    upgrade
        .protocols([voyage_protocol::duplex::SUBPROTOCOL])
        .max_message_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .max_frame_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| crate::duplex::serve(socket, backend, identity.vessel_id, permit))
}

async fn local_events(
    State(state): State<HttpState>,
    Json(request): Json<VesselEventRequest>,
) -> Response {
    if request.protocol != VESSEL_API_VERSION
        || request.subscriptions.is_empty()
        || request.subscriptions.len() > 256
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut identities = std::collections::HashSet::new();
    if request
        .subscriptions
        .iter()
        .any(|subscription| !identities.insert(subscription.session_id))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(permit) = state.event_capacity.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let supervisor = state.supervisor;
    let stream = async_stream::stream! {
        let _permit = permit;
        let mut pending = futures_util::stream::FuturesUnordered::new();
        for subscription in request.subscriptions {
            pending.push(observe_local(supervisor.clone(), subscription));
        }
        while let Some((mut subscription, event, keep)) = futures_util::StreamExt::next(&mut pending).await {
            if let Some(cursor) = event.result.get("cursor").and_then(Value::as_u64) {
                subscription.after = cursor;
            }
            let changed = event.error.is_some()
                || event.result.get("replay_gap") == Some(&Value::Bool(true))
                || event.result.get("events").and_then(Value::as_array).is_some_and(|events| !events.is_empty());
            if changed {
                let data = match serde_json::to_string(&event) {
                    Ok(data) => data,
                    Err(_) => break,
                };
                yield Ok::<Event, std::convert::Infallible>(Event::default().event("update").data(data));
            }
            if keep {
                pending.push(observe_local(supervisor.clone(), subscription));
            }
        }
    };
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(5))
                .text("keep-alive"),
        )
        .into_response()
}

async fn observe_local(
    supervisor: Arc<Supervisor>,
    subscription: VesselEventSubscription,
) -> (VesselEventSubscription, VesselEvent, bool) {
    let session_id = subscription.session_id;
    let response = supervisor
        .voyage(
            VoyageRequest {
                session_id,
                incarnation: None,
                command: VoyageCommand::Events {
                    after: subscription.after,
                    limit: 128,
                    wait_ms: 10_000,
                },
            },
            None,
        )
        .await;
    let response = super::api::response(response);
    let mut subscription = subscription;
    let old_incarnation = subscription.incarnation;
    let (result, error, outcome_unknown, keep) = if let Some(error) = response.error {
        (
            response.result,
            Some(error),
            response.outcome_unknown,
            false,
        )
    } else {
        match serde_json::from_value::<VoyageReply>(response.result) {
            Ok(reply) => {
                subscription.incarnation = reply.incarnation;
                let mut result = reply.result;
                if reply.incarnation != old_incarnation {
                    result["owner_changed"] = Value::Bool(true);
                    result["replay_gap"] = Value::Bool(true);
                    result["recovery"] = Value::String("snapshot".into());
                }
                (result, None, false, true)
            }
            Err(_) => (
                Value::Null,
                Some("invalid Vessel observation".into()),
                false,
                false,
            ),
        }
    };
    let incarnation = subscription.incarnation;
    (
        subscription,
        VesselEvent {
            protocol: VESSEL_API_VERSION,
            session_id,
            incarnation,
            result,
            error,
            outcome_unknown,
        },
        keep,
    )
}

async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install termination handler");
    tokio::select! {
        _ = terminate.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

async fn local_boundary(
    State(state): State<HttpState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.headers().contains_key("origin")
        || request.headers().get_all("authorization").iter().count() != 1
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let candidate: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    if !bool::from(candidate.ct_eq(&state.token_hash)) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut response = next.run(request).await;
    drop(permit);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

async fn local_command(
    State(state): State<HttpState>,
    Json(request): Json<VesselRequest>,
) -> Response {
    if request.protocol != VESSEL_API_VERSION {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let result = tokio::time::timeout(
        Duration::from_secs(25),
        state.supervisor.handle(request.command),
    )
    .await
    .unwrap_or_else(|_| {
        Err(anyhow::anyhow!("supervisor request deadline exceeded")
            .context(routing::OutcomeUnknown))
    });
    let response = super::api::response(result);
    Json(response).into_response()
}

impl Supervisor {
    pub(super) async fn handle(&self, command: VesselCommand) -> Result<Value> {
        match command {
            VesselCommand::Notifications { operation } => self.notifications(operation, None).await,
            VesselCommand::DiscoverModels {
                workspace,
                configuration,
            } => self.discover_models(workspace, configuration).await,
            command @ (VesselCommand::AcceptParticipant { .. }
            | VesselCommand::RemoveParticipant { .. }) => self.participant_admin(command).await,
            VesselCommand::Assign { .. }
            | VesselCommand::FenceAssignment { .. }
            | VesselCommand::ObserveAssignment { .. }
            | VesselCommand::CancelAssignment { .. } => {
                anyhow::bail!("participant operations require an explicit scoped execution grant")
            }
            VesselCommand::Identity => Ok(serde_json::to_value(super::identity::public(
                &self.directory,
            )?)?),
            VesselCommand::TrustVessel { identity } => {
                let _serial = self.registrations.lock().await?;
                super::identity::pin(&self.directory, &identity)?;
                Ok(serde_json::json!({"trusted":identity.vessel_id}))
            }
            command @ (VesselCommand::PrepareTransfer { .. }
            | VesselCommand::ExportTransfer { .. }
            | VesselCommand::AcceptTransfer { .. }
            | VesselCommand::TransferChunk { .. }
            | VesselCommand::UploadTransferChunk { .. }
            | VesselCommand::ActivateTransfer { .. }) => self.transfer(command).await,
            command @ VesselCommand::Import { .. } => self.initialize_import(command).await,
            command @ VesselCommand::Branch { .. } => self.branch(command).await,
            command @ VesselCommand::Grant { .. } => self.grant(command).await,
            command @ VesselCommand::RevokeGrant { .. } => self.revoke_grant(command).await,
            VesselCommand::Granted {
                expected_vessel_id,
                grant_id,
                token,
                command,
            } => {
                self.granted(grant_id, token, expected_vessel_id, *command)
                    .await
            }
            command @ VesselCommand::ManagedImport { .. } => self.initialize_managed(command).await,
            VesselCommand::Capabilities => Ok(
                json!({"protocol":VESSEL_API_VERSION,"version":env!("CARGO_PKG_VERSION"),"vessel_id":super::identity::public(&self.directory)?.vessel_id,"platform":std::env::consts::OS,"features":["sqlite_catalogue","notifications","sessionless_models","provider_accounts","account_start","private_account_enrollment","catalogue","start","start_configured","start_resolution","inspect","voyage_operations","stop","restart","explicit_recovery","durable_receipts","history_paging","events","sse_events","duplex_socket","decisions","lifecycle","branch","ordinary_import","managed_import","scoped_grants","revocation","participant_bindings","participant_assignments","signed_owner_transfer"],"max_frame_bytes":MAX_VESSEL_BODY,"capacity":null,"max_connections":64}),
            ),
            command @ (VesselCommand::Accounts { .. }
            | VesselCommand::AccountDefaults { .. }
            | VesselCommand::AccountUsage { .. }
            | VesselCommand::AccountSetDefault { .. }
            | VesselCommand::AccountModels { .. }
            | VesselCommand::StartAccount { .. }
            | VesselCommand::ResolveStartAccount { .. }
            | VesselCommand::EnrollAccount { .. }
            | VesselCommand::CancelAccountEnrollment { .. }
            | VesselCommand::ResolveAccountEnrollment { .. }
            | VesselCommand::PrivateAccountEnrollment { .. }) => {
                self.host_accounts(command, super::accounts::Scope::Owner)
                    .await
            }
            VesselCommand::Catalogue => {
                let entries = self.catalogue().await?;
                Ok(serde_json::to_value(entries)?)
            }
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => self.start(command_id, session_id, workspace, None).await,
            VesselCommand::ResolveStart {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                self.resolve_start(command_id, session_id, workspace, config_path)
                    .await
            }
            VesselCommand::StartConfigured {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                self.start(command_id, session_id, workspace, Some(config_path))
                    .await
            }
            command @ VesselCommand::Recover { .. } => self.recover(command).await,
            VesselCommand::Restart {
                command_id,
                session_id,
                incarnation,
            } => self.restart(command_id, session_id, incarnation).await,
            VesselCommand::Inspect { session_id } => {
                let registration = self.registration(session_id).await?;
                Ok(serde_json::to_value(
                    routing::inspect(
                        &registry::directory(&self.directory, session_id),
                        &registration,
                    )
                    .await,
                )?)
            }
            VesselCommand::Voyage(request) => self.voyage(request, None).await,
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => super::api::reply(self.stop(session_id, incarnation, None).await?),
        }
    }

    pub(super) async fn registration(&self, session_id: Uuid) -> Result<ProcessRegistration> {
        super::database::registration(&self.directory, session_id).await
    }
}
