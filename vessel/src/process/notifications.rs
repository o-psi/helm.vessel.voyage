//! Durable attention references. This service never admits a decision or a run.
//! Source delegation is account-local; recipients separately consent to retention.
mod store;

use super::{access::store as access, registry, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use uuid::Uuid;
use voyage_protocol::{
    notifications::*,
    process::{
        ConnectionGrant, GrantBinding, ProcessGrant, ProcessRegistration, ProcessRight,
        ProcessState, RuntimeInitialization,
    },
    vessel::{VoyageCommand, VoyageRequest},
};

impl Supervisor {
    /// Re-read authority on each observation, including after asynchronous owner reads.
    /// Configured destinations are not authority caches and never contain credentials.
    fn notification_authority(
        &self,
        destination: &Destination,
        right: ProcessRight,
        registration: &ProcessRegistration,
    ) -> Result<Option<GrantBinding>> {
        let vessel = super::identity::public(&self.directory)?.vessel_id;
        ensure!(
            destination.source_vessel_id == vessel,
            "notification source unavailable"
        );
        ensure!(
            registration.session_id == destination.source_session_id,
            "notification source unavailable"
        );
        ensure!(
            registration.state != ProcessState::Relinquished
                && !matches!(
                    registration.initialize,
                    Some(RuntimeInitialization::Participant { .. })
                ),
            "notification source unavailable"
        );
        if destination.recipient_grant_id == vessel {
            ensure!(
                destination.recipient_principal_id == vessel
                    && destination.recipient_grant_revision == 1,
                "notification recipient unavailable"
            );
            return Ok(None);
        }
        let id = destination.recipient_grant_id;
        if access::connection_path(&self.directory, id).exists() {
            let grant: ConnectionGrant =
                access::load(&access::connection_path(&self.directory, id))?;
            access::current_connection(&self.directory, &grant)?;
            ensure!(
                grant.grant_id == id
                    && grant.principal_id == destination.recipient_principal_id
                    && grant.revision == destination.recipient_grant_revision
                    && grant.rights.contains(&right)
                    && destination.expires_at_ms <= grant.expires_at_ms
                    && (!destination.event_kinds.contains(&NotificationKind::Budget)
                        || grant.rights.contains(&ProcessRight::History))
                    && grant
                        .workspaces
                        .iter()
                        .any(|w| w.path == registration.workspace),
                "notification recipient unavailable"
            );
            return Ok(Some(self.connection_session(
                &grant,
                registration.session_id,
                &registration.workspace,
            )?));
        }
        let grant: ProcessGrant = access::load(&access::grant_path(&self.directory, id))?;
        access::current(&grant)?;
        // Delegated participant/derived session tokens are not human inbox identities.
        // Use the originating workspace connection, not its derived credential.
        ensure!(
            grant.grant_id == id
                && grant.principal_id == destination.recipient_principal_id
                && grant.revision == destination.recipient_grant_revision
                && grant.session_id == registration.session_id
                && grant.workspace == registration.workspace
                && grant.parent_grant.is_none()
                && grant.participant_binding.is_none()
                && grant.connection_binding.is_none()
                && grant.rights.contains(&right)
                && (!destination.event_kinds.contains(&NotificationKind::Budget)
                    || grant.rights.contains(&ProcessRight::History))
                && destination.expires_at_ms <= grant.expires_at_ms,
            "notification recipient unavailable"
        );
        Ok(Some(GrantBinding {
            grant_id: id,
            principal_id: grant.principal_id,
            revision: grant.revision,
        }))
    }

    fn notification_recipient(
        destination: &Destination,
        actor: &Option<GrantBinding>,
        vessel: Uuid,
    ) -> bool {
        match actor {
            None => {
                destination.recipient_grant_id == vessel
                    && destination.recipient_principal_id == vessel
                    && destination.recipient_grant_revision == 1
            }
            Some(actor) => {
                actor.grant_id == destination.recipient_grant_id
                    && actor.principal_id == destination.recipient_principal_id
                    && actor.revision == destination.recipient_grant_revision
            }
        }
    }
}

impl Supervisor {
    pub(super) async fn notifications(
        &self,
        operation: NotificationOperation,
        actor: Option<GrantBinding>,
    ) -> Result<Value> {
        if let NotificationOperation::Open {
            destination_id,
            event_id,
        } = operation
        {
            return self
                .open_notification(destination_id, event_id, actor)
                .await;
        }
        // This is also the local grant/revocation ordering lock. No network wait
        // occurs while it is held. A subsequent owner read rechecks separately.
        let registrations = self.registrations.lock().await;
        let store = store::Store::open_current(self.directory.join("notifications"))?;
        let vessel = super::identity::public(&self.directory)?.vessel_id;
        let now = access::now()?;
        if let NotificationOperation::Configure {
            command_id,
            destination,
        } = operation
        {
            ensure!(
                actor.is_none(),
                "notification configuration requires local source authority"
            );
            let registration = registrations
                .get(&destination.source_session_id)
                .ok_or_else(|| anyhow::anyhow!("notification source unavailable"))?;
            self.notification_authority(&destination, ProcessRight::Observe, registration)?;
            return Ok(serde_json::to_value(store.configure(
                command_id,
                destination,
                now,
            )?)?);
        }
        if matches!(operation, NotificationOperation::Attention) {
            let mut available = 0u64;
            for record in store.destinations()? {
                let d = &record.destination;
                if Self::notification_recipient(d, &actor, vessel)
                    && !d.quiet_hours_utc.is_some_and(|hours| hours.contains(now))
                    && registrations
                        .get(&d.source_session_id)
                        .is_some_and(|registration| {
                            self.notification_authority(d, ProcessRight::Observe, registration)
                                .is_ok()
                        })
                {
                    available += store.attention_count(d.id, now)?;
                }
            }
            return Ok(json!({"available":available,"observation_is_not_approval":true}));
        }
        if matches!(operation, NotificationOperation::Destinations) {
            let mut records = Vec::new();
            let mut delivery = Vec::new();
            for record in store.destinations()? {
                let destination = &record.destination;
                if actor.is_none() {
                    delivery.push(json!({"destination_id":destination.id,"producer":store.cursor(destination.id)?,"budget_delivery":self.budget_delivery_status(destination.id)}));
                    records.push(serde_json::to_value(record)?);
                } else if Self::notification_recipient(destination, &actor, vessel)
                    && registrations
                        .get(&destination.source_session_id)
                        .is_some_and(|registration| {
                            self.notification_authority(
                                destination,
                                ProcessRight::Observe,
                                registration,
                            )
                            .is_ok()
                        })
                {
                    delivery.push(json!({"destination_id":destination.id,"producer":store.cursor(destination.id)?,"budget_delivery":self.budget_delivery_status(destination.id)}));
                    records.push(serde_json::to_value(record)?);
                }
            }
            return Ok(
                json!({"destinations":records,"delivery":delivery,"local_recipient_id":if actor.is_none(){Some(vessel)}else{None}}),
            );
        }
        let destination_id = match &operation {
            NotificationOperation::Accept { destination_id, .. }
            | NotificationOperation::Revoke { destination_id, .. }
            | NotificationOperation::Test { destination_id, .. }
            | NotificationOperation::Inbox { destination_id, .. }
            | NotificationOperation::Receipt { destination_id, .. } => *destination_id,
            _ => anyhow::bail!("unsupported notification operation"),
        };
        let record = store
            .destination(destination_id)?
            .ok_or_else(|| anyhow::anyhow!("notification destination unavailable"))?;
        let destination = &record.destination;
        let recipient = Self::notification_recipient(destination, &actor, vessel);
        // Revocation cannot be blocked by source disappearance or full inboxes.
        if let NotificationOperation::Revoke { command_id, .. } = operation {
            ensure!(
                actor.is_none() || recipient,
                "notification destination unavailable"
            );
            store.revoke(command_id, destination_id, now)?;
            return Ok(
                json!({"destination_id":destination_id,"revoked":true,"execution_cleanup":"not_implied"}),
            );
        }
        let registration = registrations
            .get(&destination.source_session_id)
            .ok_or_else(|| anyhow::anyhow!("notification source unavailable"))?;
        self.notification_authority(destination, ProcessRight::Observe, registration)?;
        match operation {
            NotificationOperation::Accept { command_id, .. } => {
                ensure!(recipient, "notification destination unavailable");
                Ok(serde_json::to_value(store.accept(
                    command_id,
                    destination_id,
                    now,
                )?)?)
            }
            NotificationOperation::Test { command_id, .. } => {
                ensure!(
                    actor.is_none(),
                    "notification test requires local source authority"
                );
                Ok(serde_json::to_value(store.test(
                    command_id,
                    destination_id,
                    registration.incarnation,
                    now,
                )?)?)
            }
            NotificationOperation::Inbox { after, limit, .. } => {
                ensure!(recipient, "notification destination unavailable");
                let page = store.inbox(destination_id, after, limit, now)?;
                Ok(
                    json!({"page":page,"producer":store.cursor(destination_id)?,"budget_delivery":self.budget_delivery_status(destination_id),
                    "attention_deferred":destination.quiet_hours_utc.is_some_and(|hours|hours.contains(now)),
                    "destination_expires_at_ms":destination.expires_at_ms}),
                )
            }
            NotificationOperation::Receipt {
                event_id, state, ..
            } => {
                ensure!(recipient, "notification destination unavailable");
                Ok(serde_json::to_value(store.receipt(
                    destination_id,
                    event_id,
                    state,
                    now,
                )?)?)
            }
            _ => anyhow::bail!("unsupported notification operation"),
        }
    }

    async fn open_notification(
        &self,
        destination_id: Uuid,
        event_id: Uuid,
        actor: Option<GrantBinding>,
    ) -> Result<Value> {
        let (destination, entry, binding) = {
            let registrations = self.registrations.lock().await;
            let store = store::Store::open_current(self.directory.join("notifications"))?;
            let vessel = super::identity::public(&self.directory)?.vessel_id;
            let record = store
                .destination(destination_id)?
                .ok_or_else(|| anyhow::anyhow!("notification destination unavailable"))?;
            ensure!(
                Self::notification_recipient(&record.destination, &actor, vessel),
                "notification destination unavailable"
            );
            let Some(registration) = registrations.get(&record.destination.source_session_id)
            else {
                return Ok(json!({"status":"unavailable","actionable":false}));
            };
            if self
                .notification_authority(&record.destination, ProcessRight::Observe, registration)
                .is_err()
            {
                return Ok(json!({"status":"unavailable","actionable":false}));
            }
            let Some(entry) = store.get(destination_id, event_id, access::now()?)? else {
                return Ok(json!({"status":"expired_or_revoked","actionable":false}));
            };
            // Never fetch all requests under local authority then filter for a
            // recipient. Exact action previews require that recipient's Decide.
            let binding = if entry.notification.decision_id.is_some() {
                match self.notification_authority(
                    &record.destination,
                    ProcessRight::Decide,
                    registration,
                ) {
                    Ok(binding) => binding,
                    Err(_) => {
                        return Ok(
                            json!({"status":"decision_authority_unavailable","actionable":false}),
                        );
                    }
                }
            } else {
                self.notification_authority(
                    &record.destination,
                    ProcessRight::Observe,
                    registration,
                )?
            };
            (record.destination, entry, binding)
        };
        let notification = &entry.notification;
        let Some(decision_id) = notification.decision_id else {
            // A source address is not a capability. Inspect/history are separate
            // operations with their own rights; this does not switch Helm's view.
            return Ok(
                json!({"status":"current","actionable":false,"notification":notification,
                "source":{"session_id":notification.session_id,"run_id":notification.run_id},
                "execution_cleanup":"not_implied"}),
            );
        };
        let Some(incarnation) = notification.incarnation else {
            return Ok(json!({"status":"stale","actionable":false}));
        };
        let reply = self
            .voyage(
                VoyageRequest {
                    session_id: notification.session_id,
                    incarnation: Some(incarnation),
                    command: VoyageCommand::Decisions,
                },
                binding,
            )
            .await;
        let Ok(reply) = reply else {
            return Ok(json!({"status":"unavailable","actionable":false}));
        };
        let Some(decision) = reply
            .get("result")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("decision_id").and_then(Value::as_str)
                        == Some(decision_id.to_string().as_str())
                        && row.get("incarnation").and_then(Value::as_str)
                            == Some(incarnation.to_string().as_str())
                        && row.get("run_id").and_then(Value::as_str)
                            == notification.run_id.map(|id| id.to_string()).as_deref()
                })
            })
            .cloned()
        else {
            return Ok(json!({"status":"resolved_or_expired","actionable":false}));
        };
        // Current identity, request and lifetime are sampled again after the owner
        // wait. The existing explicit Respond admission remains the final arbiter.
        let registrations = self.registrations.lock().await;
        let store = store::Store::open_current(self.directory.join("notifications"))?;
        let now = access::now()?;
        let Some(registration) = registrations.get(&destination.source_session_id) else {
            return Ok(json!({"status":"unavailable","actionable":false}));
        };
        if registration.incarnation != incarnation
            || self
                .notification_authority(&destination, ProcessRight::Decide, registration)
                .is_err()
            || store.get(destination_id, event_id, now)?.is_none()
            || decision
                .get("expires_at_ms")
                .and_then(Value::as_u64)
                .is_none_or(|expires| expires <= now)
        {
            return Ok(json!({"status":"stale","actionable":false}));
        }
        Ok(
            json!({"status":"current","actionable":false,"notification":notification,"decision":decision,
            "decision_requires":"separate_explicit_owner_response","approval_granted":false}),
        )
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryAttempt {
    failures: u8,
    next_attempt_ms: u64,
    drained_incarnation: Option<Uuid>,
    gap: bool,
}

#[derive(serde::Deserialize)]
struct SourcePage {
    events: Vec<SourceEvent>,
    next_after: u64,
    has_more: bool,
    gap: bool,
}
#[derive(serde::Deserialize)]
struct SourceEvent {
    sequence: u64,
    source_event_id: Uuid,
    session_id: Uuid,
    run_id: Uuid,
    incarnation: Option<Uuid>,
    kind: NotificationKind,
    created_at_ms: u64,
    expires_at_ms: Option<u64>,
    decision_id: Option<Uuid>,
}

impl Supervisor {
    /// A bounded metadata courier, not an agent loop or a wake-up scheduler.
    /// No destination means no owner polling, and drained suspended owners are
    /// not repeatedly restarted merely to discover that nothing has changed.
    pub(super) fn start_notification_delivery(
        self: &std::sync::Arc<Self>,
    ) -> tokio::task::JoinHandle<()> {
        let supervisor = std::sync::Arc::downgrade(self);
        tokio::spawn(async move {
            let mut offset = 0usize;
            loop {
                let Some(supervisor) = supervisor.upgrade() else {
                    break;
                };
                let records = {
                    let _serial = supervisor.registrations.lock().await;
                    store::Store::open_current(supervisor.directory.join("notifications"))
                        .and_then(|store| store.destinations())
                        .unwrap_or_default()
                };
                if !records.is_empty() {
                    for index in 0..records.len().min(8) {
                        let record = &records[(offset + index) % records.len()];
                        // Failures remain in the private bounded cursor/attempt
                        // record, never in public subprocess diagnostics.
                        let _ = supervisor
                            .deliver_notifications(record.destination.id)
                            .await;
                        let _ = supervisor
                            .deliver_budget_notifications(record.destination.id)
                            .await;
                    }
                    offset = (offset + 8) % records.len();
                }
                drop(supervisor);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        })
    }

    async fn deliver_notifications(&self, destination_id: Uuid) -> Result<()> {
        let attempt_path = self
            .directory
            .join("notifications")
            .join(format!("delivery-{destination_id}.json"));
        let (destination, accepted_at, registration, cursor, mut attempt, stopped_before) = {
            let registrations = self.registrations.lock().await;
            let store = store::Store::open_current(self.directory.join("notifications"))?;
            let now = access::now()?;
            let record = store
                .destination(destination_id)?
                .ok_or_else(|| anyhow::anyhow!("destination unavailable"))?;
            let Some(accepted_at) = record.accepted_at_ms else {
                return Ok(());
            };
            if record.revoked_at_ms.is_some() || record.destination.expires_at_ms <= now {
                return Ok(());
            }
            let Some(registration) = registrations
                .get(&record.destination.source_session_id)
                .cloned()
            else {
                store.advance(destination_id, 0, Some(ProducerError::Unavailable))?;
                return Ok(());
            };
            if self
                .notification_authority(&record.destination, ProcessRight::Observe, &registration)
                .is_err()
            {
                store.advance(destination_id, 0, Some(ProducerError::Unavailable))?;
                return Ok(());
            }
            let mut attempt: DeliveryAttempt = if attempt_path.exists() {
                access::load(&attempt_path)?
            } else {
                DeliveryAttempt::default()
            };
            if attempt.failures >= 5
                || attempt.next_attempt_ms > now
                || attempt.drained_incarnation == Some(registration.incarnation)
            {
                return Ok(());
            }
            let cursor = store.cursor(destination_id)?;
            // Persist send/read attempt identity before IPC. A crash repeats only
            // this read plus immutable metadata IDs, never an execution command.
            attempt.failures += 1;
            attempt.next_attempt_ms = now
                .checked_add(1000u64 << attempt.failures.min(5))
                .ok_or_else(|| anyhow::anyhow!("clock overflow"))?;
            access::save(&attempt_path, &attempt)?;
            let stopped = super::recovery::clean_stop(
                &registry::directory(&self.directory, registration.session_id),
                &registration,
            );
            (
                record.destination,
                accepted_at,
                registration,
                cursor,
                attempt,
                stopped,
            )
        };
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.forward_current(
                &registry::directory(&self.directory, registration.session_id),
                &registration,
                voyage_protocol::process::RuntimeCommand::NotificationEvents {
                    after: cursor.after,
                    limit: 64,
                },
                None,
            ),
        )
        .await;
        let registrations = self.registrations.lock().await;
        let store = store::Store::open_current(self.directory.join("notifications"))?;
        let now = access::now()?;
        let latest = store
            .destination(destination_id)?
            .ok_or_else(|| anyhow::anyhow!("destination unavailable"))?;
        let Some(current) = registrations.get(&destination.source_session_id) else {
            return Ok(());
        };
        if latest.revoked_at_ms.is_some()
            || latest.destination != destination
            || destination.expires_at_ms <= now
            || current.incarnation != registration.incarnation
            || self
                .notification_authority(&destination, ProcessRight::Observe, current)
                .is_err()
        {
            store.advance(
                destination_id,
                cursor.after,
                Some(ProducerError::Unavailable),
            )?;
            return Ok(());
        }
        let page = match result {
            Ok(Ok(response)) if response.error.is_none() => {
                serde_json::from_value::<SourcePage>(response.result).ok()
            }
            _ => None,
        };
        let Some(page) = page else {
            store.advance(
                destination_id,
                cursor.after,
                Some(ProducerError::Unavailable),
            )?;
            return Ok(());
        };
        if page.events.len() > 64 || page.next_after < cursor.after {
            store.advance(
                destination_id,
                cursor.after,
                Some(ProducerError::InvalidSource),
            )?;
            return Ok(());
        }
        let mut previous = cursor.after;
        for event in page.events {
            if event.sequence <= previous
                || event.sequence > page.next_after
                || event.session_id != destination.source_session_id
                || event.source_event_id.is_nil()
                || event.run_id.is_nil()
                || event.created_at_ms > now
                || matches!(
                    event.kind,
                    NotificationKind::Test | NotificationKind::Budget
                )
                || (event.kind == NotificationKind::Attention) != event.decision_id.is_some()
            {
                store.advance(
                    destination_id,
                    cursor.after,
                    Some(ProducerError::InvalidSource),
                )?;
                return Ok(());
            }
            previous = event.sequence;
            if event.created_at_ms < accepted_at || !destination.event_kinds.contains(&event.kind) {
                continue;
            }
            let Some(expiry) = event
                .created_at_ms
                .checked_add(destination.notification_ttl_ms)
            else {
                store.advance(
                    destination_id,
                    cursor.after,
                    Some(ProducerError::InvalidSource),
                )?;
                return Ok(());
            };
            let expiry = expiry
                .min(destination.expires_at_ms)
                .min(event.expires_at_ms.unwrap_or(u64::MAX));
            if expiry <= now {
                continue;
            }
            let notification = Notification {
                event_id: event.source_event_id,
                session_id: event.session_id,
                run_id: Some(event.run_id),
                incarnation: event.incarnation,
                kind: event.kind,
                source_event_id: event.source_event_id,
                created_at_ms: event.created_at_ms,
                expires_at_ms: expiry,
                decision_id: event.decision_id,
                budget: None,
            };
            if store.publish(destination_id, notification, now).is_err() {
                store.advance(
                    destination_id,
                    cursor.after,
                    Some(ProducerError::Unavailable),
                )?;
                return Ok(());
            }
        }
        store.advance(destination_id, page.next_after, None)?;
        attempt.gap |= page.gap;
        if attempt.gap {
            store.advance(destination_id, page.next_after, Some(ProducerError::Gap))?;
        }
        attempt.failures = 0;
        attempt.next_attempt_ms = 0;
        if stopped_before
            && !page.has_more
            && super::recovery::clean_stop(
                &registry::directory(&self.directory, current.session_id),
                current,
            )
        {
            attempt.drained_incarnation = Some(current.incarnation);
        }
        access::save(&attempt_path, &attempt)?;
        Ok(())
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetDeliveryAttempt {
    state: DeliveryAttempt,
    after: u64,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceBudgetScope {
    Session(Uuid),
    Project(Uuid),
}
#[derive(serde::Deserialize)]
struct SourceBudgetEvent {
    source_event_id: Uuid,
    ledger_sequence: u64,
    scope: SourceBudgetScope,
    revision: u64,
    dimension: BudgetDimension,
    threshold: BudgetThreshold,
    created_at: String,
    session: Uuid,
    run: Uuid,
}
#[derive(serde::Deserialize)]
struct SourceBudgetPage {
    schema_version: u32,
    events: Vec<SourceBudgetEvent>,
    next_after: u64,
}

impl Supervisor {
    fn budget_delivery_status(&self, destination_id: Uuid) -> Value {
        let path = self
            .directory
            .join("notifications")
            .join(format!("budget-delivery-{destination_id}.json"));
        if !path.exists() {
            return Value::Null;
        }
        match access::load::<BudgetDeliveryAttempt>(&path) {
            Ok(attempt) => {
                json!({"after":attempt.after,"pending_or_unavailable":attempt.state.failures>0,
                "attempts":attempt.state.failures,"stopped_after_failures":attempt.state.failures>=5})
            }
            Err(_) => json!({"pending_or_unavailable":true,"state_unavailable":true}),
        }
    }

    async fn deliver_budget_notifications(&self, destination_id: Uuid) -> Result<()> {
        let path = self
            .directory
            .join("notifications")
            .join(format!("budget-delivery-{destination_id}.json"));
        let (destination, accepted, registration, mut attempt, stopped) = {
            let registrations = self.registrations.lock().await;
            let store = store::Store::open_current(self.directory.join("notifications"))?;
            let Some(record) = store.destination(destination_id)? else {
                return Ok(());
            };
            let Some(accepted) = record.accepted_at_ms else {
                return Ok(());
            };
            let now = access::now()?;
            if record.revoked_at_ms.is_some()
                || record.destination.expires_at_ms <= now
                || !record
                    .destination
                    .event_kinds
                    .contains(&NotificationKind::Budget)
            {
                return Ok(());
            }
            let Some(registration) = registrations
                .get(&record.destination.source_session_id)
                .cloned()
            else {
                return Ok(());
            };
            self.notification_authority(&record.destination, ProcessRight::Observe, &registration)?;
            let mut attempt: BudgetDeliveryAttempt = if path.exists() {
                access::load(&path)?
            } else {
                BudgetDeliveryAttempt::default()
            };
            if attempt.state.failures >= 5
                || attempt.state.next_attempt_ms > now
                || attempt.state.drained_incarnation == Some(registration.incarnation)
            {
                return Ok(());
            }
            attempt.state.failures += 1;
            attempt.state.next_attempt_ms = now
                .checked_add(1000u64 << attempt.state.failures.min(5))
                .ok_or_else(|| anyhow::anyhow!("clock overflow"))?;
            access::save(&path, &attempt)?;
            let stopped = super::recovery::clean_stop(
                &registry::directory(&self.directory, registration.session_id),
                &registration,
            );
            (record.destination, accepted, registration, attempt, stopped)
        };
        // Compatibility boundary with #71's separately delivered owner accounting
        // API. An older protocol refuses; it never falls back to inferred totals.
        let command = serde_json::from_value::<voyage_protocol::process::RuntimeCommand>(json!({
            "op":"budget_events","after":attempt.after,"limit":64
        }));
        let response = match command {
            Ok(command) => tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.forward_current(
                    &registry::directory(&self.directory, registration.session_id),
                    &registration,
                    command,
                    None,
                ),
            )
            .await
            .ok()
            .and_then(Result::ok),
            Err(_) => None,
        };
        let registrations = self.registrations.lock().await;
        let store = store::Store::open_current(self.directory.join("notifications"))?;
        let now = access::now()?;
        let Some(record) = store.destination(destination_id)? else {
            return Ok(());
        };
        let Some(current) = registrations.get(&destination.source_session_id) else {
            return Ok(());
        };
        if record.revoked_at_ms.is_some()
            || record.destination != destination
            || destination.expires_at_ms <= now
            || current.incarnation != registration.incarnation
            || self
                .notification_authority(&destination, ProcessRight::Observe, current)
                .is_err()
        {
            return Ok(());
        }
        let page = response
            .filter(|response| response.error.is_none())
            .and_then(|response| serde_json::from_value::<SourceBudgetPage>(response.result).ok());
        let Some(page) = page else {
            store.advance(destination_id, 0, Some(ProducerError::Unavailable))?;
            return Ok(());
        };
        if page.schema_version != 1 || page.events.len() > 64 || page.next_after < attempt.after {
            store.advance(destination_id, 0, Some(ProducerError::InvalidSource))?;
            return Ok(());
        }
        let count = page.events.len();
        let mut previous = attempt.after;
        for event in page.events {
            let timestamp = chrono::DateTime::parse_from_rfc3339(&event.created_at)
                .ok()
                .and_then(|time| u64::try_from(time.timestamp_millis()).ok());
            let Some(created) = timestamp else {
                store.advance(destination_id, 0, Some(ProducerError::InvalidSource))?;
                return Ok(());
            };
            if event.ledger_sequence <= previous
                || event.ledger_sequence > page.next_after
                || event.session != destination.source_session_id
                || event.source_event_id.is_nil()
                || event.run.is_nil()
                || created > now
            {
                store.advance(destination_id, 0, Some(ProducerError::InvalidSource))?;
                return Ok(());
            }
            previous = event.ledger_sequence;
            let expiry = created
                .checked_add(destination.notification_ttl_ms)
                .ok_or_else(|| anyhow::anyhow!("notification expiry overflow"))?
                .min(destination.expires_at_ms);
            if created < accepted || expiry <= now {
                continue;
            }
            let (scope, scope_id) = match event.scope {
                SourceBudgetScope::Session(id) => (BudgetScope::Session, id),
                SourceBudgetScope::Project(id) => (BudgetScope::Project, id),
            };
            ensure!(
                scope_id != Uuid::nil()
                    && (scope != BudgetScope::Session || scope_id == event.session),
                "budget source scope mismatch"
            );
            let notification = Notification {
                event_id: event.source_event_id,
                source_event_id: event.source_event_id,
                session_id: event.session,
                run_id: Some(event.run),
                incarnation: None,
                kind: NotificationKind::Budget,
                created_at_ms: created,
                expires_at_ms: expiry,
                decision_id: None,
                budget: Some(BudgetDetail {
                    scope,
                    scope_id,
                    revision: event.revision,
                    dimension: event.dimension,
                    threshold: event.threshold,
                    ledger_sequence: event.ledger_sequence,
                }),
            };
            if store.publish(destination_id, notification, now).is_err() {
                store.advance(destination_id, 0, Some(ProducerError::Unavailable))?;
                return Ok(());
            }
        }
        // The accounting cursor is separate from the owner outbox cursor. A crash
        // before this write repeats publication with the same immutable event IDs.
        attempt.after = page.next_after;
        attempt.state.failures = 0;
        attempt.state.next_attempt_ms = 0;
        if count < 64
            && stopped
            && super::recovery::clean_stop(
                &registry::directory(&self.directory, current.session_id),
                current,
            )
        {
            attempt.state.drained_incarnation = Some(current.incarnation);
        }
        access::save(&path, &attempt)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination() -> Destination {
        Destination {
            id: Uuid::new_v4(),
            recipient_grant_id: Uuid::new_v4(),
            recipient_principal_id: Uuid::new_v4(),
            recipient_grant_revision: 7,
            source_vessel_id: Uuid::new_v4(),
            source_session_id: Uuid::new_v4(),
            event_kinds: vec![NotificationKind::Attention],
            expires_at_ms: 1000,
            notification_ttl_ms: 500,
            quiet_hours_utc: None,
        }
    }

    #[test]
    fn inbox_identity_requires_exact_recipient_not_owner_or_principal_alone() {
        let destination = destination();
        let host = destination.source_vessel_id;
        let actor = GrantBinding {
            grant_id: destination.recipient_grant_id,
            principal_id: destination.recipient_principal_id,
            revision: 7,
        };
        assert!(Supervisor::notification_recipient(
            &destination,
            &Some(actor.clone()),
            host
        ));
        assert!(!Supervisor::notification_recipient(
            &destination,
            &None,
            host
        ));
        for wrong in [
            GrantBinding {
                grant_id: Uuid::new_v4(),
                ..actor.clone()
            },
            GrantBinding {
                principal_id: Uuid::new_v4(),
                ..actor.clone()
            },
            GrantBinding {
                revision: 8,
                ..actor.clone()
            },
        ] {
            assert!(!Supervisor::notification_recipient(
                &destination,
                &Some(wrong),
                host
            ));
        }
        let local = Destination {
            recipient_grant_id: host,
            recipient_principal_id: host,
            recipient_grant_revision: 1,
            ..destination
        };
        assert!(Supervisor::notification_recipient(&local, &None, host));
        assert!(!Supervisor::notification_recipient(
            &local,
            &Some(actor),
            host
        ));
        assert!(!Supervisor::notification_recipient(
            &local,
            &None,
            Uuid::new_v4()
        ));
    }

    #[test]
    fn canonical_budget_envelope_is_typed_and_never_fabricates_incarnation() {
        let source = json!({"schema_version":1,"events":[{
            "source_event_id":Uuid::new_v4(),"ledger_sequence":9,
            "scope":{"session":Uuid::new_v4()},"revision":1,
            "dimension":"estimated_microcurrency","threshold":"warning",
            "created_at":"2026-09-11T00:00:00Z","session":Uuid::new_v4(),"run":Uuid::new_v4()
        }],"next_after":9,"limit":64});
        let page: SourceBudgetPage = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(
            page.events[0].dimension,
            BudgetDimension::EstimatedMicrocurrency
        );
        let mut forged = source;
        forged["events"][0]["threshold"] = json!("approved");
        assert!(serde_json::from_value::<SourceBudgetPage>(forged).is_err());
    }

    #[test]
    fn transport_does_not_offer_a_notification_decision_or_arbitrary_payload() {
        for operation in ["approve", "respond", "publish", "execute", "wake"] {
            assert!(
                serde_json::from_value::<NotificationOperation>(json!({"operation":operation}))
                    .is_err()
            );
        }
        assert!(
            serde_json::from_value::<NotificationOperation>(json!({
                "operation":"open","destination_id":Uuid::new_v4(),"event_id":Uuid::new_v4(),
                "response":"approved","credentials":"CANARY"
            }))
            .is_err()
        );
    }
}
