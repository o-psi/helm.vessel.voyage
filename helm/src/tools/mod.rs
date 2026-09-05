mod filesystem;
pub mod mcp;
mod process;
mod questions;
mod shell;
mod todo;

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
        if self.policy.check_execution_authority().is_err() {
            return ApprovalOutcome::Denied;
        }
        let outcome = self.inner.approve(request).await;
        if self.policy.check_execution_authority().is_err() {
            ApprovalOutcome::Denied
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
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
    pub fn approved(&self) -> bool {
        *self == Self::Approved
    }
}

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .filter(|value| value.len() >= 4)
                .collect(),
        }
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

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    terminals: Option<ProcessTool>,
}

impl ToolRegistry {
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
        self.tools.insert(tool.definition().name, Arc::new(tool));
    }
    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.definition().name;
        if self.tools.contains_key(&name) {
            return Err(ToolError::Failed(format!("duplicate tool name `{name}`")));
        }
        self.tools.insert(name, tool);
        Ok(())
    }
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }
    pub fn retain_allowed(&mut self, allowed: &std::collections::BTreeSet<String>) {
        self.tools.retain(|name, _| allowed.contains(name));
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
            )
        });
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
                Decision::Ask(reason)
                    if !context
                        .approver
                        .approve(&context.approval("mcp.call", name, reason.clone()))
                        .await
                        .approved() =>
                {
                    return Err(ToolError::Denied("user declined approval".into()));
                }
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
            if environment
                .iter()
                .any(|(name, _)| context.environment.contains_key(name))
            {
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
            return serde_json::to_string(&outcome)
                .map_err(|_| ToolError::Failed("one-shot shell outcome encoding failed".into()));
        }
        let result = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::Failed(format!("unknown tool `{name}`")))?
            .execute(arguments, &dispatch)
            .await;
        result
            .and_then(|output| {
                if name == "questions" {
                    questions::redact_result(&output, &context.redactor)
                } else {
                    Ok(context.redactor.redact(output))
                }
            })
            .map_err(|error| error.redacted(&context.redactor))
    }
}

fn allowed_in_read_only(name: &str, arguments: &Value) -> bool {
    let action = arguments.get("action").and_then(Value::as_str);
    match name {
        "questions" | "read_file" | "list_directory" | "search_files" => true,
        "process" => matches!(action, Some("read" | "list")),
        "todo" => action == Some("list"),
        "completion" => matches!(action, Some("snapshot" | "read")),
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

#[cfg(test)]
mod security_tests {
    use super::*;

    #[derive(Debug)]
    struct Revocable(std::sync::atomic::AtomicBool);
    impl crate::policy::ExecutionAuthority for Revocable {
        fn check(&self) -> anyhow::Result<()> {
            anyhow::ensure!(self.0.load(std::sync::atomic::Ordering::SeqCst), "revoked");
            Ok(())
        }
    }
    struct RevokingApprover(Arc<Revocable>);
    #[async_trait]
    impl Approver for RevokingApprover {
        async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
            tokio::task::yield_now().await;
            self.0.0.store(false, std::sync::atomic::Ordering::SeqCst);
            ApprovalOutcome::Approved
        }
    }
    #[tokio::test]
    async fn foreground_revocation_during_approval_does_not_grant_dispatch() {
        let workspace = tempfile::tempdir().unwrap();
        let authority = Arc::new(Revocable(std::sync::atomic::AtomicBool::new(true)));
        let policy = Arc::new(
            Policy::new(&crate::config::Config::default(), workspace.path().into())
                .unwrap()
                .with_execution_authority(authority.clone()),
        );
        let approver = DispatchApprover {
            inner: Arc::new(RevokingApprover(authority)),
            policy: policy.clone(),
        };
        assert!(policy.check_current().is_ok());
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            execution_id: uuid::Uuid::new_v4(),
            action: "write".into(),
            target: "file".into(),
            reason: "approval".into(),
            mode: InteractionMode::Unattended,
        };
        assert_eq!(approver.approve(&request).await, ApprovalOutcome::Denied);
        assert!(policy.check_current().is_err());
        assert!(policy.clone().check_execution_authority().is_err());
    }

    #[test]
    fn streaming_public_redaction_handles_overlaps_all_secret_and_placeholder_once() {
        for (secrets, text) in [
            (vec!["ababa"], "abababa"),
            (vec!["aaaa", "aaaaa"], "aaaaaaaaaaaaaaaaa"),
            (vec!["REMOTE_SECRET"], "REMOTE_SECRET"),
            (vec!["秘密🔐canary"], "before 秘密🔐canary after"),
            (vec!["SECRET", "REDACTED"], "SECRET"),
        ] {
            let redactor = Redactor::new(secrets.iter().map(|value| (*value).into()));
            let expected = redactor.redact_public_prefix(text);
            let boundaries = text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
                .collect::<Vec<_>>();
            for split in boundaries {
                let mut pending = String::new();
                let mut disclosed = String::new();
                for chunk in [&text[..split], &text[split..]] {
                    pending.push_str(chunk);
                    let end = redactor.stable_prefix(&pending, false);
                    disclosed.push_str(&redactor.redact_public_prefix(&pending[..end]));
                    pending.drain(..end);
                }
                disclosed.push_str(&redactor.redact_public_prefix(&pending));
                assert_eq!(disclosed, expected, "split {split} in {text}");
                for secret in &secrets {
                    if !"[REDACTED]".contains(secret) {
                        assert!(!disclosed.contains(secret));
                    }
                }
            }
        }
    }
    #[test]
    fn read_only_completion_allows_observation_but_not_adoption_or_reviews() {
        for action in ["snapshot", "read"] {
            assert!(allowed_in_read_only(
                "completion",
                &serde_json::json!({"action":action})
            ));
        }
        for action in ["adopt", "account", "unknown"] {
            assert!(!allowed_in_read_only(
                "completion",
                &serde_json::json!({"action":action})
            ));
        }
    }
    #[test]
    fn redacts_all_occurrences_without_echoing_short_values() {
        let redactor = Redactor::new(["long-secret".into(), "abc".into()]);
        assert_eq!(
            redactor.redact("long-secret / long-secret / abc"),
            "[REDACTED] / [REDACTED] / abc"
        );
    }

    #[tokio::test]
    async fn unattended_denial_is_immediate_and_explicit() {
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            execution_id: uuid::Uuid::new_v4(),
            action: "test".into(),
            target: "target".into(),
            reason: "risk".into(),
            mode: InteractionMode::Unattended,
        };
        let outcome = tokio::time::timeout(
            Duration::from_millis(50),
            UnattendedApprover { allow: false }.approve(&request),
        )
        .await
        .unwrap();
        assert_eq!(outcome, ApprovalOutcome::Unavailable);
    }

    struct DeniedTool;
    #[async_trait]
    impl Tool for DeniedTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "denied".into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
            }
        }
        async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
            Err(ToolError::Denied("long-secret".into()))
        }
    }

    #[tokio::test]
    async fn registry_preserves_typed_errors_while_redacting() {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::config::Config::default();
        let context = ToolContext {
            completion: None,
            policy: Arc::new(Policy::new(&config, directory.path().to_owned()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::new(["long-secret".into()])),
        };
        let mut registry = ToolRegistry::default();
        registry.register(DeniedTool);
        assert!(
            matches!(registry.execute("denied",serde_json::json!({}),&context).await,Err(ToolError::Denied(message)) if message=="[REDACTED]")
        );
    }

    #[tokio::test]
    async fn read_only_registry_blocks_mutations_before_tool_dispatch() {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            access: Some(AccessMode::ReadOnly),
            ..crate::config::Config::default()
        };
        let context = ToolContext {
            completion: None,
            policy: Arc::new(Policy::new(&config, directory.path().to_owned()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: true }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::default()),
        };
        let registry = ToolRegistry::standard();
        assert!(matches!(
            registry
                .execute("shell", serde_json::json!({"command":"pwd"}), &context)
                .await,
            Err(ToolError::Denied(message)) if message.contains("read-only")
        ));
        assert!(matches!(
            registry
                .execute(
                    "process",
                    serde_json::json!({"action":"write","data":"secret"}),
                    &context,
                )
                .await,
            Err(ToolError::Denied(message)) if message.contains("read-only")
        ));
        assert!(
            registry
                .execute("list_directory", serde_json::json!({"path":"."}), &context)
                .await
                .is_ok()
        );

        let mut visible = ToolRegistry::standard();
        visible.retain_read_only();
        let names = visible
            .definitions()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "list_directory",
                "process",
                "questions",
                "read_file",
                "search_files"
            ]
        );
    }
}
