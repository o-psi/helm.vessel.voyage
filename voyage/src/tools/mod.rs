mod browser;
pub use browser::BrowserTool;
pub(crate) mod action_schema;
mod filesystem;
pub mod mcp;
pub(crate) mod output;
#[cfg(test)]
pub(crate) mod reliability_tests;
mod report;
pub use report::ToolReport;
pub(crate) mod process;
mod questions;
pub(crate) mod schema;
mod shell;
mod todo;
mod vessel;
pub use vessel::{VesselContext, VesselSettings, VesselTool};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use thiserror::Error;

use crate::{
    config::AccessMode,
    model::ToolDefinition,
    policy::{Decision, Policy},
};
pub use filesystem::{ApplyPatch, ListDirectory, ReadFile, SearchFiles, WriteFile};
pub use process::{
    ProcessTool, TerminalManager, TerminalMetadata, TerminalShutdown, TerminalShutdownFailure,
};
pub use questions::{MAX_ANSWER_BYTES, Question, QuestionAnswer, Questions};
pub use shell::{ManagedShell, SecretShellOutcome, Shell, ShellShutdown};
pub use todo::TodoTool;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("denied: {0}")]
    Denied(String),
    #[error("tool timed out after {0:?}")]
    Timeout(Duration),
    #[error("tool was cancelled")]
    Cancelled,
    #[error("tool failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome;

    /// Optional frontend clarification capability. This does not grant approval.
    /// The default preserves noninteractive/library frontends without reading stdin.
    async fn ask_question(&self, _question: &Question) -> QuestionAnswer {
        QuestionAnswer::Unavailable
    }
}

/// Rechecks optional foreground authority after an awaited approval. It never grants access.
struct DispatchApprover {
    inner: Arc<dyn Approver>,
    policy: Arc<Policy>,
}
#[async_trait]
impl Approver for DispatchApprover {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        if request.cancellation.is_cancelled() {
            return ApprovalOutcome::Cancelled;
        }
        if self.policy.check_execution_authority().is_err() {
            return ApprovalOutcome::Invalidated;
        }
        let outcome = self.inner.approve(request).await;
        if request.cancellation.is_cancelled() {
            ApprovalOutcome::Cancelled
        } else if self.policy.check_execution_authority().is_err() {
            ApprovalOutcome::Invalidated
        } else {
            outcome
        }
    }
    async fn ask_question(&self, question: &Question) -> QuestionAnswer {
        self.inner.ask_question(question).await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    Attended,
    Unattended,
}

/// Runtime-assigned attribution, never supplied by tool arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalSource {
    pub agent_id: uuid::Uuid,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ApprovalSource>,
    /// Cancellation belongs to the executing tool, not the UI connection.
    #[serde(skip)]
    pub cancellation: tokio_util::sync::CancellationToken,
    /// Runtime-only binding; never accepted from a serialized approval response.
    #[serde(skip)]
    pub access_generation: Option<(Arc<crate::policy::LiveAccess>, u64)>,
    pub id: uuid::Uuid,
    pub execution_id: uuid::Uuid,
    pub action: String,
    pub target: String,
    pub reason: String,
    pub mode: InteractionMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalOutcome {
    Approved,
    Denied,
    Expired,
    Cancelled,
    Invalidated,
    Unavailable,
}

pub struct UnattendedApprover {
    pub allow: bool,
}

#[async_trait]
impl Approver for UnattendedApprover {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        let outcome = if self.allow {
            ApprovalOutcome::Approved
        } else {
            ApprovalOutcome::Unavailable
        };
        tracing::warn!(approval_id = %request.id, execution_id = %request.execution_id,
            action = %request.action, target = %request.target, outcome = ?outcome,
            "unattended approval decided without prompting");
        outcome
    }
}

impl ApprovalOutcome {
    /// Preserve why authority was not granted; absence of approval is not a user denial.
    pub fn require_approved(&self) -> Result<(), ToolError> {
        match self {
            Self::Approved => Ok(()),
            Self::Denied => Err(ToolError::Denied("user declined approval".into())),
            Self::Expired => Err(ToolError::Denied(
                "approval expired without a response".into(),
            )),
            Self::Cancelled => Err(ToolError::Cancelled),
            Self::Invalidated => Err(ToolError::Denied(
                "approval invalidated by an authority or access change".into(),
            )),
            Self::Unavailable => Err(ToolError::Denied(
                "approval interface unavailable; no approval was granted".into(),
            )),
        }
    }

    pub fn approved(&self) -> bool {
        *self == Self::Approved
    }
}

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub(crate) fn with_additional(&self, secrets: impl IntoIterator<Item = String>) -> Self {
        let mut combined = self.secrets.clone();
        combined.extend(secrets.into_iter().filter(|secret| secret.len() >= 4));
        combined.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        combined.dedup();
        Self { secrets: combined }
    }
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .filter(|value| value.len() >= 4)
                .collect(),
        }
    }

    /// Test confidentiality without relying on the replacement text differing.
    pub fn contains_secret(&self, text: &str) -> bool {
        self.secrets.iter().any(|secret| text.contains(secret))
    }

    /// Return the raw prefix safe to commit to a public stream. Any suffix that
    /// may complete a configured secret remains private until a later chunk.
    pub(crate) fn stable_prefix(&self, text: &str, flush: bool) -> usize {
        if flush {
            return text.len();
        }
        let mut hold = 0;
        for secret in &self.secrets {
            let pattern = secret.as_bytes();
            let mut prefix = vec![0; pattern.len()];
            for i in 1..pattern.len() {
                let mut matched = prefix[i - 1];
                while matched > 0 && pattern[i] != pattern[matched] {
                    matched = prefix[matched - 1];
                }
                if pattern[i] == pattern[matched] {
                    matched += 1;
                }
                prefix[i] = matched;
            }
            let mut matched = 0;
            for byte in text.as_bytes() {
                while matched > 0 && (matched == pattern.len() || *byte != pattern[matched]) {
                    matched = prefix[matched - 1];
                }
                if *byte == pattern[matched] {
                    matched += 1;
                }
            }
            // A complete secret is safe to redact now. Its proper suffix may
            // still begin an overlapping occurrence, so retain that suffix.
            if matched == pattern.len() {
                matched = prefix[matched - 1];
            }
            hold = hold.max(matched);
        }
        let mut end = text.len() - hold;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        // Descending starts reach the transitive safe boundary in one pass,
        // including overlapping configured values, without quadratic rescans.
        let mut matches = self
            .secrets
            .iter()
            .flat_map(|secret| {
                text.match_indices(secret)
                    .map(move |(start, _)| (start, start + secret.len()))
            })
            .collect::<Vec<_>>();
        matches.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
        for (start, finish) in matches {
            if start < end && finish > end {
                end = start;
            }
        }
        end
    }
    /// Redact source bytes once; replacement markers are never reprocessed.
    pub(crate) fn redact_public_prefix(&self, text: &str) -> String {
        let mut matches = self
            .secrets
            .iter()
            .flat_map(|secret| {
                text.match_indices(secret)
                    .map(move |(start, _)| (start, start + secret.len()))
            })
            .collect::<Vec<_>>();
        matches.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in matches {
            if let Some(previous) = merged.last_mut()
                && start < previous.1
            {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut output = String::new();
        let mut offset = 0;
        for (start, end) in merged {
            output.push_str(&text[offset..start]);
            output.push_str("[REDACTED]");
            offset = end;
        }
        output.push_str(&text[offset..]);
        output
    }
    pub fn redact(&self, input: impl Into<String>) -> String {
        self.secrets.iter().fold(input.into(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
    }
}

#[derive(Clone)]
pub struct ToolContext {
    /// Canonical assistant call being dispatched; never supplied in tool arguments.
    pub tool_call_id: Option<String>,
    pub artifact_scope: Option<crate::artifacts::Scope>,
    /// Dedicated capability, never forwarded to shell/MCP or serialized.
    pub github: Option<crate::github::Credential>,
    pub completion: Option<crate::completion::runtime::RunHandle>,
    pub policy: Arc<Policy>,
    pub approver: Arc<dyn Approver>,
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub environment: BTreeMap<String, String>,
    pub cancellation: tokio_util::sync::CancellationToken,
    pub execution_id: uuid::Uuid,
    pub interaction: InteractionMode,
    pub redactor: Arc<Redactor>,
}

impl ToolContext {
    pub fn approval(
        &self,
        action: &str,
        target: impl Into<String>,
        reason: String,
    ) -> ApprovalRequest {
        ApprovalRequest {
            source: None,
            cancellation: self.cancellation.clone(),
            access_generation: self.policy.access_binding(),
            id: uuid::Uuid::new_v4(),
            execution_id: self.execution_id,
            action: action.into(),
            target: self.redactor.redact(target.into()),
            reason: self.redactor.redact(reason),
            mode: self.interaction.clone(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError>;
    /// Ordered result contract; ordinary text tools retain their existing implementation.
    async fn execute_output(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<voyage_protocol::tool_result::ToolOutput, ToolError> {
        self.execute(arguments, context)
            .await
            .map(voyage_protocol::tool_result::ToolOutput::text)
    }
    /// One dispatch only: legacy typed tools adapt without executing twice.
    async fn execute_report(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<ToolReport, ToolError> {
        self.execute_output(arguments, context)
            .await
            .map(ToolReport::output)
    }
    /// Trusted one-shot implementations may consume a current-run environment.
    /// The typed result cannot carry captured output or arbitrary diagnostic text.
    async fn execute_secret_environment(
        &self,
        _arguments: Value,
        _context: &ToolContext,
        _environment: crate::workflow::secrets::BoundEnvironment,
    ) -> Result<shell::SecretShellOutcome, ToolError> {
        Err(ToolError::InvalidArguments(
            "this tool implementation does not support workflow secret bindings".into(),
        ))
    }
}

struct ToolContract {
    definition: ToolDefinition,
    input: schema::CompiledSchema,
    output: Option<schema::CompiledSchema>,
}

impl ToolContract {
    fn compile(definition: ToolDefinition) -> Result<Self, ToolError> {
        let input = schema::CompiledSchema::compile(&definition.input_schema)?;
        let output = definition
            .output_schema
            .as_ref()
            .map(schema::CompiledSchema::compile)
            .transpose()?;
        Ok(Self {
            definition,
            input,
            output,
        })
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    contracts: BTreeMap<String, ToolContract>,
    terminals: Option<ProcessTool>,
    mcp: Vec<mcp::McpLease>,
    extensions: Option<Arc<crate::extensions::runtime::Manager>>,
}

impl ToolRegistry {
    pub(crate) fn own_extensions(&mut self, manager: Arc<crate::extensions::runtime::Manager>) {
        self.extensions = Some(manager);
    }
    pub(crate) fn extensions(&self) -> Option<Arc<crate::extensions::runtime::Manager>> {
        self.extensions.clone()
    }
    pub fn own_mcp(&mut self, server: Arc<mcp::McpServer>) {
        self.mcp.push(mcp::McpLease(server));
    }
    pub fn mcp_servers(&self) -> Vec<Arc<mcp::McpServer>> {
        self.mcp.iter().map(|lease| lease.0.clone()).collect()
    }
    pub fn standard() -> Self {
        Self::standard_with_terminal_limits(16, 8 * 1024 * 1024)
    }
    pub fn standard_with_terminal_limits(max_count: usize, max_unread_bytes: usize) -> Self {
        let mut registry = Self::default();
        registry.register(ReadFile);
        registry.register(Questions);
        registry.register(WriteFile);
        registry.register(ListDirectory);
        registry.register(SearchFiles);
        registry.register(Shell);
        registry.register(ApplyPatch);
        let terminals = ProcessTool::with_limits(max_count, max_unread_bytes);
        registry.terminals = Some(terminals.clone());
        registry.register(terminals);
        registry
    }
    pub async fn shutdown_terminals(&self, timeout: Duration) -> TerminalShutdown {
        match &self.terminals {
            Some(terminals) => terminals.shutdown(timeout).await,
            None => TerminalShutdown::empty(),
        }
    }
    pub fn terminals(&self) -> Option<ProcessTool> {
        self.terminals.clone()
    }
    pub(crate) fn reuse_terminals(&mut self, terminals: ProcessTool) {
        if self.terminals.is_some() {
            self.terminals = Some(terminals.clone());
            self.register(terminals);
        }
    }
    pub fn register_subagents(
        &mut self,
        tool: crate::subagent::SubagentTool,
    ) -> Result<(), ToolError> {
        self.register_arc(Arc::new(tool))
    }
    pub fn register_todos(&mut self, tool: TodoTool) -> Result<(), ToolError> {
        self.register_arc(Arc::new(tool))
    }
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let contract =
            ToolContract::compile(tool.definition()).expect("built-in tool schema must compile");
        let name = contract.definition.name.clone();
        self.contracts.insert(name.clone(), contract);
        self.tools.insert(name, Arc::new(tool));
    }
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let definition = tool.definition();
        let name = definition.name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolError::Failed(format!("duplicate tool name `{name}`")));
        }
        let contract = ToolContract::compile(definition)?;
        self.contracts.insert(name.clone(), contract);
        self.tools.insert(name, tool);
        Ok(())
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.contracts
            .values()
            .map(|contract| contract.definition.clone())
            .collect()
    }
    pub fn retain_allowed(&mut self, allowed: &std::collections::BTreeSet<String>) {
        if let Some(manager) = &self.extensions {
            manager.restrict_host_read(allowed.contains("read_file"));
        }
        self.tools.retain(|name, _| allowed.contains(name));
        self.contracts.retain(|name, _| allowed.contains(name));
        if !allowed.contains("process") {
            self.terminals = None;
        }
    }
    pub fn retain_read_only(&mut self) {
        self.tools.retain(|name, _| {
            matches!(
                name.as_str(),
                "questions"
                    | "read_file"
                    | "list_directory"
                    | "search_files"
                    | "process"
                    | "subagent"
                    | "todo"
                    | "completion"
                    | "github"
                    | "vessel"
            )
        });
        self.contracts
            .retain(|name, _| self.tools.contains_key(name));
    }
    /// Check structured successful output against the snapshotted discovery contract.
    pub(crate) fn validate_output(
        &self,
        name: &str,
        value: Option<&Value>,
    ) -> Result<(), ToolError> {
        if let Some(schema) = self
            .contracts
            .get(name)
            .and_then(|contract| contract.output.as_ref())
        {
            let value = value.ok_or_else(|| {
                ToolError::Failed("tool omitted declared structured output".into())
            })?;
            schema.validate(value).map_err(|_| {
                ToolError::Failed("tool output does not match its declared JSON Schema".into())
            })?;
        }
        Ok(())
    }

    pub async fn execute(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
    ) -> Result<String, ToolError> {
        self.execute_with_workflow_secrets(name, arguments, context, None)
            .await
    }

    pub async fn execute_with_workflow_secrets(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
        bindings: Option<&crate::workflow::secrets::RunBindings>,
    ) -> Result<String, ToolError> {
        let output = self
            .execute_output_with_workflow_secrets(name, arguments, context, bindings)
            .await?;
        if output.is_error {
            Err(ToolError::Failed(output.text_fallback()))
        } else {
            Ok(output.text_fallback())
        }
    }

    pub async fn execute_output_with_workflow_secrets(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
        bindings: Option<&crate::workflow::secrets::RunBindings>,
    ) -> Result<voyage_protocol::tool_result::ToolOutput, ToolError> {
        self.execute_report_with_workflow_secrets(name, arguments, context, bindings)
            .await
            .map(|report| report.output)
    }

    pub async fn execute_report_with_workflow_secrets(
        &self,
        name: &str,
        arguments: Value,
        context: &ToolContext,
        bindings: Option<&crate::workflow::secrets::RunBindings>,
    ) -> Result<ToolReport, ToolError> {
        self.contracts
            .get(name)
            .ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?
            .input
            .validate(&arguments)?;
        // Bind every permission decision (including MCP approval) to one access generation.
        let mut dispatch_context = context.clone();
        dispatch_context.policy = Arc::new(context.policy.for_dispatch());
        dispatch_context.approver = Arc::new(DispatchApprover {
            inner: context.approver.clone(),
            policy: dispatch_context.policy.clone(),
        });
        let context = &dispatch_context;
        let private_environment = if let Some(references) = arguments.get("workflow_secrets") {
            if name != "shell" {
                return Err(ToolError::InvalidArguments(
                    "workflow secrets are supported only by the one-shot shell tool".into(),
                ));
            }
            let references: Vec<String> =
                serde_json::from_value(references.clone()).map_err(|_| {
                    ToolError::InvalidArguments(
                        "workflow secret references must be an array of names".into(),
                    )
                })?;
            if references.is_empty() {
                None
            } else {
                let bindings = bindings.ok_or_else(|| {
                    ToolError::InvalidArguments(
                        "workflow secret references are unavailable in this run".into(),
                    )
                })?;
                Some(
                    bindings
                        .resolve(context.execution_id, &references)
                        .map_err(|e| ToolError::InvalidArguments(e.to_string()))?,
                )
            }
        } else {
            None
        };
        if context.policy.access_mode() == AccessMode::ReadOnly
            && !allowed_in_read_only(name, &arguments)
        {
            return Err(ToolError::Denied(format!(
                "`{name}` action is disabled in read-only access mode"
            )));
        }
        if name.starts_with("mcp_") {
            match context.policy.external_tool(name) {
                Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
                Decision::Ask(reason) => context
                    .approver
                    .approve(&context.approval("mcp.call", name, reason.clone()))
                    .await
                    .require_approved()?,
                _ => {}
            }
        }
        context
            .policy
            .check_execution_authority()
            .map_err(|_| ToolError::Denied("foreground execution authority unavailable".into()))?;
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let mut dispatch = context.clone();
        dispatch.approver = Arc::new(DispatchApprover {
            inner: context.approver.clone(),
            policy: context.policy.clone(),
        });
        if let Some(environment) = private_environment {
            if environment.iter().any(|(name, _)| {
                // Generated keys are ASCII. Conservatively reserve their case
                // aliases on every platform, including Unicode case mappings,
                // rather than allowing a Windows Command.env overwrite.
                context.environment.keys().any(|configured| {
                    configured.to_uppercase() == name
                        || configured.to_lowercase() == name.to_ascii_lowercase()
                })
            }) {
                return Err(ToolError::InvalidArguments(
                    "workflow secret environment conflicts with configured environment".into(),
                ));
            }
            let tool = self
                .tools
                .get(name)
                .ok_or_else(|| ToolError::Failed("one-shot shell tool is unavailable".into()))?;
            let outcome = tool
                .execute_secret_environment(arguments, &dispatch, environment)
                .await
                .map_err(|e| e.redacted(&context.redactor))?;
            // Only fixed enum/status metadata reaches serialization; no raw tool
            // string or secret-dependent redaction can corrupt this envelope.
            let text = serde_json::to_string(&outcome)
                .map_err(|_| ToolError::Failed("one-shot shell outcome encoding failed".into()))?;
            let code = match outcome {
                SecretShellOutcome::Exited { code } => Some(code),
                SecretShellOutcome::Signalled => None,
            };
            return Ok(ToolReport::command(text, code, false));
        }
        let result = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?
            .execute_report(arguments, &dispatch)
            .await;
        let mut report = result.map_err(|error| error.redacted(&context.redactor))?;
        if !report.output.is_error
            && self
                .validate_output(name, report.output.structured_content.as_ref())
                .is_err()
        {
            report.limit(
                voyage_protocol::tool_result::IncompleteReason::Withheld,
                "Tool returned invalid structured output; output withheld. Do not replay effects.",
            );
        }
        let redaction = if name == "questions" {
            questions::redact_result(&report.output.text_fallback(), &context.redactor).map(
                |text| {
                    report.output = voyage_protocol::tool_result::ToolOutput::text(text);
                },
            )
        } else {
            output::redact(&mut report.output, &context.redactor)
        };
        if redaction.is_err() {
            report.limit(
                voyage_protocol::tool_result::IncompleteReason::Withheld,
                "Tool output withheld by confidentiality checks; effects were not replayed.",
            );
        }
        if report.output.text_fallback().len() > context.max_output_bytes {
            report.limit(
                voyage_protocol::tool_result::IncompleteReason::OutputLimit,
                "Tool output exceeds the configured budget; effects were not replayed.",
            );
        }
        // Even diagnostics must obey tiny budgets, without dropping known exit facts.
        if report.output.text_fallback().len() > context.max_output_bytes {
            report.output = voyage_protocol::tool_result::ToolOutput::text("");
        }
        report.synchronize();
        Ok(report)
    }
}

fn allowed_in_read_only(name: &str, arguments: &Value) -> bool {
    let action = arguments.get("action").and_then(Value::as_str);
    match name {
        "browser" => {
            serde_json::from_value::<voyage_protocol::browser::BrowserAction>(arguments.clone())
                .is_ok_and(|a| a.observation_only())
        }
        "questions" | "read_file" | "list_directory" | "search_files" => true,
        "process" => matches!(action, Some("read" | "list")),
        "todo" => action == Some("list"),
        "completion" => matches!(action, Some("snapshot" | "read")),
        "github" => matches!(action, Some("read" | "logs" | "inspect" | "list")),
        "vessel" => matches!(
            action,
            Some(
                "inspect"
                    | "capabilities"
                    | "list"
                    | "search"
                    | "history"
                    | "follow"
                    | "wait"
                    | "receipt"
                    | "routes"
                    | "operations"
                    | "controls"
            )
        ),
        "subagent" => match action {
            Some(
                "status" | "list" | "archive" | "wait" | "wait_many" | "message" | "follow_up"
                | "worktree_status" | "worktree_conflicts" | "cancel",
            ) => true,
            Some("spawn") => !arguments
                .get("worktree")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            _ => false,
        },
        _ => false,
    }
}

impl ToolError {
    fn redacted(self, redactor: &Redactor) -> Self {
        match self {
            Self::InvalidArguments(message) => Self::InvalidArguments(redactor.redact(message)),
            Self::Denied(message) => Self::Denied(redactor.redact(message)),
            Self::Failed(message) => Self::Failed(redactor.redact(message)),
            Self::Timeout(duration) => Self::Timeout(duration),
            Self::Cancelled => Self::Cancelled,
        }
    }
}

pub(crate) fn truncate(mut bytes: Vec<u8>, max: usize) -> String {
    if bytes.len() <= max {
        return String::from_utf8_lossy(&bytes).into_owned();
    }
    bytes.truncate(max);
    format!(
        "{}\n\n[output truncated at {max} bytes]",
        String::from_utf8_lossy(&bytes)
    )
}
