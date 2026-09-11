//! Command-line grammar and explicit configuration selection.
use crate::managed;
use clap::{Parser, Subcommand};
use clap_complete::Shell;
use helm::config::AccessMode;
use std::path::PathBuf;
#[derive(Parser)]
#[command(version, about = "A general-purpose LLM harness for terminal work")]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) policy: helm::policy_profile::cli::SelectionArgs,
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,
    /// Override a configuration value for this invocation (`KEY=VALUE`).
    #[arg(long = "set", global = true, value_name = "KEY=VALUE")]
    pub(crate) set: Vec<String>,
    #[arg(long, global = true)]
    pub(crate) model: Option<String>,
    /// Provider transport (`openai` retains its legacy Chat Completions behavior).
    #[arg(long, global = true, value_enum)]
    pub(crate) provider: Option<ProviderArg>,
    #[arg(long, global = true)]
    pub(crate) workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum, hide = true)]
    pub(crate) approval: Option<ApprovalArg>,
    /// Agent authority: inspect only, ask on consequential actions, or proceed without prompts.
    #[arg(long, global = true, value_enum, conflicts_with = "approval")]
    pub(crate) access: Option<AccessArg>,
    #[arg(short, long, global = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
    pub(crate) verbose: bool,
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub(crate) log_format: LogFormat,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Clone, clap::ValueEnum)]
pub(crate) enum ApprovalArg {
    Always,
    OnRisk,
    Never,
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum AccessArg {
    ReadOnly,
    Approval,
    Unrestricted,
}

impl From<AccessArg> for AccessMode {
    fn from(value: AccessArg) -> Self {
        match value {
            AccessArg::ReadOnly => Self::ReadOnly,
            AccessArg::Approval => Self::Approval,
            AccessArg::Unrestricted => Self::Unrestricted,
        }
    }
}
#[derive(Clone, clap::ValueEnum)]
pub(crate) enum ProviderArg {
    #[value(name = "openai-responses")]
    OpenaiResponses,
    #[value(name = "openai-chat", alias = "openai", alias = "openai-compatible")]
    OpenaiChat,
    #[value(name = "chatgpt-oauth")]
    ChatGptOauth,
    Anthropic,
}

impl From<ProviderArg> for helm::ProviderKind {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::OpenaiResponses => Self::OpenaiResponses,
            ProviderArg::OpenaiChat => Self::OpenaiChat,
            ProviderArg::ChatGptOauth => Self::ChatGptOauth,
            ProviderArg::Anthropic => Self::Anthropic,
        }
    }
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum LogFormat {
    Text,
    Json,
}
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Install or inspect the local shared-browser adapter (never shares a browser).
    Browser(helm::process_client::browser::BrowserArgs),
    /// Connect through local HTTP or scoped HTTPS.
    Connect(helm::process_client::cli::ConnectArgs),
    /// Inspect GitHub context and publish only after exact attended review.
    Github(GithubArgs),
    /// Install, inspect and explicitly review declarative or executable packages.
    Extension(helm::extensions::cli::ExtensionArgs),
    /// Inspect and configure local session/project inference attempt allowances.
    Inference(helm::inference::cli::InferenceArgs),
    /// Manage named policy profiles and preview explicit launch selection.
    Policy(helm::policy_profile::cli::PolicyArgs),
    /// Discover, inspect and run saved nonsecret workflows.
    Workflow(helm::workflow::WorkflowArgs),
    /// Inspect a repository and explicitly review generated project guidance.
    Onboard(helm::onboarding::OnboardArgs),
    /// Use private local sessions with an authoritative SQLite journal.
    Managed(managed::Args),
    Run {
        #[arg(required = true)]
        prompt: Vec<String>,
        #[arg(long)]
        resume: Option<String>,
        #[arg(long)]
        no_save: bool,
    },
    Chat {
        #[arg(long)]
        resume: Option<String>,
        /// Use the line-oriented interface, even when attached to a terminal.
        #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
        plain: bool,
    },
    Sessions,
    /// List models available to the configured provider/account.
    Models {
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect resolved configuration with secret bindings concealed.
    Config,
    /// Explicit local/compatible endpoint setup and discovery.
    LocalProvider {
        #[command(subcommand)]
        command: helm::local_provider::Command,
    },
    /// Generate a shell completion script on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff manpage on stdout.
    Manpage,
    /// Check configuration and local runtime dependencies without contacting a model.
    Doctor,
}

#[derive(clap::Args)]
pub(crate) struct GithubArgs {
    /// Existing local voyage whose references and operation scope to use.
    #[arg(long)]
    pub(crate) session: Option<String>,
    #[command(flatten)]
    pub(crate) args: helm::github::operator::Args,
}
