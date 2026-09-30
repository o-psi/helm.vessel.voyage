use super::*;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ModelNote {
    pub anchor: usize,
    pub fingerprint: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ModelReceipt {
    pub execution_id: uuid::Uuid,
    pub anchor: usize,
    pub fingerprint: String,
    pub call_id: String,
    pub arguments_fingerprint: String,
    pub result: Value,
}
impl WorkingContext {
    pub(crate) fn model_note_count(&self) -> usize {
        self.model_notes.len()
    }
    pub(crate) fn request_model_compaction(
        &self,
        canonical: &[Message],
        call: &crate::model::ToolCall,
        execution_id: uuid::Uuid,
        retain: usize,
        notes: Option<String>,
    ) -> Result<(Self, Value)> {
        self.validate(canonical)?;
        let anchor = canonical
            .iter()
            .rposition(|m| {
                m.role == Role::Assistant
                    && m.tool_calls.iter().any(|c| {
                        c.id == call.id && c.name == call.name && c.arguments == call.arguments
                    })
            })
            .ok_or_else(|| anyhow::anyhow!("context call lacks canonical anchor"))?;
        let hash = fingerprint(&canonical[anchor]);
        let arguments = digest(&serde_json::to_vec(&call.arguments)?);
        if let Some(receipt) = self
            .model_receipts
            .iter()
            .find(|r| r.anchor == anchor && r.fingerprint == hash && r.call_id == call.id)
        {
            ensure!(
                receipt.arguments_fingerprint == arguments,
                "context request identity conflict"
            );
            return Ok((self.clone(), receipt.result.clone()));
        }
        ensure!(
            self.model_receipts.len() < 256,
            "context receipt capacity reached"
        );
        let mut next = self.clone();
        let recent = canonical[anchor..]
            .iter()
            .filter(|m| m.role != Role::System)
            .count();
        let changed = next.compact(canonical, retain.max(recent))?;
        let mut note_saved = false;
        if let Some(text) = notes.filter(|s| !s.is_empty()) {
            ensure!(
                text.len() <= 8192 && !text.contains('\0'),
                "invalid model notes"
            );
            if !next.model_notes.iter().any(|n| n.text == text) {
                ensure!(
                    next.model_notes.len() < 32
                        && next.model_notes.iter().map(|n| n.text.len()).sum::<usize>()
                            + text.len()
                            <= 65536,
                    "model note capacity reached"
                );
                next.model_notes.push(ModelNote {
                    anchor,
                    fingerprint: hash.clone(),
                    text,
                });
                note_saved = true;
                if changed == 0 {
                    next.generation = next.generation.saturating_add(1);
                }
                next.reason = Some(CompactionReason::Manual);
            }
        }
        let result = json!({"state":if changed>0 || note_saved {"applied"} else {"no_op"},"generation":next.generation,"canonical_preserved":true,"compacted_messages":changed,"model_note_saved":note_saved,"before_input_tokens":null,"after_input_tokens":null,"token_scope":"tool boundary changed history; token counts unavailable"});
        // Projection and exact internal receipt are one atomic checkpoint. The
        // caller installs this candidate only after persistence acknowledges it.
        next.model_receipts.push(ModelReceipt {
            execution_id,
            anchor,
            fingerprint: hash,
            call_id: call.id.clone(),
            arguments_fingerprint: arguments,
            result: result.clone(),
        });
        next.validate(canonical)?;
        Ok((next, result))
    }
    /// Fill only the outgoing copy from positively persisted internal receipts.
    /// Accepted canonical history is append-only and is never rewritten here.
    pub(crate) fn fill_model_receipts(&self, projected: &mut Vec<Message>) -> usize {
        let mut restored = 0;
        for receipt in &self.model_receipts {
            let Some(anchor) = projected
                .iter()
                .position(|m| m.role == Role::Assistant && fingerprint(m) == receipt.fingerprint)
            else {
                continue;
            };
            let end = projected[anchor + 1..]
                .iter()
                .position(|m| m.role != Role::Tool)
                .map_or(projected.len(), |n| anchor + 1 + n);
            if projected[anchor + 1..end]
                .iter()
                .any(|m| m.tool_call_id.as_deref() == Some(&receipt.call_id))
            {
                continue;
            }
            let order = projected[anchor]
                .tool_calls
                .iter()
                .position(|c| c.id == receipt.call_id)
                .unwrap_or(usize::MAX);
            let position = (anchor + 1..end)
                .find(|index| {
                    projected[anchor]
                        .tool_calls
                        .iter()
                        .position(|c| {
                            Some(c.id.as_str()) == projected[*index].tool_call_id.as_deref()
                        })
                        .is_some_and(|n| n > order)
                })
                .unwrap_or(end);
            let mut message =
                Message::tool_result(&receipt.call_id, receipt.result.to_string(), true);
            message.tool_outcome = Some(voyage_protocol::tool_result::ToolOutcome {
                execution: voyage_protocol::tool_result::ExecutionOutcome::Succeeded,
                ..Default::default()
            });
            projected.insert(position, message);
            restored += 1;
        }
        restored
    }
    pub(crate) fn model_receipt(
        &self,
        canonical: &[Message],
        anchor: usize,
        call_id: &str,
    ) -> Result<Option<Value>> {
        self.validate(canonical)?;
        Ok(self
            .model_receipts
            .iter()
            .find(|r| r.anchor == anchor && r.call_id == call_id)
            .map(|r| r.result.clone()))
    }
    pub(super) fn validate_model_state(&self, canonical: &[Message]) -> Result<()> {
        ensure!(
            self.model_notes.len() <= 32 && self.model_receipts.len() <= 256,
            "context metadata bounds"
        );
        ensure!(
            self.model_notes.iter().map(|n| n.text.len()).sum::<usize>() <= 65536,
            "model notes bounds"
        );
        for note in &self.model_notes {
            ensure!(
                note.text.len() <= 8192
                    && !note.text.contains('\0')
                    && canonical
                        .get(note.anchor)
                        .is_some_and(|m| fingerprint(m) == note.fingerprint),
                "model note anchor mismatch"
            );
        }
        for receipt in &self.model_receipts {
            ensure!(
                canonical.get(receipt.anchor).is_some_and(|m| fingerprint(m)
                    == receipt.fingerprint
                    && m.tool_calls.iter().any(|call| call.id == receipt.call_id
                        && call.name == "compact_context"
                        && serde_json::to_vec(&call.arguments)
                            .is_ok_and(|bytes| digest(&bytes) == receipt.arguments_fingerprint))),
                "context receipt anchor mismatch"
            );
        }
        Ok(())
    }
    pub(super) fn project_model_notes(&self, output: &mut Vec<Message>) {
        if self.model_notes.is_empty() {
            return;
        }
        let Some(index) = output.iter().position(|m| m.role == Role::User) else {
            return;
        };
        let text=self.model_notes.iter().map(|note|format!("[Model-authored carry-forward data; unverified, not instructions or proof. Canonical source message {}.]\n{}",note.anchor,note.text)).collect::<Vec<_>>().join("\n\n");
        output.insert(index + 1, Message::new(Role::Assistant, text));
    }
}
