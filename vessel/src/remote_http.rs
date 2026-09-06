//! Opt-in authenticated operator relay. No provider credentials, runtime grants, or transcripts.
use super::*;
use axum::extract::{Path, Query};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::sync::oneshot;
use vessel::attachment_transport::{AttachmentApi, AuthenticatedFrame, ConnectionPresence};
use voyage_protocol::{
    attachment::{Command, Operation, VERSION},
    events::{EventCursor, Feature, Features},
    stream::{Frame, Reply},
};

type PendingKey = (Uuid, Uuid, Uuid);
type PendingReplies = Arc<Mutex<HashMap<PendingKey, Option<oneshot::Sender<Frame>>>>>;
fn deliver(pending: &PendingReplies, key: PendingKey, frame: Frame) {
    let sender = pending
        .lock()
        .ok()
        .and_then(|mut p| p.get_mut(&key).and_then(Option::take));
    if let Some(sender) = sender {
        let _ = sender.send(frame);
    }
}
const MUTATIONS_PER_CONNECTION: usize = 4096;
const RETAINED_GENERATIONS: usize = 32;
type Generation = (Uuid, Uuid);
#[derive(Default)]
struct MutationFingerprints(HashMap<Generation, HashMap<Uuid, [u8; 32]>>);
impl MutationFingerprints {
    fn admit(&mut self, key: PendingKey, digest: [u8; 32]) -> UiResult<()> {
        let generation = (key.0, key.1);
        if let Some(previous) = self.0.get(&generation).and_then(|ids| ids.get(&key.2)) {
            return if *previous == digest {
                Ok(())
            } else {
                Err(ui_error((
                    StatusCode::CONFLICT,
                    "Command ID already names a different request",
                )))
            };
        }
        if !self.0.contains_key(&generation) && self.0.len() >= RETAINED_GENERATIONS {
            return Err(ui_error((
                StatusCode::SERVICE_UNAVAILABLE,
                "Remote connection bookkeeping full; retry after retired connections close",
            )));
        }
        let ids = self.0.entry(generation).or_default();
        if ids.len() >= MUTATIONS_PER_CONNECTION {
            return Err(ui_error((
                StatusCode::CONFLICT,
                "Remote command capacity reached; reconnect the foreground worker before new commands; exact retries remain available",
            )));
        }
        ids.insert(key.2, digest);
        Ok(())
    }
    fn prune(&mut self, candidates: &[Generation], registered: &[Generation]) {
        // Only consider keys captured before the registry snapshot: an insertion
        // racing that snapshot must not lose its immutable digest.
        for candidate in candidates {
            if !registered.contains(candidate) {
                self.0.remove(candidate);
            }
        }
    }
}
fn mutation_digest(frame: &Frame) -> UiResult<Option<[u8; 32]>> {
    let Frame::Command { command } = frame else {
        return Ok(None);
    };
    if !matches!(
        command.operation,
        Operation::Submit { .. } | Operation::Cancel { .. }
    ) {
        return Ok(None);
    }
    let bytes = command
        .admission_bytes()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    Ok(Some(Sha256::digest(bytes).into()))
}
fn transport_id(operation: &Operation, requested: Uuid) -> Uuid {
    if matches!(
        operation,
        Operation::List { .. } | Operation::Inspect { .. }
    ) {
        Uuid::new_v4()
    } else {
        requested
    }
}
fn restore_command_id(frame: Frame, requested: Uuid) -> UiResult<Frame> {
    match frame {
        Frame::Result {
            connection_id,
            reply,
            ..
        } => Ok(Frame::Result {
            connection_id,
            command_id: requested,
            reply,
        }),
        _ => Err(ui_error(StatusCode::BAD_GATEWAY)),
    }
}
#[derive(Clone)]
pub(super) struct RemoteApi {
    attachment: AttachmentApi,
    origin: String,
    pending: PendingReplies,
    mutations: Arc<Mutex<MutationFingerprints>>,
}
struct Pending {
    key: PendingKey,
    pending: PendingReplies,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.key);
        }
    }
}
impl RemoteApi {
    pub fn new(enrollment: vessel::enrollment_http::EnrollmentApi) -> Result<Self> {
        let origin = enrollment.origin().to_owned();
        let features = Features::new(vec![
            Feature::SequencedEvents,
            Feature::Replay,
            Feature::ToolActivity,
            Feature::Usage,
            Feature::ManagedExecution,
        ])
        .map_err(anyhow::Error::msg)?;
        let (attachment, mut incoming) = AttachmentApi::new(enrollment, features)?;
        let this = Self {
            attachment,
            origin,
            pending: Arc::new(Mutex::new(HashMap::new())),
            mutations: Arc::default(),
        };
        let monitor = this.clone();
        tokio::spawn(async move {
            let mut maintenance = tokio::time::interval(Duration::from_secs(1));
            loop {
                let received = tokio::select! {
                    value = incoming.recv() => value,
                    _ = maintenance.tick() => {
                        if monitor.attachment.is_shutdown() { break; }
                        let candidates = monitor.mutations.lock().map(|m| m.0.keys().copied().collect::<Vec<_>>()).unwrap_or_default();
                        let registered = monitor.attachment.registered_generations().await;
                        if let Ok(mut mutations) = monitor.mutations.lock() {
                            mutations.prune(&candidates, &registered);
                        }
                        continue;
                    }
                };
                let Some(AuthenticatedFrame {
                    machine_id,
                    connection_id,
                    frame,
                    ..
                }) = received
                else {
                    break;
                };
                if !monitor
                    .attachment
                    .is_current(machine_id, connection_id)
                    .await
                {
                    continue;
                }
                let request = match &frame {
                    Frame::Result { command_id, .. } => *command_id,
                    Frame::Replay { request_id, .. }
                    | Frame::SnapshotRequired { request_id, .. } => *request_id,
                    // Live text is never cached by Vessel. Clients explicitly request bounded
                    // replay pages from Helm's transactional public cursor.
                    _ => continue,
                };
                deliver(
                    &monitor.pending,
                    (machine_id, connection_id, request),
                    frame,
                );
            }
        });
        Ok(this)
    }
    pub fn attachment(&self) -> AttachmentApi {
        self.attachment.clone()
    }
    async fn connection(&self, machine: Uuid) -> UiResult<ConnectionPresence> {
        let current = self
            .attachment
            .connections()
            .await
            .into_iter()
            .find(|c| c.machine_id == machine)
            .ok_or_else(|| {
                ui_error((
                    StatusCode::NOT_FOUND,
                    "Current remote connection unavailable",
                ))
            })?;
        if !self
            .attachment
            .supports_feature(machine, current.connection_id, Feature::ManagedExecution)
            .await
        {
            return Err(ui_error((
                StatusCode::NOT_FOUND,
                "Current remote execution connection unavailable",
            )));
        }
        Ok(current)
    }
    async fn exchange(
        &self,
        current: &ConnectionPresence,
        id: Uuid,
        frame: Frame,
    ) -> UiResult<Frame> {
        let (sender, receiver) = oneshot::channel();
        let key = (current.machine_id, current.connection_id, id);
        if let Some(digest) = mutation_digest(&frame)? {
            self.mutations
                .lock()
                .map_err(|_| ui_error(StatusCode::SERVICE_UNAVAILABLE))?
                .admit(key, digest)?;
        }
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| ui_error(StatusCode::SERVICE_UNAVAILABLE))?;
            if pending.len() >= 64 || pending.contains_key(&key) {
                return Err(ui_error((StatusCode::CONFLICT, "Remote request busy")));
            }
            pending.insert(key, Some(sender));
        }
        let _pending = Pending {
            key,
            pending: self.pending.clone(),
        };
        self.attachment
            .send(current.machine_id, frame)
            .await
            .map_err(|_| ui_error((StatusCode::CONFLICT, "Remote connection unavailable")))?;
        let frame = tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| {
                ui_error((
                    StatusCode::GATEWAY_TIMEOUT,
                    "Remote outcome unconfirmed; retry the same command ID and deadline",
                ))
            })?
            .map_err(|_| ui_error(StatusCode::SERVICE_UNAVAILABLE))?;
        if !self
            .attachment
            .is_current(current.machine_id, current.connection_id)
            .await
        {
            return Err(ui_error((
                StatusCode::CONFLICT,
                "Remote connection changed; observe with a fresh connection",
            )));
        }
        Ok(frame)
    }
}
fn authorize<'a>(state: &'a AppState, headers: &HeaderMap) -> UiResult<&'a RemoteApi> {
    operator_auth(state, headers)?;
    let api = state
        .remote
        .as_ref()
        .ok_or_else(|| ui_error(StatusCode::NOT_FOUND))?;
    if headers.get_all("origin").iter().count() > 1
        || headers
            .get("origin")
            .is_some_and(|v| v.to_str().ok() != Some(api.origin.as_str()))
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    Ok(api)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    command_id: Uuid,
    expires_at_ms: i64,
    operation: Operation,
}
pub(super) async fn command(
    State(state): State<AppState>,
    Path(machine): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    let request: Request = serde_json::from_slice(&body)
        .map_err(|_| ui_error((StatusCode::BAD_REQUEST, "Invalid remote request")))?;
    if headers.get("x-voyage-request").is_none_or(|v| v != "2") {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    if !matches!(
        request.operation,
        Operation::List { .. }
            | Operation::Inspect { .. }
            | Operation::Submit { .. }
            | Operation::Cancel { .. }
    ) {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    let current = api.connection(machine).await?;
    let wire_id = transport_id(&request.operation, request.command_id);
    let command = Command {
        version: VERSION,
        connection_id: current.connection_id,
        machine_id: machine,
        principal_id: current.owner_id,
        command_id: wire_id,
        expires_at_ms: request.expires_at_ms,
        operation: request.operation,
    };
    command
        .validate_structure()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let frame = api
        .exchange(&current, wire_id, Frame::Command { command })
        .await?;
    Ok(private_response(restore_command_id(
        frame,
        request.command_id,
    )?))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Watch {
    session_id: Uuid,
    after: u64,
    #[serde(default = "default_limit")]
    limit: u16,
}
fn default_limit() -> u16 {
    32
}
pub(super) async fn watch(
    State(state): State<AppState>,
    Path(machine): Path<Uuid>,
    headers: HeaderMap,
    Query(request): Query<Watch>,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    let current = api.connection(machine).await?;
    let request_id = Uuid::new_v4();
    let after = EventCursor::new(request.after).map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let frame = Frame::ReplayRequest {
        connection_id: current.connection_id,
        request_id,
        session_id: request.session_id,
        after,
        limit: request.limit,
    };
    frame
        .encode()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let result = api.exchange(&current, request_id, frame).await?;
    validate_watch_reply(&result, request_id, request.session_id, after)?;
    Ok(private_response(result))
}

fn validate_watch_reply(
    result: &Frame,
    request_id: Uuid,
    session: Uuid,
    after: EventCursor,
) -> UiResult<()> {
    let valid = match result {
        Frame::Replay {
            request_id: id,
            session_id,
            after: received,
            ..
        }
        | Frame::SnapshotRequired {
            request_id: id,
            session_id,
            after: received,
            ..
        } => *id == request_id && *session_id == session && *received == after,
        // v2 already carries bounded denials. For a replay refusal the result ID
        // names its request, and no session metadata or cursor is disclosed.
        Frame::Result {
            command_id,
            reply: Reply::Denied { .. },
            ..
        } => *command_id == request_id,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ui_error(StatusCode::BAD_GATEWAY))
    }
}

fn private_response(frame: Frame) -> Response {
    let mut response = Json(frame).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remote_authorization_uses_validated_enrollment_origin() {
        const TOKEN: &str = "synthetic-origin-test-operator-token";
        for configured in [
            "https://vessel.example",
            "https://vessel.example/",
            "HTTPS://VESSEL.EXAMPLE:443/",
            "https://Vessel.Example:443",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let store = vessel::enrollment::EnrollmentStore::open(
                &directory.path().join("enrollment"),
                configured,
                false,
            )
            .unwrap();
            let enrollment = vessel::enrollment_http::EnrollmentApi::new(store, TOKEN).unwrap();
            let remote = RemoteApi::new(enrollment).unwrap();
            let state = AppState {
                database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
                operator_token_hash: Some(token_hash(TOKEN)),
                attachment: None,
                remote: Some(remote.clone()),
                control: None,
            };
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
            assert!(authorize(&state, &headers).is_ok());
            headers.insert("origin", HeaderValue::from_static("https://vessel.example"));
            headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
            assert!(authorize(&state, &headers).is_ok(), "{configured}");
            for denied in [
                "https://foreign.example",
                "null",
                "https://vessel.example/",
                "https://user:secret@vessel.example",
                "https://vessel.example?query",
                "https://vessel.example https://foreign.example",
            ] {
                headers.insert("origin", denied.parse().unwrap());
                assert_eq!(
                    authorize(&state, &headers)
                        .err()
                        .unwrap()
                        .into_response()
                        .status(),
                    StatusCode::FORBIDDEN
                );
            }
            headers.insert("origin", HeaderValue::from_static("https://vessel.example"));
            headers.append("origin", HeaderValue::from_static("https://vessel.example"));
            assert_eq!(
                authorize(&state, &headers)
                    .err()
                    .unwrap()
                    .into_response()
                    .status(),
                StatusCode::FORBIDDEN
            );
            headers.remove("origin");
            headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
            assert_eq!(
                authorize(&state, &headers)
                    .err()
                    .unwrap()
                    .into_response()
                    .status(),
                StatusCode::FORBIDDEN
            );
            headers.remove("sec-fetch-site");
            headers.remove("authorization");
            assert!(authorize(&state, &headers).is_err());
            remote.attachment.shutdown().await;
        }
    }

    fn reply(id: Uuid) -> Frame {
        Frame::SnapshotRequired {
            connection_id: Uuid::new_v4(),
            request_id: id,
            session_id: Uuid::new_v4(),
            after: EventCursor::new(0).unwrap(),
            latest: EventCursor::new(0).unwrap(),
        }
    }

    #[test]
    fn watch_accepts_only_matching_observation_or_bounded_denial() {
        use voyage_protocol::stream::DenialCode;
        let request = Uuid::new_v4();
        let session = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let after = EventCursor::new(0).unwrap();
        for code in [DenialCode::Unauthorized, DenialCode::InvalidRequest] {
            let denial = Frame::Result {
                connection_id: connection,
                command_id: request,
                reply: Reply::Denied { code },
            };
            assert!(validate_watch_reply(&denial, request, session, after).is_ok());
            assert!(validate_watch_reply(&denial, Uuid::new_v4(), session, after).is_err());
        }
        let accepted = Frame::Result {
            connection_id: connection,
            command_id: request,
            reply: Reply::Accepted {},
        };
        assert!(validate_watch_reply(&accepted, request, session, after).is_err());
        for observation in [
            Frame::Replay {
                connection_id: connection,
                request_id: request,
                session_id: session,
                after,
                latest: after,
                events: vec![],
            },
            Frame::SnapshotRequired {
                connection_id: connection,
                request_id: request,
                session_id: session,
                after,
                latest: EventCursor::new(1).unwrap(),
            },
        ] {
            assert!(validate_watch_reply(&observation, request, session, after).is_ok());
            assert!(validate_watch_reply(&observation, Uuid::new_v4(), session, after).is_err());
            assert!(validate_watch_reply(&observation, request, Uuid::new_v4(), after).is_err());
            assert!(
                validate_watch_reply(&observation, request, session, EventCursor::new(1).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn delivered_reply_retains_slot_until_its_waiter_guard_drops() {
        let key = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let pending = PendingReplies::default();
        let (sender, mut receiver) = oneshot::channel();
        pending.lock().unwrap().insert(key, Some(sender));
        let guard = Pending {
            key,
            pending: pending.clone(),
        };
        deliver(&pending, key, reply(key.2));
        assert!(receiver.try_recv().is_ok());
        // Delivery does not free the slot while the original HTTP future still owns it.
        assert!(pending.lock().unwrap().contains_key(&key));
        drop(guard);
        assert!(!pending.lock().unwrap().contains_key(&key));
        let (next, mut next_receiver) = oneshot::channel();
        pending.lock().unwrap().insert(key, Some(next));
        let next_guard = Pending {
            key,
            pending: pending.clone(),
        };
        deliver(&pending, key, reply(key.2));
        assert!(next_receiver.try_recv().is_ok());
        drop(next_guard);
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_http_waiter_releases_its_slot() {
        let key = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let pending = PendingReplies::default();
        let (sender, receiver) = oneshot::channel();
        pending.lock().unwrap().insert(key, Some(sender));
        let guard = Pending {
            key,
            pending: pending.clone(),
        };
        drop(receiver);
        drop(guard);
        assert!(pending.lock().unwrap().is_empty());
        deliver(&pending, key, reply(key.2));
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn cancelled_mutation_keeps_immutable_digest_and_exact_retry_can_receive_late_reply() {
        let key = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut fingerprints = MutationFingerprints::default();
        fingerprints
            .admit(key, [1; 32])
            .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        let pending = PendingReplies::default();
        let (sender, receiver) = oneshot::channel();
        pending.lock().unwrap().insert(key, Some(sender));
        let guard = Pending {
            key,
            pending: pending.clone(),
        };
        drop(receiver);
        drop(guard);
        assert!(fingerprints.admit(key, [2; 32]).is_err());
        assert!(pending.lock().unwrap().is_empty());
        fingerprints
            .admit(key, [1; 32])
            .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        let (sender, mut receiver) = oneshot::channel();
        pending.lock().unwrap().insert(key, Some(sender));
        let guard = Pending {
            key,
            pending: pending.clone(),
        };
        // An old response is only reusable for the identical immutable request.
        deliver(&pending, key, reply(key.2));
        assert!(receiver.try_recv().is_ok());
        drop(guard);
    }

    #[test]
    fn fingerprint_capacity_preserves_exact_retries_and_prunes_only_retired_candidates() {
        let machine = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let mut fingerprints = MutationFingerprints::default();
        let first = (machine, connection, Uuid::new_v4());
        fingerprints
            .admit(first, [1; 32])
            .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        for _ in 1..MUTATIONS_PER_CONNECTION {
            fingerprints
                .admit((machine, connection, Uuid::new_v4()), [1; 32])
                .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        }
        assert!(
            fingerprints
                .admit((machine, connection, Uuid::new_v4()), [1; 32])
                .is_err()
        );
        fingerprints
            .admit(first, [1; 32])
            .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        assert!(fingerprints.admit(first, [2; 32]).is_err());
        let candidates = vec![(machine, connection)];
        let replacement = (machine, Uuid::new_v4(), Uuid::new_v4());
        fingerprints
            .admit(replacement, [3; 32])
            .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        fingerprints.prune(&candidates, &[(machine, connection)]);
        assert!(fingerprints.0.contains_key(&(machine, connection)));
        fingerprints.prune(&candidates, &[]);
        assert!(!fingerprints.0.contains_key(&(machine, connection)));
        assert!(fingerprints.0.contains_key(&(replacement.0, replacement.1)));
        // Retired generations cannot disclose into a replacement's pending slot.
        let pending = PendingReplies::default();
        let (sender, mut receiver) = oneshot::channel();
        pending.lock().unwrap().insert(replacement, Some(sender));
        deliver(&pending, first, reply(first.2));
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        for _ in 1..RETAINED_GENERATIONS {
            fingerprints
                .admit((Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()), [1; 32])
                .unwrap_or_else(|_| panic!("remote relay fixture failed"));
        }
        assert!(
            fingerprints
                .admit((Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()), [1; 32])
                .is_err()
        );
    }

    #[test]
    fn readonly_ids_are_fresh_and_restored_without_consuming_mutation_capacity() {
        let requested = Uuid::new_v4();
        assert!(restore_command_id(reply(requested), requested).is_err());
        let operation = Operation::List {
            after: None,
            limit: 1,
        };
        let first = transport_id(&operation, requested);
        let second = transport_id(&operation, requested);
        assert_ne!(first, requested);
        assert_ne!(first, second);
        let command = Command {
            version: VERSION,
            connection_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            command_id: first,
            expires_at_ms: 42,
            operation,
        };
        assert!(
            mutation_digest(&Frame::Command {
                command: command.clone()
            })
            .unwrap_or_else(|_| panic!("remote relay fixture failed"))
            .is_none()
        );
        let frame = Frame::Result {
            connection_id: command.connection_id,
            command_id: first,
            reply: voyage_protocol::stream::Reply::Sessions { sessions: vec![] },
        };
        assert!(
            matches!(restore_command_id(frame, requested).unwrap_or_else(|_| panic!("remote relay fixture failed")), Frame::Result { command_id, .. } if command_id == requested)
        );
        let mut mutation = command;
        mutation.operation = Operation::Cancel {
            session_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
        };
        assert_eq!(transport_id(&mutation.operation, requested), requested);
        let digest = mutation_digest(&Frame::Command {
            command: mutation.clone(),
        })
        .unwrap_or_else(|_| panic!("remote relay fixture failed"))
        .unwrap();
        mutation.expires_at_ms += 1;
        assert_ne!(
            mutation_digest(&Frame::Command { command: mutation })
                .unwrap_or_else(|_| panic!("remote relay fixture failed"))
                .unwrap(),
            digest
        );
    }
}
