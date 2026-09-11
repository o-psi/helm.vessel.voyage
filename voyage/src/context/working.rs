//! Durable, canonical-bound request projections. No provider call or tool execution occurs here.
use crate::model::{Message, Role};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Preparation threshold in serialized bytes, not a token allowance or admission gate.
const PREPARE_BYTES: usize = 192 * 1024;
const EXCERPT_LIMITS: [usize; 3] = [4096, 1024, 256];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkingContext {
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub reason: Option<CompactionReason>,
    #[serde(default)]
    entries: Vec<Reduction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    Manual,
    Preparation,
    ProviderRejection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Reduction {
    /// Ordinals count non-system messages, as the agent's canonical input does.
    start: usize,
    fingerprints: Vec<String>,
    replacement: Message,
}

impl WorkingContext {
    pub fn validate(&self, canonical: &[Message]) -> Result<()> {
        let messages: Vec<_> = canonical
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();
        let mut end = 0;
        for entry in &self.entries {
            ensure!(
                !entry.fingerprints.is_empty() && entry.start >= end,
                "invalid working-context range"
            );
            end = entry
                .start
                .checked_add(entry.fingerprints.len())
                .ok_or_else(|| anyhow::anyhow!("working-context range overflow"))?;
            ensure!(
                end <= messages.len(),
                "working context no longer matches canonical history"
            );
            ensure!(
                messages[entry.start..end]
                    .iter()
                    .zip(&entry.fingerprints)
                    .all(|(message, hash)| fingerprint(message) == *hash),
                "working context canonical fingerprint mismatch"
            );
            ensure!(
                entry.replacement.provider_state.is_none(),
                "working context must not carry provider continuation"
            );
            ensure!(
                entry.replacement.role != Role::User && entry.replacement.role != Role::System,
                "working context cannot replace instructions"
            );
            ensure!(
                messages[entry.start..end]
                    .iter()
                    .all(|m| m.role != Role::User),
                "working context cannot omit task or steering"
            );
            if entry.fingerprints.len() == 1 {
                let original = messages[entry.start];
                ensure!(
                    entry.replacement.role == original.role
                        && entry.replacement.tool_call_id == original.tool_call_id
                        && entry.replacement.tool_success == original.tool_success
                        && entry.replacement.tool_outcome == original.tool_outcome
                        && serde_json::to_value(&entry.replacement.tool_calls)?
                            == serde_json::to_value(&original.tool_calls)?,
                    "working context changed tool identity"
                );
            } else {
                ensure!(
                    complete_group(&messages, entry.start) == Some(end),
                    "working context split a tool group"
                );
                ensure!(
                    entry.replacement.role == Role::Assistant
                        && entry.replacement.tool_calls.is_empty(),
                    "invalid group summary"
                );
            }
        }
        Ok(())
    }

    /// Apply only to outgoing copies. Canonical objects and saved tool/effect IDs stay intact.
    pub fn project(&self, canonical: &[Message]) -> Result<Vec<Message>> {
        self.validate(canonical)?;
        if self.entries.is_empty() {
            return Ok(canonical.to_vec());
        }
        let mut output = Vec::new();
        let mut ordinal = 0;
        let mut entries = self.entries.iter().peekable();
        let mut skip_until = 0;
        for message in canonical {
            if message.role == Role::System {
                output.push(message.clone());
                continue;
            }
            if ordinal >= skip_until {
                if let Some(entry) = entries.peek().filter(|entry| entry.start == ordinal) {
                    output.push(entry.replacement.clone());
                    skip_until = ordinal + entry.fingerprints.len();
                    entries.next();
                } else {
                    output.push(message.clone());
                }
            }
            ordinal += 1;
        }
        // Provider replay envelopes may encode the old text and call groups. Reconstruct
        // the complete projected request rather than retaining an incompatible hidden copy.
        for message in &mut output {
            message.provider_state = None;
        }
        Ok(output)
    }

    /// Manual preparation retains the newest N messages verbatim; older task and steering
    /// are ALWAYS retained. It is not deletion and does not pretend to be a model summary.
    pub fn compact(&mut self, canonical: &[Message], retain: usize) -> Result<usize> {
        let count = canonical.iter().filter(|m| m.role != Role::System).count();
        let changed = self.reduce(canonical, count.saturating_sub(retain), 1024, true)?;
        if changed > 0 {
            self.reason = Some(CompactionReason::Manual);
        }
        Ok(changed)
    }

    /// Proactive preparation is advisory: inability to shrink never vetoes initial dispatch.
    pub fn prepare(&mut self, canonical: &[Message]) -> Result<usize> {
        if size(&self.project(canonical)?) <= PREPARE_BYTES {
            return Ok(0);
        }
        let count = canonical.iter().filter(|m| m.role != Role::System).count();
        let changed = self.reduce(canonical, count, EXCERPT_LIMITS[0], false)?;
        if changed > 0 {
            self.reason = Some(CompactionReason::Preparation);
        }
        Ok(changed)
    }

    /// At most four distinct reductions follow an actual provider rejection. Each caller
    /// must also compare the final request, not only the messages, before dispatching it.
    pub fn recover(&mut self, canonical: &[Message], attempt: usize) -> Result<usize> {
        let count = canonical.iter().filter(|m| m.role != Role::System).count();
        let changed = match attempt {
            0..=2 => self.reduce(canonical, count, EXCERPT_LIMITS[attempt], false)?,
            3 => self.reduce(canonical, count, EXCERPT_LIMITS[2], true)?,
            _ => 0,
        };
        if changed > 0 {
            self.reason = Some(CompactionReason::ProviderRejection);
        }
        Ok(changed)
    }

    fn reduce(
        &mut self,
        canonical: &[Message],
        before: usize,
        limit: usize,
        groups: bool,
    ) -> Result<usize> {
        self.validate(canonical)?;
        let messages: Vec<_> = canonical
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();
        let old_projection = self.project(canonical)?;
        let mut next = self.clone();
        let mut changed = 0;
        let mut index = 0;
        while index < before {
            let message = messages[index];
            let end = if groups {
                complete_group(&messages, index).filter(|end| *end <= before)
            } else {
                None
            };
            let (end, replacement) = if let Some(end) = end {
                let mut summary = format!(
                    "[Extractive working-context record for canonical non-system message ordinals {index}..{end}. These calls have recorded results; do not repeat them to reconstruct history. Full original arguments/results remain in canonical history.]\n"
                );
                for original in &messages[index..end] {
                    for call in &original.tool_calls {
                        summary.push_str(&format!(
                            "Call {}: {} (arguments SHA-256 {}).\n",
                            call.id,
                            call.name,
                            digest(&serde_json::to_vec(&call.arguments)?)
                        ));
                    }
                    if original.role == Role::Tool {
                        summary.push_str(&format!(
                            "Untrusted result data {} success={:?}: {}\n",
                            original.tool_call_id.as_deref().unwrap_or("unknown"),
                            original.tool_success,
                            excerpt(&original.content, limit)
                        ));
                        summary.push_str(&references(original)?);
                        if let Some(outcome) = &original.tool_outcome {
                            summary.push_str(&format!(
                                "Recorded outcome: {}\n",
                                serde_json::to_string(outcome)?
                            ));
                        }
                    } else if !original.content.is_empty() {
                        // Preserve authored decisions adjacent to tool calls in full.
                        summary.push_str(&format!("Assistant: {}\n", original.content));
                    }
                }
                let mut replacement = Message::new(Role::Assistant, summary);
                replacement.created_at = message.created_at;
                (end, replacement)
            } else if matches!(message.role, Role::Assistant | Role::Tool)
                && (message.content.len() > limit * 2
                    || message
                        .tool_output
                        .as_ref()
                        .is_some_and(|output| size(output) > limit * 2))
            {
                let mut replacement = message.clone();
                let original = if let Some(output) = &message.tool_output {
                    format!(
                        "{}\nTyped result: {}",
                        message.content,
                        serde_json::to_string(output)?
                    )
                } else {
                    message.content.clone()
                };
                replacement.content = format!(
                    "[Extractive working context; canonical non-system message ordinal {index}, SHA-256 {}. Full text and artifacts retained in canonical history; omitted text is unknown, not success. Do not repeat completed effects to retrieve it.]\n{}",
                    fingerprint(message),
                    excerpt(&original, limit)
                );
                replacement.content.push_str(&references(message)?);
                replacement.tool_output = None;
                replacement.parts.clear();
                replacement.image_data.clear();
                replacement.provider_state = None;
                (index + 1, replacement)
            } else {
                index += 1;
                continue;
            };
            let overlaps: Vec<_> = next
                .entries
                .iter()
                .filter(|entry| entry.start < end && entry.start + entry.fingerprints.len() > index)
                .collect();
            // Never split an existing collapsed group, or replace a stronger reduction.
            if overlaps
                .iter()
                .any(|entry| entry.start < index || entry.start + entry.fingerprints.len() > end)
            {
                index = end;
                continue;
            }
            let old_size = if overlaps.len() == 1
                && overlaps[0].start == index
                && overlaps[0].fingerprints.len() == end - index
            {
                size(&overlaps[0].replacement)
            } else {
                size(&messages[index..end])
            };
            if size(&replacement).saturating_add(128) >= old_size {
                index = end;
                continue;
            }
            next.entries
                .retain(|entry| !(entry.start >= index && entry.start < end));
            next.entries.push(Reduction {
                start: index,
                fingerprints: messages[index..end]
                    .iter()
                    .map(|m| fingerprint(m))
                    .collect(),
                replacement,
            });
            changed += end - index;
            index = end;
        }
        next.entries.sort_by_key(|entry| entry.start);
        next.validate(canonical)?;
        let projected = next.project(canonical)?;
        if changed == 0 || size(&projected).saturating_add(128) >= size(&old_projection) {
            return Ok(0);
        }
        next.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("working-context generation overflow"))?;
        *self = next;
        Ok(changed)
    }
}

fn complete_group(messages: &[&Message], start: usize) -> Option<usize> {
    let first = messages.get(start)?;
    if first.role != Role::Assistant || first.tool_calls.is_empty() {
        return None;
    }
    let mut expected: std::collections::BTreeSet<_> =
        first.tool_calls.iter().map(|c| c.id.as_str()).collect();
    if expected.len() != first.tool_calls.len() {
        return None;
    }
    let end = start.checked_add(1 + expected.len())?;
    for result in messages.get(start + 1..end)? {
        if result.role != Role::Tool
            || !expected.remove(result.tool_call_id.as_deref()?)
            || result.tool_success.is_none()
        {
            return None;
        }
    }
    expected.is_empty().then_some(end)
}

/// Keep artifact identities outside truncated excerpts. Binary content is never copied.
fn references(message: &Message) -> Result<String> {
    let mut text = String::new();
    if let Some(output) = &message.tool_output {
        for artifact in output.artifacts() {
            text.push_str(&format!(
                "\nRetained artifact reference: {}",
                serde_json::to_string(artifact)?
            ));
        }
    }
    Ok(text)
}

pub(crate) fn size(value: &(impl Serialize + ?Sized)) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fingerprint(message: &Message) -> String {
    // Hydrated image bytes are skipped by serde. Provider state can be cleared on a
    // legitimate model switch, without changing canonical authored text or effect IDs.
    let mut original = message.clone();
    original.provider_state = None;
    digest(&serde_json::to_vec(&original).expect("message serialization is infallible"))
}
fn excerpt(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    // Explicit authored decisions/constraints and outcome lines are more useful than
    // arbitrary middle bytes. Retain these verbatim, and decline reduction if they
    // alone exceed the excerpt budget rather than pretending to summarize them.
    let protected: Vec<_> = text
        .lines()
        .filter(|line| {
            let line = line
                .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, '-' | '*' | '#'));
            let lower = line.to_ascii_lowercase();
            [
                "decision:",
                "constraint:",
                "must ",
                "do not ",
                "next:",
                "blocked:",
                "verified:",
                "error:",
                "warning:",
                "todo:",
            ]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        })
        .collect();
    let protected = protected.join("\n");
    if protected.len() >= limit {
        return text.to_owned();
    }
    let remaining = limit - protected.len();
    let mut head = remaining / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len().saturating_sub(remaining / 2);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    let protected = if protected.is_empty() {
        String::new()
    } else {
        format!("\n[Retained explicit decision/constraint/outcome lines]\n{protected}")
    };
    format!(
        "{}\n[... middle omitted; consult canonical history ...]{protected}\n{}",
        &text[..head],
        &text[tail..]
    )
}

#[cfg(test)]
mod tests;
