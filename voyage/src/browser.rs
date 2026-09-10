//! The remote Voyage owns policy and durable intents; the local executor owns the browser.
//! No browser process, provider loop, CDP or reverse shell is started here.
use crate::{attachment::journal::Journal, tools::ToolContext};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
use voyage_protocol::browser::*;

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    request: BrowserRequest,
    receipt: BrowserReceipt,
    result: Option<BrowserResult>,
    result_hash: Option<String>,
}
// Payloads and cached replies are bounded independently from never-evicted identities.
const MAX_RETIRED_REQUESTS: usize = 8192;
const MAX_CACHED_COMMANDS: usize = 256;
const MAX_COMMAND_IDENTITIES: usize = 65_792;
// Keep capacity for a result and cleanup per outstanding request when admission stops.
const COMMAND_RECOVERY_RESERVE: usize = 2 * MAX_BROWSER_REQUESTS;
#[derive(Clone, Serialize, Deserialize)]
struct RetiredEntry {
    binding: BrowserBinding,
    receipt: BrowserReceipt,
    result_hash: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Offer {
    binding: BrowserBinding,
    principal: Uuid,
    control: BrowserControl,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Durable {
    offer: Option<Offer>,
    entries: BTreeMap<Uuid, Entry>,
    commands: BTreeMap<Uuid, (String, BrowserReply)>,
    #[serde(default)]
    observations: BTreeMap<Uuid, Uuid>,
    #[serde(default)]
    retired_entries: BTreeMap<Uuid, RetiredEntry>,
    #[serde(default)]
    retired_commands: BTreeMap<Uuid, String>,
}
struct LiveRun {
    id: Uuid,
    cancel: tokio_util::sync::CancellationToken,
}
struct Inner {
    durable: Durable,
    run: Option<LiveRun>,
    guards: BTreeMap<Uuid, ToolContext>,
}

/// Only SQLite's explicit non-admission contention is retryable. Never retry I/O,
/// identity, schema or uncertain external/browser effects.
pub(crate) fn storage_busy(error: &anyhow::Error) -> bool {
    matches!(error.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_))
        if matches!(code.code,rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked))
}

pub struct BrowserBroker {
    directory: PathBuf,
    session: Uuid,
    incarnation: Uuid,
    inner: Mutex<Inner>,
}
impl std::fmt::Debug for BrowserBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserBroker")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn digest<T: Serialize>(v: &T) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(v)?)))
}
fn same_executor(a: &BrowserBinding, b: &BrowserBinding) -> bool {
    a.session_id == b.session_id
        && a.incarnation == b.incarnation
        && a.browser_id == b.browser_id
        && a.resource_id == b.resource_id
        && a.executor_id == b.executor_id
}
fn epochs(a: &BrowserBinding, b: &BrowserBinding) -> bool {
    a.controller_epoch == b.controller_epoch && a.capture_epoch == b.capture_epoch
}
fn unresolved(d: &Durable) -> bool {
    d.entries.values().any(|e| e.receipt.cleanup_pending)
}
fn fence(d: &mut Durable) {
    d.observations.clear();
    for e in d.entries.values_mut() {
        match e.receipt.state {
            BrowserRequestState::Pending => e.receipt.state = BrowserRequestState::Cancelled,
            BrowserRequestState::Dispatched => {
                e.receipt.state = BrowserRequestState::Unresolved;
                e.receipt.cleanup_pending = true;
            }
            _ => {}
        }
        // No queued capture may cross a privacy/control fence.
        e.result = None;
    }
}
impl BrowserBroker {
    pub fn open(directory: PathBuf, session: Uuid, incarnation: Uuid) -> Result<Arc<Self>> {
        let mut journal = Journal::open(directory.clone())?;
        let mut durable: Durable = journal
            .browser_load(session)?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        fence(&mut durable);
        if let Some(offer) = &mut durable.offer {
            offer.control = BrowserControl::Disconnected;
        }
        journal.browser_save(session, &serde_json::to_string(&durable)?)?;
        Ok(Arc::new(Self {
            directory,
            session,
            incarnation,
            inner: Mutex::new(Inner {
                durable,
                run: None,
                guards: BTreeMap::new(),
            }),
        }))
    }
    fn commit(&self, inner: &mut Inner, mut durable: Durable) -> Result<()> {
        // Never retire live waiters, unknown cleanup, or an unconsumed capture. A
        // retired request retains its original binding and receipt indefinitely.
        let ids: Vec<_> = durable
            .entries
            .iter()
            .filter_map(|(id, e)| {
                (!inner.guards.contains_key(id)
                    && !e.receipt.cleanup_pending
                    && e.result.is_none()
                    && !matches!(
                        e.receipt.state,
                        BrowserRequestState::Pending | BrowserRequestState::Dispatched
                    ))
                .then_some(*id)
            })
            .take(MAX_RETIRED_REQUESTS.saturating_sub(durable.retired_entries.len()))
            .collect();
        for id in ids {
            let e = durable
                .entries
                .remove(&id)
                .expect("selected retained entry");
            durable.retired_entries.insert(
                id,
                RetiredEntry {
                    binding: e.request.binding,
                    receipt: e.receipt,
                    result_hash: e.result_hash,
                },
            );
        }
        while durable.commands.len() > MAX_CACHED_COMMANDS {
            let (id, (hash, _)) = durable
                .commands
                .pop_first()
                .expect("nonempty command cache");
            durable.retired_commands.insert(id, hash);
        }
        Journal::open(self.directory.clone())?
            .browser_save(self.session, &serde_json::to_string(&durable)?)?;
        inner.durable = durable;
        Ok(())
    }
    fn expire(&self, inner: &mut Inner) -> Result<()> {
        let expired = inner.durable.offer.as_ref().is_some_and(|o| {
            o.binding.expires_at_ms <= now()
                && matches!(
                    o.control,
                    BrowserControl::Shared | BrowserControl::Human | BrowserControl::Private
                )
        });
        let cancelled = inner.run.as_ref().is_some_and(|r| r.cancel.is_cancelled());
        if expired || cancelled {
            let mut d = inner.durable.clone();
            fence(&mut d);
            if expired {
                if let Some(o) = &mut d.offer {
                    o.control = BrowserControl::Disconnected;
                }
            }
            self.commit(inner, d)?;
        }
        Ok(())
    }
    pub fn available(&self) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if self.expire(&mut inner).is_err() {
            return false;
        }
        inner.durable.offer.as_ref().is_some_and(|o| {
            o.binding.incarnation == self.incarnation
                && o.control == BrowserControl::Shared
                && o.binding.expires_at_ms > now()
        })
    }
    pub fn blocks_suspension(&self) -> Result<bool> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        self.expire(&mut inner)?;
        Ok(unresolved(&inner.durable)
            || inner.durable.offer.as_ref().is_some_and(|o| {
                o.binding.incarnation == self.incarnation
                    && o.binding.expires_at_ms > now()
                    && o.control != BrowserControl::Revoked
                    && o.control != BrowserControl::Disconnected
            }))
    }
    pub fn begin_run(&self, id: Uuid, cancel: tokio_util::sync::CancellationToken) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        self.expire(&mut inner)?;
        ensure!(
            !unresolved(&inner.durable),
            "browser dispatched effects require observed cleanup"
        );
        if inner
            .durable
            .offer
            .as_ref()
            .is_some_and(|o| o.binding.run_id.is_some_and(|bound| bound != id))
        {
            let mut d = inner.durable.clone();
            fence(&mut d);
            if let Some(o) = &mut d.offer {
                o.control = BrowserControl::Disconnected;
            }
            self.commit(&mut inner, d)?;
        }
        inner.run = Some(LiveRun { id, cancel });
        Ok(())
    }
    pub fn finish_run(&self) -> Result<bool> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        let mut d = inner.durable.clone();
        fence(&mut d);
        self.commit(&mut inner, d)?;
        inner.run = None;
        inner.guards.clear();
        Ok(!unresolved(&inner.durable))
    }
    pub fn operate(&self, principal: Uuid, operation: BrowserOperation) -> Result<BrowserReply> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        self.expire(&mut inner)?;
        let binding = operation.binding();
        ensure!(
            binding.session_id == self.session && !principal.is_nil(),
            "browser session authority mismatch"
        );
        let cleanup_read = matches!(
            operation,
            BrowserOperation::Receipt { .. }
                | BrowserOperation::Cleanup { .. }
                | BrowserOperation::Result { .. }
        );
        ensure!(
            cleanup_read || binding.incarnation == self.incarnation,
            "browser incarnation stale"
        );
        ensure!(
            !binding.browser_id.is_nil()
                && !binding.resource_id.is_nil()
                && !binding.executor_id.is_nil(),
            "browser identities required"
        );
        let command = match &operation {
            BrowserOperation::Offer { command_id, .. }
            | BrowserOperation::Claim { command_id, .. }
            | BrowserOperation::Result { command_id, .. }
            | BrowserOperation::Control { command_id, .. }
            | BrowserOperation::Cleanup { command_id, .. } => Some(*command_id),
            _ => None,
        };
        let hash = digest(&(principal, &operation))?;
        if let Some(id) = command {
            ensure!(!id.is_nil(), "browser command identity required");
            if let Some((previous, reply)) = inner.durable.commands.get(&id) {
                ensure!(*previous == hash, "browser command identity conflict");
                return Ok(reply.clone());
            }
            if let Some(previous) = inner.durable.retired_commands.get(&id) {
                ensure!(*previous == hash, "browser command identity conflict");
                anyhow::bail!(
                    "browser command reply retired; query current status/receipt, never replay effects"
                );
            }
            let recovery_id = match &operation {
                BrowserOperation::Cleanup {
                    request_id,
                    observed: true,
                    ..
                } => Some(request_id),
                BrowserOperation::Result { result, .. }
                    if result.state != BrowserRequestState::Unresolved =>
                {
                    Some(&result.request_id)
                }
                _ => None,
            };
            let recovery = recovery_id.is_some_and(|id| {
                inner
                    .durable
                    .entries
                    .get(id)
                    .is_some_and(|e| e.receipt.cleanup_pending)
            });
            let limit = MAX_COMMAND_IDENTITIES
                - if recovery {
                    0
                } else {
                    COMMAND_RECOVERY_RESERVE
                };
            ensure!(
                inner.durable.commands.len() + inner.durable.retired_commands.len() < limit,
                "browser command identity capacity reached; identities are never evicted (recovery reserve retained)"
            );
        }
        let mut d = inner.durable.clone();
        let retired = match &operation {
            BrowserOperation::Receipt { request_id, .. }
            | BrowserOperation::Cleanup { request_id, .. } => d.retired_entries.get(request_id),
            BrowserOperation::Result { result, .. } => d.retired_entries.get(&result.request_id),
            _ => None,
        };
        if let Some(e) = retired {
            ensure!(
                d.offer.as_ref().is_some_and(|o| o.principal == principal) && e.binding == *binding,
                "browser retired receipt authority mismatch"
            );
            if let BrowserOperation::Result { result, .. } = &operation {
                ensure!(
                    e.result_hash
                        .as_ref()
                        .is_some_and(|h| digest(result).is_ok_and(|hash| *h == hash)),
                    "browser result retired or identity conflict; query receipt, cleanup already observed"
                );
            }
            // Already positively quiescent; Cleanup cannot change website outcome.
            let reply = BrowserReply::Receipt {
                receipt: e.receipt.clone(),
            };
            if let Some(id) = command {
                d.commands.insert(id, (hash, reply.clone()));
            }
            self.commit(&mut inner, d)?;
            return Ok(reply);
        }
        let reply = match operation {
            BrowserOperation::Offer { binding, .. } => {
                ensure!(
                    binding.expires_at_ms > now()
                        && binding.expires_at_ms <= now().saturating_add(MAX_BROWSER_LEASE_MS),
                    "browser lease invalid"
                );
                ensure!(
                    binding.run_id.is_none()
                        || inner.run.as_ref().is_some_and(
                            |r| Some(r.id) == binding.run_id && !r.cancel.is_cancelled()
                        ),
                    "browser offer run stale"
                );
                ensure!(!unresolved(&d), "previous browser effects unresolved");
                if let Some(o) = &d.offer {
                    ensure!(
                        o.principal == principal,
                        "browser belongs to another authenticated principal"
                    );
                    ensure!(
                        matches!(
                            o.control,
                            BrowserControl::Revoked | BrowserControl::Disconnected
                        ) || o.binding.expires_at_ms <= now()
                            || (same_executor(&o.binding, &binding)
                                && (epochs(&o.binding, &binding)
                                    || (binding.controller_epoch > o.binding.controller_epoch
                                        && binding.capture_epoch > o.binding.capture_epoch))),
                        "browser already has a local executor"
                    );
                    if same_executor(&o.binding, &binding) && o.control != BrowserControl::Shared {
                        ensure!(
                            binding.controller_epoch > o.binding.controller_epoch
                                && binding.capture_epoch > o.binding.capture_epoch,
                            "renewed sharing requires fresh controller and capture epochs"
                        );
                    }
                }
                fence(&mut d);
                d.offer = Some(Offer {
                    binding,
                    principal,
                    control: BrowserControl::Shared,
                });
                Self::status(&d)?
            }
            BrowserOperation::Control {
                binding, control, ..
            } => {
                let o = d.offer.as_ref().context("browser not offered")?;
                ensure!(
                    o.principal == principal && same_executor(&o.binding, &binding),
                    "browser executor/principal mismatch"
                );
                ensure!(
                    binding.controller_epoch >= o.binding.controller_epoch
                        && binding.capture_epoch >= o.binding.capture_epoch,
                    "browser epoch stale"
                );
                if control != o.control
                    && matches!(
                        control,
                        BrowserControl::Shared | BrowserControl::Human | BrowserControl::Private
                    )
                {
                    ensure!(
                        binding.controller_epoch > o.binding.controller_epoch
                            && binding.capture_epoch > o.binding.capture_epoch,
                        "control transition requires fresh controller and capture epochs"
                    );
                }
                if matches!(
                    control,
                    BrowserControl::Shared | BrowserControl::Human | BrowserControl::Private
                ) {
                    ensure!(
                        binding.expires_at_ms > now()
                            && binding.expires_at_ms <= now().saturating_add(MAX_BROWSER_LEASE_MS),
                        "browser lease invalid"
                    );
                    ensure!(
                        o.control != BrowserControl::Disconnected
                            && o.control != BrowserControl::Revoked,
                        "disconnected browser requires explicit new offer"
                    );
                }
                // Heartbeats in any unchanged mode do not establish a new privacy
                // boundary. In particular, never erase queued captures on renew.
                if control != o.control || !epochs(&o.binding, &binding) {
                    fence(&mut d);
                }
                d.offer = Some(Offer {
                    binding,
                    principal,
                    control,
                });
                Self::status(&d)?
            }
            BrowserOperation::Pending { binding, limit } => {
                Self::authorized(&d, principal, &binding, true)?;
                ensure!(
                    (1..=16).contains(&limit),
                    "browser pending limit must be 1..16"
                );
                let mut requests = Vec::new();
                let mut bytes = 0;
                for e in d.entries.values().filter(|e| {
                    e.receipt.state == BrowserRequestState::Pending
                        && e.request.expires_at_ms > now()
                }) {
                    let size = serde_json::to_vec(&e.request)?.len();
                    if requests.len() >= usize::from(limit)
                        || bytes + size > MAX_BROWSER_RESULT_BYTES
                    {
                        break;
                    }
                    bytes += size;
                    requests.push(e.request.clone());
                }
                BrowserReply::Pending { requests }
            }
            BrowserOperation::Claim {
                binding,
                request_id,
                action_sha256,
                ..
            } => {
                Self::authorized(&d, principal, &binding, true)?;
                ensure!(
                    !d.entries
                        .values()
                        .any(|e| e.receipt.state == BrowserRequestState::Dispatched),
                    "one browser action may be dispatched at a time"
                );
                if let Some(target) = d
                    .entries
                    .get(&request_id)
                    .and_then(|e| target(&e.request.action))
                {
                    ensure!(
                        d.observations.get(&target.page_id) == Some(&target.observation_id),
                        "browser observation changed before claim"
                    );
                }
                let e = d
                    .entries
                    .get_mut(&request_id)
                    .context("unknown browser request")?;
                ensure!(
                    e.request.binding == binding && e.request.action_sha256 == action_sha256,
                    "browser request binding/hash conflict"
                );
                ensure!(
                    e.receipt.state == BrowserRequestState::Pending
                        && e.request.expires_at_ms > now(),
                    "browser request is not dispatchable; never replay a claim"
                );
                ensure!(
                    inner
                        .run
                        .as_ref()
                        .is_some_and(|r| Some(r.id) == binding.run_id && !r.cancel.is_cancelled()),
                    "browser run stale/cancelled"
                );
                let guard = inner
                    .guards
                    .get(&request_id)
                    .context("browser execution authority no longer live")?;
                guard.policy.check_execution_authority()?;
                ensure!(
                    e.request.action.observation_only()
                        || guard.policy.access_mode() != crate::config::AccessMode::ReadOnly,
                    "browser effects disabled in read-only mode before claim"
                );
                ensure!(!guard.cancellation.is_cancelled(), "browser call cancelled");
                e.receipt.state = BrowserRequestState::Dispatched;
                e.receipt.cleanup_pending = true;
                BrowserReply::Receipt {
                    receipt: e.receipt.clone(),
                }
            }
            BrowserOperation::Result {
                binding, result, ..
            } => {
                let o = d.offer.as_ref().context("browser not offered")?;
                ensure!(
                    o.principal == principal,
                    "browser result principal mismatch"
                );
                let capture_current = same_executor(&o.binding, &binding)
                    && epochs(&o.binding, &binding)
                    && o.control == BrowserControl::Shared
                    && o.binding.expires_at_ms > now()
                    && binding.incarnation == self.incarnation
                    && inner
                        .run
                        .as_ref()
                        .is_some_and(|r| Some(r.id) == binding.run_id && !r.cancel.is_cancelled())
                    && inner.guards.get(&result.request_id).is_some_and(|g| {
                        !g.cancellation.is_cancelled()
                            && g.policy.check_execution_authority().is_ok()
                    });
                ensure!(
                    serde_json::to_vec(&result)?.len() <= MAX_BROWSER_RESULT_BYTES,
                    "browser result exceeds bound"
                );
                ensure!(
                    matches!(
                        result.state,
                        BrowserRequestState::Completed
                            | BrowserRequestState::Refused
                            | BrowserRequestState::Cancelled
                            | BrowserRequestState::Unresolved
                    ),
                    "invalid browser result state"
                );
                ensure!(
                    result.text.len() <= 256 * 1024,
                    "browser text exceeds bound"
                );
                ensure!(
                    result.page_id.is_some() == result.observation_id.is_some(),
                    "browser page and observation must be paired"
                );
                if let Some(image) = &result.image {
                    use base64::Engine;
                    ensure!(
                        result.state == BrowserRequestState::Completed,
                        "noncompleted browser result cannot disclose images"
                    );
                    let bytes =
                        base64::engine::general_purpose::STANDARD.decode(&image.data_base64)?;
                    ensure!(
                        bytes.len() <= 2 * 1024 * 1024,
                        "browser image bound exceeded"
                    );
                    let (mime, _, _) = crate::images::validate(&bytes)?;
                    let actual = serde_json::to_value(mime)?;
                    ensure!(
                        actual.as_str() == Some(image.mime_type.as_str()),
                        "browser raster MIME mismatch"
                    );
                }
                ensure!(
                    result.file.is_none() || result.image.is_none(),
                    "browser result cannot combine binary transfers"
                );
                if let Some(file) = &result.file {
                    use base64::Engine;
                    ensure!(
                        result.state == BrowserRequestState::Completed,
                        "noncompleted browser result cannot disclose files"
                    );
                    ensure!(
                        !file.name.is_empty()
                            && file.name.len() <= 128
                            && !file
                                .name
                                .chars()
                                .any(|c| c.is_control() || c == '/' || c == '\\'),
                        "invalid browser download name"
                    );
                    ensure!(
                        !file.mime_type.is_empty()
                            && file.mime_type.len() <= 127
                            && !file.mime_type.chars().any(char::is_control),
                        "invalid browser download MIME"
                    );
                    let bytes =
                        base64::engine::general_purpose::STANDARD.decode(&file.data_base64)?;
                    ensure!(
                        !bytes.is_empty() && bytes.len() <= 2 * 1024 * 1024,
                        "browser file bound exceeded"
                    );
                }
                let result_hash = digest(&result)?;
                let e = d
                    .entries
                    .get_mut(&result.request_id)
                    .context("unknown browser request")?;
                ensure!(
                    e.request.binding == binding && e.request.action_sha256 == result.action_sha256,
                    "browser result binding/hash conflict"
                );
                ensure!(
                    result.file.is_none()
                        || matches!(e.request.action, BrowserAction::Download { .. }),
                    "binary files are only accepted for explicit download disclosure"
                );
                ensure!(
                    result.image.is_none()
                        || matches!(e.request.action, BrowserAction::Screenshot { .. }),
                    "images require explicit screenshot request"
                );
                if let Some(previous) = &e.result_hash {
                    ensure!(*previous == result_hash, "browser result identity conflict");
                } else {
                    ensure!(
                        matches!(
                            e.receipt.state,
                            BrowserRequestState::Dispatched | BrowserRequestState::Unresolved
                        ),
                        "browser result was never dispatched"
                    );
                    e.receipt.state = result.state;
                    e.receipt.cleanup_pending = result.state == BrowserRequestState::Unresolved;
                    if capture_current
                        && let (Some(page), Some(observation)) =
                            (result.page_id, result.observation_id)
                    {
                        ensure!(
                            !page.is_nil() && !observation.is_nil(),
                            "browser observation identity required"
                        );
                        ensure!(
                            d.observations.len() < 128 || d.observations.contains_key(&page),
                            "browser page bound exceeded"
                        );
                        d.observations.insert(page, observation);
                    }
                    e.result_hash = Some(result_hash);
                    e.result = capture_current.then_some(result);
                }
                BrowserReply::Receipt {
                    receipt: e.receipt.clone(),
                }
            }
            BrowserOperation::Receipt {
                binding,
                request_id,
            } => {
                let e = d
                    .entries
                    .get(&request_id)
                    .context("unknown browser request")?;
                let o = d.offer.as_ref().context("browser not offered")?;
                ensure!(
                    o.principal == principal && e.request.binding == binding,
                    "browser receipt authority mismatch"
                );
                BrowserReply::Receipt {
                    receipt: e.receipt.clone(),
                }
            }
            BrowserOperation::Cleanup {
                binding,
                request_id,
                observed,
                ..
            } => {
                let o = d.offer.as_ref().context("browser not offered")?;
                ensure!(
                    o.principal == principal,
                    "browser cleanup principal mismatch"
                );
                let e = d
                    .entries
                    .get_mut(&request_id)
                    .context("unknown browser request")?;
                ensure!(
                    e.request.binding == binding,
                    "browser cleanup binding mismatch"
                );
                ensure!(
                    e.receipt.state != BrowserRequestState::Pending,
                    "pending request must first be cancelled"
                );
                if observed {
                    e.receipt.cleanup_pending = false;
                    if e.receipt.state == BrowserRequestState::Dispatched {
                        e.receipt.state = BrowserRequestState::Unresolved;
                    }
                }
                // This is executor attestation of cleanup, not evidence of successful effects.
                BrowserReply::Receipt {
                    receipt: e.receipt.clone(),
                }
            }
        };
        if let Some(id) = command {
            d.commands.insert(id, (hash, reply.clone()));
        }
        self.commit(&mut inner, d)?;
        Ok(reply)
    }
    fn authorized(
        d: &Durable,
        principal: Uuid,
        binding: &BrowserBinding,
        shared: bool,
    ) -> Result<()> {
        let o = d.offer.as_ref().context("browser not offered")?;
        ensure!(
            o.principal == principal
                && same_executor(&o.binding, binding)
                && epochs(&o.binding, binding),
            "browser authority/epoch mismatch"
        );
        ensure!(
            !shared || (o.control == BrowserControl::Shared && o.binding.expires_at_ms > now()),
            "browser is not locally shared"
        );
        Ok(())
    }
    fn status(d: &Durable) -> Result<BrowserReply> {
        let o = d.offer.as_ref().context("browser not offered")?;
        Ok(BrowserReply::Status {
            status: BrowserStatus {
                binding: o.binding.clone(),
                control: o.control,
                available: o.control == BrowserControl::Shared && o.binding.expires_at_ms > now(),
                cleanup_pending: unresolved(d),
            },
        })
    }
    pub(crate) fn enqueue(
        &self,
        action: BrowserAction,
        context: &ToolContext,
        id: Uuid,
    ) -> Result<Uuid> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        self.expire(&mut inner)?;
        context.policy.check_execution_authority()?;
        ensure!(
            !context.cancellation.is_cancelled(),
            "browser tool cancelled"
        );
        ensure!(
            inner.durable.commands.len() + inner.durable.retired_commands.len()
                < MAX_COMMAND_IDENTITIES - COMMAND_RECOVERY_RESERVE,
            "browser command admission full; cleanup capacity reserved"
        );
        let run = inner
            .run
            .as_ref()
            .context("browser requires active root run")?;
        ensure!(!run.cancel.is_cancelled(), "browser run cancelled");
        let o = inner
            .durable
            .offer
            .as_ref()
            .context("browser not offered")?;
        Self::authorized(&inner.durable, o.principal, &o.binding, true)?;
        ensure!(
            inner.durable.entries.len() < MAX_BROWSER_REQUESTS
                && inner.durable.retired_entries.len() < MAX_RETIRED_REQUESTS,
            "browser live request or retired identity bound reached; exact identities cannot be evicted"
        );
        let mut binding = o.binding.clone();
        binding.run_id = Some(run.id);
        ensure!(
            !inner.durable.entries.contains_key(&id)
                && !inner.durable.retired_entries.contains_key(&id),
            "browser tool identity already exists; query receipt, never repeat execution"
        );
        let target = target(&action);
        if let Some(target) = target {
            ensure!(
                inner.durable.observations.get(&target.page_id) == Some(&target.observation_id),
                "stale browser page or observation; inspect again"
            );
        }
        let hash = digest(&action)?;
        let request = BrowserRequest {
            request_id: id,
            binding,
            action,
            action_sha256: hash.clone(),
            // Request deadline is independent of the renewable executor lease.
            // Claim and local dispatch still check the CURRENT lease and epochs.
            expires_at_ms: now().saturating_add(context.timeout.as_millis().min(60_000) as u64),
        };
        let mut d = inner.durable.clone();
        d.entries.insert(
            id,
            Entry {
                request,
                receipt: BrowserReceipt {
                    request_id: id,
                    action_sha256: hash,
                    state: BrowserRequestState::Pending,
                    cleanup_pending: false,
                },
                result: None,
                result_hash: None,
            },
        );
        self.commit(&mut inner, d)?;
        inner.guards.insert(id, context.clone());
        Ok(id)
    }
    pub(crate) fn poll(
        &self,
        id: Uuid,
        cancel: bool,
    ) -> Result<(BrowserReceipt, Option<BrowserResult>)> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("browser lock poisoned"))?;
        self.expire(&mut inner)?;
        let mut d = inner.durable.clone();
        let e = d.entries.get_mut(&id).context("browser request missing")?;
        if cancel || e.request.expires_at_ms <= now() {
            match e.receipt.state {
                BrowserRequestState::Pending => e.receipt.state = BrowserRequestState::Cancelled,
                BrowserRequestState::Dispatched => {
                    e.receipt.state = BrowserRequestState::Unresolved;
                    e.receipt.cleanup_pending = true;
                }
                _ => {}
            }
            e.result = None;
        }
        let reply = (e.receipt.clone(), e.result.take());
        let terminal = !matches!(
            reply.0.state,
            BrowserRequestState::Pending | BrowserRequestState::Dispatched
        );
        if cancel
            || reply.1.is_some()
            || !matches!(
                reply.0.state,
                BrowserRequestState::Pending | BrowserRequestState::Dispatched
            )
        {
            self.commit(&mut inner, d)?;
        }
        // Keep the waiter guard until its checkpoint is durable. A busy database must
        // not let a concurrent lease/control commit retire a still-waiting tool.
        if terminal {
            inner.guards.remove(&id);
        }
        Ok(reply)
    }
}

fn target(action: &BrowserAction) -> Option<&BrowserTarget> {
    match action {
        BrowserAction::Navigate { target, .. }
        | BrowserAction::Click { target, .. }
        | BrowserAction::Fill { target, .. }
        | BrowserAction::Scroll { target, .. }
        | BrowserAction::Screenshot { target, .. }
        | BrowserAction::Upload { target, .. }
        | BrowserAction::Download { target, .. }
        | BrowserAction::Tabs {
            operation: BrowserTabs::Select { target } | BrowserTabs::Close { target },
        } => Some(target),
        _ => None,
    }
}
