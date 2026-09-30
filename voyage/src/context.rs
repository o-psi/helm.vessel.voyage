//! Optional operator-requested budgeting. Reduction affects only a request projection.
use crate::model::ModelRequest;

mod policy;
mod working;
pub use policy::{ContextPolicy, check_operator_limit};
pub use working::WorkingContext;

/// Zero disables local token admission checks. Providers enforce their own capacity.
pub const DEFAULT_CONTEXT_WINDOW: usize = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextError {
    pub input_tokens: u64,
    pub required_tokens: Option<u64>,
    pub limit: u64,
}
impl std::fmt::Display for ContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.required_tokens {
            Some(required) => write!(
                f,
                "Prepared input and response headroom need {required} tokens, above explicit context_window {} (provider-counted input {}). Canonical history is retained; increase the local cap or reduce response headroom.",
                self.limit, self.input_tokens
            ),
            None => write!(
                f,
                "Provider-counted input uses {} tokens, above explicit context_window {}. Response headroom is unknown. Canonical history is retained; increase the local cap or narrow the task.",
                self.input_tokens, self.limit
            ),
        }
    }
}
impl std::error::Error for ContextError {}

/// Payload/resource evidence only. It is not a token estimate or pressure signal.
pub fn payload_bytes(request: &ModelRequest) -> usize {
    serde_json::to_vec(request).map_or(usize::MAX, |bytes| bytes.len())
}
