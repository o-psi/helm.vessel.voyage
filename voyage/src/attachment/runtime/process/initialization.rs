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
        settings: Value,
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
            let pending_cleanup = store.journal.list_session_summaries(None,100)?.sessions.into_iter().find(|summary|summary.id==store.session_id).and_then(|summary|summary.pending_cleanup_run);
            let fence = Fence { generation, session_id: store.session_id, incarnation };
            let total = 9_usize.checked_add(session.messages.len()).context("entity count overflow")?;
            let offset = usize::try_from(offset)?;
            anyhow::ensure!(offset <= total, "entity offset beyond scope");
            let end = offset.saturating_add(limit as usize).min(total);
            let mut events = Vec::new();
            if offset == 0 { events.push(InitializationEvent::Begin { fence: fence.clone(), cursor }); }
            for index in offset..end {
                let (entity_kind, entity_id, value) = match index {
                    0 => (EntityKind::Session, "session".to_string(), json!({"session_id":session.id,"revision":saved.revision,"created_at":session.created_at,"name":session.name,"model":session.model,"workspace":session.workspace,"total_messages":session.messages.len(),"message_offset":0,"history_truncated":false,"pending_cleanup_run":pending_cleanup})),
                    1 => (EntityKind::Lifecycle, "lifecycle".to_string(), store.journal.lifecycle_status(store.session_id)?),
                    2 => (EntityKind::Goal, "goal".to_string(), serde_json::to_value(store.journal.goal(store.session_id)?)?),
                    3 => (EntityKind::Resource, "retained_cleanup".to_string(), store.journal.retained_cleanup(store.session_id)?),
                    4 => { let run = store.journal.process_latest_run(store.session_id)?; (EntityKind::Run, "run".to_string(), run.map(|run| projection::run(session, &run)).unwrap_or(Value::Null)) },
                    5 => (EntityKind::Resource, "cleanup".to_string(), store.journal.cleanup_progress(store.session_id)?),
                    6 => (EntityKind::Resource, "session_resources".to_string(), store.journal.session_resources(store.session_id)?),
                    7 => (EntityKind::Usage, "usage".to_string(), serde_json::to_value(store.journal.delegated_usage(session.id)?)?),
                    8 => (EntityKind::Settings, "settings".to_string(), settings.clone()),
                    _ => { let message = index - 9; (EntityKind::Message, format!("message:{message}"), initial_message(&session.messages, message)?) }
                };
                anyhow::ensure!(serde_json::to_vec(&value)?.len() <= MAX_ENTITY_BYTES, "entity requires bounded content chunks");
                events.push(InitializationEvent::Entity { fence: fence.clone(), sequence: index as u64, entity_kind, entity_id, value });
            }
            if end == total { events.push(InitializationEvent::Complete { fence, sequence: total as u64, cursor }); }
            Ok(json!({"version":3,"revision":saved.revision,"cursor":cursor,"events":events,"next_offset":end,"has_more":end<total}))
        }).await?
    }
}

fn initial_message(messages: &[crate::model::Message], index: usize) -> anyhow::Result<Value> {
    let mut value = projection::page(messages, index, 1)?
        .into_iter()
        .next()
        .context("missing message projection")?;
    if serde_json::to_vec(&value)?.len() > MAX_ENTITY_BYTES {
        let message = messages.get(index).context("missing message")?;
        value = json!({"message_index":index,"role":message.role,"content":"","projection_truncated":true,"complete_message":"message_chunk","content_bytes":message.content.len(),"tool_calls":[],"tool_calls_omitted":!message.tool_calls.is_empty()});
    }
    Ok(value)
}
