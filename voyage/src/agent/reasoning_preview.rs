//! Separate bounded display accumulator: no executable/canonical provider state.
use crate::{provider::ProviderDelta, tools::Redactor};
use std::collections::BTreeMap;
use voyage_protocol::reasoning_preview::{
    MAX_BLOCKS, MAX_TEXT_BYTES, ReasoningKind, ReasoningPreview,
};

#[derive(Default)]
pub(super) struct Previews(BTreeMap<(usize, ReasoningKind), (String, bool)>);
fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
impl Previews {
    pub(super) fn update(&mut self, delta: &ProviderDelta) {
        let ProviderDelta::Reasoning { index, kind, text } = delta else {
            return;
        };
        if !self.0.contains_key(&(*index, *kind)) && self.0.len() >= MAX_BLOCKS {
            return;
        }
        let (raw, truncated) = self.0.entry((*index, *kind)).or_default();
        if !*truncated {
            let part = prefix(text, MAX_TEXT_BYTES.saturating_sub(raw.len()));
            raw.push_str(part);
            *truncated = part.len() != text.len();
        }
    }
    pub(super) fn public(
        &self,
        attempt_id: uuid::Uuid,
        redactor: &Redactor,
    ) -> Vec<ReasoningPreview> {
        self.0
            .iter()
            .map(|(&(index, kind), (raw, truncated))| {
                // Even an interrupted prefix must not publish a partial secret.
                let end = redactor.stable_prefix(raw, false);
                let text = redactor.redact_public_prefix(&raw[..end]);
                ReasoningPreview {
                    attempt_id,
                    index,
                    kind,
                    truncated: *truncated || text.len() > MAX_TEXT_BYTES,
                    text: prefix(&text, MAX_TEXT_BYTES).into(),
                    finalized: false,
                }
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn delta(index: usize, text: &str) -> ProviderDelta {
        ProviderDelta::Reasoning {
            index,
            kind: ReasoningKind::Summary,
            text: text.into(),
        }
    }
    #[test]
    fn disclosure_is_bounded_interleaved_and_secret_safe_across_chunks() {
        let redactor = Redactor::default().with_additional(["secret-token".into()]);
        let mut p = Previews::default();
        p.update(&delta(2, "hello secret-"));
        assert!(
            !p.public(uuid::Uuid::nil(), &redactor)[0]
                .text
                .contains("secret")
        );
        p.update(&delta(1, "世界"));
        p.update(&delta(2, "token"));
        let out = p.public(uuid::Uuid::nil(), &redactor);
        assert_eq!(out[0].text, "世界");
        assert!(out[1].text.contains("[REDACTED]"));
        assert!(!out[1].finalized);
        p.update(&delta(3, &"世".repeat(MAX_TEXT_BYTES)));
        let out = p.public(uuid::Uuid::nil(), &redactor);
        assert!(out[2].truncated);
        assert!(out[2].text.len() <= MAX_TEXT_BYTES);
        for i in 4..100 {
            p.update(&delta(i, "x"));
        }
        assert_eq!(p.public(uuid::Uuid::nil(), &redactor).len(), MAX_BLOCKS);
    }
}
