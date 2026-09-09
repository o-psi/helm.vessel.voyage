//! Runtime facts accompany, but never replace, canonical tool text.
use super::ToolError;
use voyage_protocol::tool_result::{
    CommandOutcome, ExecutionOutcome, IncompleteReason, ToolOutcome, ToolOutput,
};

pub struct ToolReport {
    pub output: ToolOutput,
    pub outcome: ToolOutcome,
}
impl ToolReport {
    pub fn output(output: ToolOutput) -> Self {
        let outcome = ToolOutcome {
            execution: if output.is_error {
                ExecutionOutcome::ExecutionError
            } else {
                ExecutionOutcome::Succeeded
            },
            ..Default::default()
        };
        Self { output, outcome }
    }
    pub fn text(text: impl Into<String>) -> Self {
        Self::output(ToolOutput::text(text))
    }
    pub fn command(text: String, code: Option<i64>, incomplete: bool) -> Self {
        let mut report = Self::text(text);
        report.outcome.command =
            Some(
                code.map_or(CommandOutcome::Signalled, |code| CommandOutcome::Exited {
                    code,
                }),
            );
        if incomplete {
            report.outcome.incomplete = Some(IncompleteReason::CaptureLimit);
        }
        report.synchronize();
        report
    }
    pub fn error(error: ToolError) -> Self {
        let execution = match &error {
            ToolError::Denied(_) => ExecutionOutcome::PolicyRefused,
            ToolError::Cancelled => ExecutionOutcome::Cancelled,
            // A timeout does not prove effects stopped or were rolled back.
            ToolError::Timeout(_) => ExecutionOutcome::Unknown,
            _ => ExecutionOutcome::ExecutionError,
        };
        let mut report = Self::text(error.to_string());
        report.outcome.execution = execution;
        report.synchronize();
        report
    }
    pub fn limit(&mut self, reason: IncompleteReason, text: &str) {
        self.output = ToolOutput::text(text);
        self.outcome.incomplete = Some(reason);
        self.synchronize();
    }
    pub fn synchronize(&mut self) {
        self.output.is_error = !self.outcome.success();
    }
}
