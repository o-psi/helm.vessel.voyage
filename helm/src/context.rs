//! Optional operator-requested budgeting. Reduction affects only a request projection.
use crate::model::{ModelRequest, Role};
use thiserror::Error;

/// Zero disables local token admission checks. Providers enforce their own capacity.
pub const DEFAULT_CONTEXT_WINDOW: usize = 0;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error(
    "request context does not fit: estimated {estimated} tokens including output reserve, limit {limit}; reduce the prompt/tool output or configure the model's context_window"
)]
pub struct ContextError {
    pub estimated: usize,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct ContextReport {
    pub estimated: usize,
    pub limit: usize,
    pub omitted_messages: usize,
}

/// One token per serialized UTF-8 byte, plus explicit per-item/framing headroom.
/// Includes replay metadata, tool schemas and the response allowance. This is a
/// conservative estimator, not provider-reported usage or a universal tokenizer.
pub fn estimate(request: &ModelRequest) -> usize {
    serialized_size(request)
        .saturating_add(4096)
        .saturating_add(request.messages.len().saturating_mul(256))
        .saturating_add(request.tools.len().saturating_mul(256))
        .saturating_add(request.max_tokens.unwrap_or(0) as usize)
}

fn serialized_size(value: &impl serde::Serialize) -> usize {
    // Count without allocating a second copy of potentially large tool results.
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    if serde_json::to_writer(&mut counter, value).is_ok() {
        counter.0
    } else {
        usize::MAX
    }
}

/// Preserve system guidance and the latest root user turn, including steering and
/// complete tool groups. Find the smallest removable prefix in one linear scan;
/// repeatedly serializing the remaining history would be quadratic. A zero limit
/// leaves the complete request unchanged; an estimate is not an automatic gate.
pub fn preflight(request: &mut ModelRequest, limit: usize) -> Result<ContextReport, ContextError> {
    let estimated = estimate(request);
    if limit == 0 || estimated <= limit {
        return Ok(ContextReport {
            estimated,
            limit,
            omitted_messages: 0,
        });
    }
    if estimated == usize::MAX {
        return Err(ContextError { estimated, limit });
    }
    let mut omitted = 0;
    let mut removed_cost = 0usize;
    let mut seen_root = false;
    let mut selected = None;
    let mut smallest = estimated;
    for (index, message) in request.messages.iter().enumerate() {
        if message.role == Role::User && message.steering.is_none() {
            if seen_root {
                let marker = crate::model::Message::new(
                    Role::System,
                    format!(
                        "[Context projection: {omitted} older messages omitted; canonical transcript retained locally.]"
                    ),
                );
                // The retained list is nonempty, so each removed/added item also
                // removes/adds exactly one JSON comma and 256 framing tokens.
                let projected = estimated
                    .saturating_sub(removed_cost)
                    .saturating_add(serialized_size(&marker))
                    .saturating_add(257);
                smallest = smallest.min(projected);
                if projected <= limit {
                    selected = Some((index, omitted, marker));
                    break;
                }
            }
            seen_root = true;
        }
        if message.role != Role::System {
            removed_cost = removed_cost
                .saturating_add(serialized_size(message))
                .saturating_add(257);
            omitted += 1;
        }
    }
    let Some((end, omitted, marker)) = selected else {
        return Err(ContextError {
            estimated: smallest,
            limit,
        });
    };
    let mut index = 0;
    request.messages.retain(|message| {
        let keep = index >= end || message.role == Role::System;
        index += 1;
        keep
    });
    request.messages.insert(0, marker);
    let estimated = estimate(request);
    if estimated > limit {
        return Err(ContextError { estimated, limit });
    }
    Ok(ContextReport {
        estimated,
        limit,
        omitted_messages: omitted,
    })
}
