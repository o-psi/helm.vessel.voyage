//! Bounded public entities read under the canonical owner's journal mutex.
use super::*;
use voyage_protocol::event_connection::{EntityKind, Fence, InitializationEvent, MAX_ENTITY_BYTES};
impl ManagedSessionOwner {
    pub(crate) async fn initialize_entities(
        &self,
        generation: Uuid,
        incarnation: Uuid,
        offset: u64,
        limit: u32,
        expected_revision: Option<u64>,
        expected_cursor: Option<u64>,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!((1..=64).contains(&limit), "entity page limit must be 1..64");
        anyhow::ensure!(!generation.is_nil(), "initialization generation required");
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared.lock().map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let saved = store.journal.load_session(store.session_id)?;
            let cursor = store.journal.observation_cursor(store.session_id)?;
            if offset > 0 { anyhow::ensure!(expected_revision == Some(saved.revision) && expected_cursor == Some(cursor), "initialization fence changed; discard staged generation"); }
            let session = &saved.session;
            let fence = Fence { generation, session_id: store.session_id, incarnation };
            let total = 4_usize.checked_add(session.messages.len()).context("entity count overflow")?;
            let offset = usize::try_from(offset)?;
            anyhow::ensure!(offset <= total, "entity offset beyond scope");
            let end = offset.saturating_add(limit as usize).min(total);
            let mut events = Vec::new();
            if offset == 0 { events.push(InitializationEvent::Begin { fence: fence.clone(), cursor }); }
            for index in offset..end {
                let (entity_kind, entity_id, value) = match index {
                    0 => (EntityKind::Session, "session".to_string(), json!({"session_id":session.id,"revision":saved.revision,"created_at":session.created_at,"name":session.name,"model":session.model,"workspace":session.workspace,"total_messages":session.messages.len()})),
                    1 => (EntityKind::Lifecycle, "lifecycle".to_string(), store.journal.lifecycle_status(store.session_id)?),
                    2 => (EntityKind::Goal, "goal".to_string(), serde_json::to_value(store.journal.goal(store.session_id)?)?),
                    3 => (EntityKind::Resource, "retained_cleanup".to_string(), store.journal.retained_cleanup(store.session_id)?),
                    _ => { let message = index - 4; (EntityKind::Message, format!("message:{message}"), projection::page(&session.messages, message, 1)?.into_iter().next().context("missing message projection")?) }
                };
                anyhow::ensure!(serde_json::to_vec(&value)?.len() <= MAX_ENTITY_BYTES, "entity requires bounded content chunks");
                events.push(InitializationEvent::Entity { fence: fence.clone(), sequence: index as u64, entity_kind, entity_id, value });
            }
            if end == total { events.push(InitializationEvent::Complete { fence, sequence: total as u64, cursor }); }
            Ok(json!({"version":3,"revision":saved.revision,"cursor":cursor,"events":events,"next_offset":end,"has_more":end<total}))
        }).await?
    }
}
