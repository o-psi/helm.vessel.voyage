//! Bounded public entities read under the canonical owner's journal mutex.
use super::*;
use voyage_protocol::event_connection::{EntityKind, Fence, InitializationEvent, MAX_ENTITY_BYTES};
pub(in crate::attachment::runtime) struct CapturedBaseline {
    expires: std::time::Instant,
    incarnation: Uuid,
    revision: u64,
    cursor: u64,
    events: Vec<InitializationEvent>,
}
const MAX_BASELINES: usize = 4;
const INITIAL_MESSAGES: usize = 96;

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
            let mut store = shared.lock().map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.entity_initializations.retain(|_,baseline|baseline.expires>std::time::Instant::now());
            if offset>0 {
                let baseline=store.entity_initializations.get(&generation).context("initialization expired; discard staged generation")?;
                anyhow::ensure!(baseline.incarnation==incarnation && expected_revision==Some(baseline.revision) && expected_cursor==Some(baseline.cursor),"initialization fence mismatch");
                let page=baseline.page(offset,limit)?;
                if page["has_more"]==false { store.entity_initializations.remove(&generation); }
                return Ok(page);
            }
            anyhow::ensure!(store.entity_initializations.len()<MAX_BASELINES || store.entity_initializations.contains_key(&generation),"initialization capacity exhausted");
            let saved = store.journal.load_session(store.session_id)?;
            let cursor = store.journal.observation_cursor(store.session_id)?;

            let session = &saved.session;
            let pending_cleanup = store.journal.list_session_summaries(None,100)?.sessions.into_iter().find(|summary|summary.id==store.session_id).and_then(|summary|summary.pending_cleanup_run);
            let fence = Fence { generation, session_id: store.session_id, incarnation };
            let message_offset=session.messages.len().saturating_sub(INITIAL_MESSAGES);
            let total = 11_usize.checked_add(session.messages.len()-message_offset).context("entity count overflow")?;
            let offset = usize::try_from(offset)?;
            anyhow::ensure!(offset <= total, "entity offset beyond scope");
            let end = total;
            let mut events = Vec::new();
            if offset == 0 { events.push(InitializationEvent::Begin { fence: fence.clone(), cursor }); }
            for index in offset..end {
                let (entity_kind, entity_id, value) = match index {
                    0 => (EntityKind::Session, "session".to_string(), json!({"session_id":session.id,"revision":saved.revision,"created_at":session.created_at,"name":session.name,"model":session.model,"workspace":session.workspace,"total_messages":session.messages.len(),"message_offset":message_offset,"history_truncated":message_offset>0,"pending_cleanup_run":pending_cleanup})),
                    1 => (EntityKind::Lifecycle, "lifecycle".to_string(), store.journal.lifecycle_status(store.session_id)?),
                    2 => (EntityKind::Goal, "goal".to_string(), serde_json::to_value(store.journal.goal(store.session_id)?)?),
                    3 => (EntityKind::Resource, "retained_cleanup".to_string(), store.journal.retained_cleanup(store.session_id)?),
                    4 => { let run = store.journal.process_latest_run(store.session_id)?; (EntityKind::Run, "run".to_string(), run.map(|run| projection::run(session, &run)).unwrap_or(Value::Null)) },
                    5 => (EntityKind::Resource, "cleanup".to_string(), store.journal.cleanup_progress(store.session_id)?),
                    6 => (EntityKind::Resource, "session_resources".to_string(), store.journal.session_resources(store.session_id)?),
                    7 => (EntityKind::Usage, "usage".to_string(), serde_json::to_value(store.journal.delegated_usage(session.id)?)?),
                    8 => (EntityKind::Settings, "settings".to_string(), settings.clone()),
                    9 => (EntityKind::Run, "turns".to_string(), json!(projection::turns(session))),
                    10 => { let retained=store.journal.retained_cleanup(store.session_id)?; let uncertain=retained["run_ids"].as_array().is_some_and(|values|!values.is_empty()) || retained["resources"].as_array().is_some_and(|values|!values.is_empty()); (EntityKind::Resource,"recovery_notice".to_string(),json!(uncertain.then_some("Conversation ready to continue. Previous interrupted work has unknown effects; its unfinished commands were not repeated."))) },
                    _ => { let message = message_offset + index - 11; (EntityKind::Message, format!("message:{message}"), initial_message(&session.messages, message)?) }
                };
                let serialized=serde_json::to_string(&value)?;
                if serialized.len()<=MAX_ENTITY_BYTES {
                    let sequence=(events.len()-1) as u64;
                    events.push(InitializationEvent::Entity{fence:fence.clone(),sequence,entity_kind,entity_id,value});
                } else {
                    let mut byte_offset=0;
                    while byte_offset<serialized.len() {
                        let mut end=(byte_offset+8192).min(serialized.len());
                        while !serialized.is_char_boundary(end){end-=1;}
                        let chunk=json!({"encoding":"entity_json_utf8","entity_kind":entity_kind,"entity_id":entity_id,"offset":byte_offset,"total_bytes":serialized.len(),"text":&serialized[byte_offset..end]});
                        let sequence=(events.len()-1) as u64;
                        events.push(InitializationEvent::Entity{fence:fence.clone(),sequence,entity_kind:EntityKind::Artifact,entity_id:format!("entitychunk:{index}:{byte_offset}"),value:chunk});
                        byte_offset=end;
                    }
                }
            }
            if end == total { events.push(InitializationEvent::Complete { fence, sequence: (events.len()-1) as u64, cursor }); }
            let baseline=CapturedBaseline{expires:std::time::Instant::now()+std::time::Duration::from_secs(60),incarnation,revision:saved.revision,cursor,events};
            let page=baseline.page(0,limit)?;
            if page["has_more"]==true { store.entity_initializations.insert(generation,baseline); }
            Ok(page)
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

impl CapturedBaseline {
    fn page(&self, offset: u64, limit: u32) -> anyhow::Result<Value> {
        let total = self.events.len() - 2;
        let offset = usize::try_from(offset)?;
        anyhow::ensure!(offset <= total, "entity offset beyond captured scope");
        let end = offset.saturating_add(limit as usize).min(total);
        let mut events = Vec::new();
        if offset == 0 {
            events.push(self.events[0].clone());
        }
        events.extend_from_slice(&self.events[1 + offset..1 + end]);
        if end == total {
            events.push(
                self.events
                    .last()
                    .context("missing captured barrier")?
                    .clone(),
            );
        }
        Ok(
            json!({"version":3,"revision":self.revision,"cursor":self.cursor,"events":events,"next_offset":end,"has_more":end<total}),
        )
    }
}
