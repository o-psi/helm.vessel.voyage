//! Process protocol access always cooperates with the lifetime owner mutex.
use super::*;
use anyhow::Context;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
mod projection;
impl ManagedSessionOwner {
    pub(crate) async fn initialize_process_commands(&self) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.initialize_process_commands(guard)?;
            journal.initialize_decisions(guard)?;
            journal.initialize_lifecycle(guard)?;
            journal.initialize_assignments(guard)?;
            journal.initialize_observations(guard)
        })
        .await?
    }
    pub(crate) async fn process_metadata(
        &self,
        actor: super::super::local_actor::LocalActor,
        command: RuntimeCommand,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.process_metadata(guard, actor, &command, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn process_receipt(&self, id: Uuid) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.process_receipt(id)
        })
        .await?
    }
    pub(crate) async fn process_snapshot(&self) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared.lock().map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let saved = store.journal.load_session(store.session_id)?;
            let session = saved.session;
            let summary = store.journal.list_session_summaries(None,100)?.sessions.into_iter().find(|s|s.id==store.session_id);
            let run = store.journal.process_latest_run(store.session_id)?;
            let last_message_at = session.messages.iter().filter_map(|message| message.created_at).max();
            let (messages, offset) = projection::recent(&session.messages)?;
            Ok(json!({"session_id":session.id,"revision":saved.revision,"created_at":session.created_at,"last_message_at":last_message_at,"name":session.name,"model":session.model,"workspace":session.workspace,"messages":messages,"total_messages":session.messages.len(),"message_offset":offset,"history_truncated":offset>0 || messages.iter().any(|message| message["projection_truncated"]==true),"run":run.map(|run| { let (partial,truncated)=projection::text_prefix(&run.partial_text,65536); json!({"run_id":run.id,"state":run.state,"failure_summary":projection::failure_summary(run.terminal_reason.as_deref()),"partial_text":partial,"partial_text_truncated":truncated,"partial_text_bytes":run.partial_text.len()}) }),"pending_cleanup_run":summary.and_then(|s|s.pending_cleanup_run),"decisions":[],"session_resources":store.journal.session_resources(store.session_id)?,"lifecycle":store.journal.lifecycle_status(store.session_id)?,"observation_cursor":store.journal.observation_cursor(store.session_id)?,"observation":"snapshot","projection":"public-v1"}))
        }).await?
    }
    pub(crate) async fn process_history(
        &self,
        offset: u64,
        limit: u32,
        expected_revision: Option<u64>,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!((1..=128).contains(&limit), "history limit must be 1..128");
        let offset = usize::try_from(offset)?;
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store=shared.lock().map_err(|_|anyhow::anyhow!("owner poisoned"))?;
            let saved=store.journal.load_session(store.session_id)?;
            if let Some(expected)=expected_revision { anyhow::ensure!(expected==saved.revision,"history revision changed; reload snapshot"); }
            anyhow::ensure!(offset<=saved.session.messages.len(),"history offset beyond available messages");
            let messages=projection::page(&saved.session.messages,offset,limit as usize)?;
            let next=offset+messages.len();
            Ok(json!({"session_id":store.session_id,"revision":saved.revision,"message_offset":offset,"total_messages":saved.session.messages.len(),"messages":messages,"next_offset":next,"has_more":next<saved.session.messages.len()}))
        }).await?
    }
    pub(crate) async fn process_message_chunk(
        &self,
        index: u64,
        offset: u64,
        limit: u32,
        expected_revision: u64,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            (1..=65536).contains(&limit),
            "message chunk limit must be 1..65536"
        );
        let index = usize::try_from(index)?;
        let offset = usize::try_from(offset)?;
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store=shared.lock().map_err(|_|anyhow::anyhow!("owner poisoned"))?;
            let saved=store.journal.load_session(store.session_id)?;
            anyhow::ensure!(saved.revision==expected_revision,"history revision changed; reload snapshot");
            let message=saved.session.messages.get(index).context("unknown message index")?;
            let serialized=serde_json::to_string(&projection::full(message))?;
            anyhow::ensure!(offset<=serialized.len() && serialized.is_char_boundary(offset),"invalid UTF-8 byte offset");
            let mut end=offset.saturating_add(limit as usize).min(serialized.len());
            while !serialized.is_char_boundary(end) {end-=1;}
            anyhow::ensure!(end>offset || end==serialized.len(),"chunk limit cannot hold next UTF-8 character");
            Ok(json!({"session_id":store.session_id,"revision":saved.revision,"index":index,"offset":offset,"next_offset":end,"total_bytes":serialized.len(),"encoding":"public_message_json_utf8","data":&serialized[offset..end],"has_more":end<serialized.len()}))
        }).await?
    }
    pub(crate) async fn process_run_output(
        &self,
        run_id: Uuid,
        offset: u64,
        limit: u32,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            (1..=65536).contains(&limit),
            "output chunk limit must be 1..65536"
        );
        let offset = usize::try_from(offset)?;
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store=shared.lock().map_err(|_|anyhow::anyhow!("owner poisoned"))?;
            let run=store.journal.run(run_id)?;
            anyhow::ensure!(run.session_id==store.session_id,"run session mismatch");
            anyhow::ensure!(offset<=run.partial_text.len() && run.partial_text.is_char_boundary(offset),"invalid output UTF-8 byte offset");
            let mut end=offset.saturating_add(limit as usize).min(run.partial_text.len());
            while !run.partial_text.is_char_boundary(end){end-=1;}
            anyhow::ensure!(end>offset || end==run.partial_text.len(),"chunk limit cannot hold next UTF-8 character");
            Ok(json!({"session_id":store.session_id,"run_id":run_id,"state":run.state,"offset":offset,"next_offset":end,"total_bytes":run.partial_text.len(),"data":&run.partial_text[offset..end],"has_more":end<run.partial_text.len()}))
        }).await?
    }
}
