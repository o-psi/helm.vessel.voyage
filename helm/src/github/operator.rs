//! Explicit idle-voyage actions; callers retain their existing session lease.
use super::repository::Object;
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub object: Object,
    pub head: Option<String>,
    pub fetched_at: DateTime<Utc>,
}
impl Reference {
    pub fn validate(&self) -> Result<()> {
        self.object.validate()?;
        ensure!(
            (self.object.kind == super::repository::ObjectKind::PullRequest) == self.head.is_some(),
            "GitHub reference head identity is inconsistent"
        );
        if let Some(head) = &self.head {
            super::context::sha(&serde_json::Value::String(head.clone()))?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct Feedback {
    pub source: String,
    pub title: String,
    pub description: String,
}
impl Feedback {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.title.trim().is_empty()
                && self.title.len() <= 256
                && self.description.len() <= 16 * 1024
                && self.source.len() <= 1024,
            "GitHub feedback exceeds bounds or has no title"
        );
        let mut source = reqwest::Url::parse(&self.source)
            .map_err(|_| anyhow::anyhow!("invalid GitHub feedback source"))?;
        if let Some(fragment) = source.fragment() {
            let id = ["issuecomment-", "pullrequestreview-", "discussion_r"]
                .iter()
                .find_map(|prefix| fragment.strip_prefix(prefix))
                .ok_or_else(|| anyhow::anyhow!("unsupported GitHub feedback source fragment"))?;
            ensure!(
                id.parse::<u64>()
                    .is_ok_and(|id| id > 0 && id <= i64::MAX as u64),
                "invalid GitHub feedback source identity"
            );
        }
        source.set_fragment(None);
        let object = Object::parse(source.as_str())?;
        ensure!(
            source.as_str() == object.url(),
            "noncanonical GitHub feedback source"
        );
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct CommandResult {
    pub display: String,
    pub reference: Option<Reference>,
    pub feedback: Option<Feedback>,
}

fn attributed_feedback(
    object: &Object,
    entry: &serde_json::Value,
    fetched_at: DateTime<Utc>,
    id: Option<u64>,
) -> Result<Feedback> {
    let source = entry["html_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("feedback source URL is missing"))?;
    let mut parsed = reqwest::Url::parse(source)
        .map_err(|_| anyhow::anyhow!("feedback source URL is malformed"))?;
    let fragment = parsed.fragment().map(str::to_owned);
    parsed.set_fragment(None);
    ensure!(
        Object::parse(parsed.as_str())? == *object,
        "feedback source belongs to another object"
    );
    let source = format!(
        "{}{}",
        object.url(),
        fragment
            .map(|fragment| format!("#{fragment}"))
            .unwrap_or_default()
    );
    let body = entry["body"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("feedback body is unavailable"))?;
    let author = match (
        entry["user"]["login"].as_str(),
        entry["user"]["id"].as_u64(),
    ) {
        (Some(login), Some(id)) if !login.is_empty() && login.len() <= 100 && id > 0 => {
            format!("{login} (GitHub user {id})")
        }
        _ => "Unavailable in the GitHub response".into(),
    };
    let feedback = Feedback {
        source,
        title: format!(
            "GitHub {}#{} feedback{}",
            object.repository.slug(),
            object.number,
            id.map(|id| format!(" {id}")).unwrap_or_default()
        ),
        description: format!(
            "GitHub author: {author}\nObserved: {}\n\n{body}",
            fetched_at.to_rfc3339()
        ),
    };
    feedback.validate()?;
    Ok(feedback)
}

#[derive(Clone, Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Clone, Debug, clap::Subcommand)]
pub enum Command {
    /// Attended local recovery across orphaned voyage/workspace records.
    Admin {
        #[command(subcommand)]
        command: super::admin::Command,
    },
    /// Show the explicitly delegated GitHub account (never the token).
    Auth,
    /// Read bounded job logs associated with the selected pull-request head SHA.
    Logs {
        url: String,
        job: u64,
    },
    /// Inspect local remote candidates without choosing between forks.
    Remotes,
    View(ViewArgs),
    /// Follow an exact JSON continuation returned by a prior observation.
    Continue {
        request: String,
    },
    /// Return a reference for the owning idle voyage to save.
    Reference {
        url: String,
    },
    /// List references saved in the selected voyage (frontend-owned).
    References,
    /// Remove one saved reference without changing GitHub.
    Unreference {
        url: String,
    },
    /// Import selected attributed feedback as open local work.
    Feedback {
        #[command(flatten)]
        view: ViewArgs,
        #[arg(long)]
        id: Option<u64>,
    },
    Prepare(PrepareArgs),
    Publish {
        id: uuid::Uuid,
        digest: String,
    },
    Inspect {
        id: uuid::Uuid,
    },
    List {
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    Cancel {
        id: uuid::Uuid,
        digest: String,
    },
    Forget {
        id: uuid::Uuid,
        digest: String,
    },
    Reconcile {
        id: uuid::Uuid,
        digest: String,
        remote_id: u64,
    },
    Dispose {
        id: uuid::Uuid,
        digest: String,
        note: String,
    },
}
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum SectionArg {
    Details,
    Comments,
    Reviews,
    Files,
    Threads,
    ThreadComments,
    ReviewComments,
    Statuses,
    Checks,
    Suites,
    SuiteChecks,
    Annotations,
    Runs,
    Jobs,
}
#[derive(Clone, Debug, clap::Args)]
pub struct ViewArgs {
    pub url: String,
    #[arg(long, value_enum, default_value_t = SectionArg::Details)]
    pub section: SectionArg,
    #[arg(long, default_value_t = 1)]
    pub page: u32,
    #[arg(long)]
    pub after: Option<String>,
    /// Exact review/check/suite/run numeric identity for nested observations.
    #[arg(long)]
    pub resource: Option<u64>,
    #[arg(long)]
    pub thread: Option<String>,
    #[arg(long)]
    pub head: Option<String>,
}
impl ViewArgs {
    fn read(self) -> Result<super::context::Read> {
        use super::context::Section;
        let resource = || {
            self.resource
                .ok_or_else(|| anyhow::anyhow!("this section requires --resource ID"))
        };
        let section = match self.section {
            SectionArg::Details => Section::Details,
            SectionArg::Comments => Section::Comments,
            SectionArg::Reviews => Section::Reviews,
            SectionArg::Files => Section::Files,
            SectionArg::Threads => Section::Threads { after: self.after },
            SectionArg::ThreadComments => Section::ThreadComments {
                thread: self
                    .thread
                    .ok_or_else(|| anyhow::anyhow!("thread comments require --thread NODE_ID"))?,
                after: self.after,
            },
            SectionArg::ReviewComments => Section::ReviewComments {
                review: resource()?,
            },
            SectionArg::Statuses => Section::Statuses,
            SectionArg::Checks => Section::Checks,
            SectionArg::Suites => Section::CheckSuites,
            SectionArg::SuiteChecks => Section::SuiteChecks { suite: resource()? },
            SectionArg::Annotations => Section::Annotations { check: resource()? },
            SectionArg::Runs => Section::WorkflowRuns,
            SectionArg::Jobs => Section::Jobs { run: resource()? },
        };
        let request = super::context::Read {
            object: Object::parse(&self.url)?,
            section,
            page: self.page,
            expected_head: self.head,
            expected_base: None,
        };
        request.validate()?;
        Ok(request)
    }
}
#[derive(Clone, Debug, clap::Args)]
pub struct PrepareArgs {
    pub url: Option<String>,
    #[arg(long, conflicts_with_all = ["body_file", "draft_file"])]
    pub body: Option<String>,
    #[arg(long, conflicts_with = "draft_file")]
    pub body_file: Option<std::path::PathBuf>,
    /// Full typed Draft JSON, including supported inline comments.
    #[arg(long)]
    pub draft_file: Option<std::path::PathBuf>,
    #[arg(long, value_parser = ["COMMENT", "APPROVE", "REQUEST_CHANGES"])]
    pub event: Option<String>,
    #[arg(long)]
    pub commit: Option<String>,
}

pub async fn execute(
    context: crate::tools::ToolContext,
    session_id: Option<uuid::Uuid>,
    words: Vec<String>,
) -> Result<CommandResult> {
    use clap::Parser;
    #[derive(clap::Parser)]
    #[command(name = "github")]
    struct Parsed {
        #[command(flatten)]
        args: Args,
    }
    let parsed = Parsed::try_parse_from(std::iter::once("github".to_owned()).chain(words));
    let args = match parsed {
        Ok(parsed) => parsed.args,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            return Ok(CommandResult {
                display: error.to_string(),
                reference: None,
                feedback: None,
            });
        }
        Err(_) => anyhow::bail!("invalid GitHub arguments; use /github --help"),
    };
    execute_args(context, session_id, args).await
}

pub async fn execute_args(
    mut context: crate::tools::ToolContext,
    session_id: Option<uuid::Uuid>,
    args: Args,
) -> Result<CommandResult> {
    use super::{
        publication::{Action, Draft, ReviewEvent},
        service::Service,
    };
    if let Some(token) = context
        .github
        .as_ref()
        .map(|credential| credential.expose())
    {
        context.redactor = std::sync::Arc::new(
            context
                .redactor
                .with_additional(super::credential_forms(token)),
        );
    }
    context.policy.check_current()?;
    ensure!(
        !context.cancellation.is_cancelled(),
        "GitHub operator action cancelled"
    );
    let mut result = CommandResult {
        display: String::new(),
        reference: None,
        feedback: None,
    };
    if let Command::Admin { command } = args.command {
        return super::admin::execute(context, command, None).await;
    }
    if let Command::Dispose { id, digest, note } = args.command {
        let owner = super::store::Owner::new(context.policy.workspace(), session_id, None)?;
        return super::admin::execute(
            context,
            super::admin::Command::Dispose { id, digest, note },
            Some(owner),
        )
        .await;
    }
    ensure!(
        !matches!(
            args.command,
            Command::References | Command::Unreference { .. }
        ),
        "reference maintenance requires an owning voyage frontend"
    );
    // Offline receipt administration does not require a usable GitHub token.
    if matches!(
        args.command,
        Command::Inspect { .. }
            | Command::List { .. }
            | Command::Cancel { .. }
            | Command::Forget { .. }
    ) {
        let write = matches!(
            args.command,
            Command::Cancel { .. } | Command::Forget { .. }
        );
        ensure!(
            !write || context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
            "GitHub journal maintenance is denied in read-only mode"
        );
        let owner = super::store::Owner::new(context.policy.workspace(), session_id, None)?;
        let policy = context.policy.clone();
        let cancel = context.cancellation.clone();
        let value = super::service::database(move |store| {
            policy.check_current()?;
            ensure!(!write || policy.access_mode() != crate::config::AccessMode::ReadOnly,
                "GitHub journal maintenance is denied in read-only mode");
            ensure!(!cancel.is_cancelled(), "GitHub operator action cancelled");
            match args.command {
                Command::Inspect { id } => Ok(serde_json::to_value(store.inspect(id, &owner)?)?),
                Command::List { offset } => Ok(serde_json::to_value(store.list(&owner, offset)?)?),
                Command::Cancel { id, digest } => Ok(serde_json::to_value(store.cancel(id, &digest, &owner)?)?),
                Command::Forget { id, digest } => { store.forget(id, &digest, &owner)?; Ok(serde_json::json!({"forgotten":id,"notice":"Local maintenance does not prove an operation was unsent or authorize repetition."})) }
                _ => unreachable!(),
            }
        }).await?;
        context.policy.check_current()?;
        result.display =
            serde_json::to_string_pretty(&super::redact_value(&value, &context.redactor))?;
        return Ok(result);
    }
    if matches!(args.command, Command::Remotes) {
        result.display =
            serde_json::to_string_pretty(&super::repository::discover(&context).await?)?;
        return Ok(result);
    }
    let service = Service::new(context.clone(), session_id)?;
    let value = match args.command {
        Command::Auth => {
            serde_json::json!({"host":"github.com","credential_source":"explicitly delegated HELM_GITHUB_TOKEN","actor":service.actor().await?})
        }
        Command::Logs { url, job } => {
            serde_json::to_value(service.logs(Object::parse(&url)?, job).await?)?
        }
        Command::View(view) => serde_json::to_value(service.read(view.read()?).await?)?,
        Command::Continue { request } => {
            ensure!(
                request.len() <= 16 * 1024,
                "GitHub continuation exceeds limit"
            );
            let request: super::context::Read = serde_json::from_str(&request)
                .map_err(|_| anyhow::anyhow!("invalid GitHub continuation"))?;
            serde_json::to_value(service.read(request).await?)?
        }
        Command::Reference { url } => {
            ensure!(
                session_id.is_some(),
                "a GitHub reference needs an owning voyage"
            );
            let page = service
                .read(super::context::Read {
                    object: Object::parse(&url)?,
                    section: super::context::Section::Details,
                    page: 1,
                    expected_head: None,
                    expected_base: None,
                })
                .await?;
            let reference = Reference {
                object: page.object,
                head: page.head,
                fetched_at: page.fetched_at,
            };
            reference.validate()?;
            result.reference = Some(reference.clone());
            serde_json::json!({"reference":reference})
        }
        Command::Feedback { view, id } => {
            ensure!(
                session_id.is_some(),
                "GitHub feedback import needs an owning voyage"
            );
            let request = view.read()?;
            ensure!(
                matches!(
                    request.section,
                    super::context::Section::Details
                        | super::context::Section::Comments
                        | super::context::Section::Reviews
                        | super::context::Section::ReviewComments { .. }
                ),
                "select issue or review feedback content"
            );
            let page = service.read(request).await?;
            let entry = if page.section == super::context::Section::Details {
                ensure!(id.is_none(), "issue body import does not take a comment ID");
                &page.data
            } else {
                let id =
                    id.ok_or_else(|| anyhow::anyhow!("comment or review feedback needs --id ID"))?;
                page.data["items"]
                    .as_array()
                    .and_then(|items| items.iter().find(|item| item["id"].as_u64() == Some(id)))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "feedback ID is absent from this page; follow its continuation"
                        )
                    })?
            };
            let mut feedback = attributed_feedback(&page.object, entry, page.fetched_at, id)?;
            ensure!(
                service.redact(&feedback.source) == feedback.source,
                "GitHub feedback source contains a configured secret"
            );
            feedback.title = service.redact(&feedback.title);
            feedback.description = service.redact(&feedback.description);
            feedback.validate()?;
            result.reference = Some(Reference {
                object: page.object,
                head: page.head,
                fetched_at: page.fetched_at,
            });
            let value = serde_json::json!({"source":feedback.source,"title":feedback.title,"description":feedback.description,"status":"pending local import"});
            result.feedback = Some(feedback);
            value
        }
        Command::Prepare(args) => {
            let draft = if let Some(path) = args.draft_file {
                ensure!(
                    args.url.is_none() && args.event.is_none() && args.commit.is_none(),
                    "full draft file cannot be combined with separate publication parameters"
                );
                serde_json::from_str::<Draft>(&read_input(&context, path).await?)
                    .map_err(|_| anyhow::anyhow!("invalid GitHub draft JSON"))?
            } else {
                let object =
                    Object::parse(args.url.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("GitHub preparation needs an object URL")
                    })?)?;
                let body = match (args.body, args.body_file) {
                    (Some(body), None) => body,
                    (None, Some(path)) => read_input(&context, path).await?,
                    _ => anyhow::bail!(
                        "GitHub preparation requires exactly one --body or --body-file"
                    ),
                };
                let action = if let Some(event) = args.event {
                    let event = match event.as_str() {
                        "COMMENT" => ReviewEvent::Comment,
                        "APPROVE" => ReviewEvent::Approve,
                        "REQUEST_CHANGES" => ReviewEvent::RequestChanges,
                        _ => anyhow::bail!("invalid GitHub review event"),
                    };
                    Action::Review {
                        event,
                        commit_id: args.commit.ok_or_else(|| {
                            anyhow::anyhow!("GitHub review requires exact --commit SHA")
                        })?,
                        body,
                        comments: Vec::new(),
                    }
                } else {
                    ensure!(args.commit.is_none(), "--commit requires a review --event");
                    Action::Comment { body }
                };
                Draft { object, action }
            };
            serde_json::to_value(service.prepare(draft).await?)?
        }
        Command::Publish { id, digest } => {
            serde_json::to_value(service.publish(id, &digest).await?)?
        }
        Command::Reconcile {
            id,
            digest,
            remote_id,
        } => serde_json::to_value(service.reconcile(id, &digest, remote_id).await?)?,
        Command::Admin { .. } | Command::Dispose { .. } => unreachable!(),
        Command::Remotes
        | Command::Inspect { .. }
        | Command::List { .. }
        | Command::Cancel { .. }
        | Command::Forget { .. }
        | Command::References
        | Command::Unreference { .. } => unreachable!(),
    };
    result.display = service.project(&value, 2 * 1024 * 1024)?;
    context.policy.check_current()?;
    ensure!(
        !context.cancellation.is_cancelled(),
        "GitHub operator action cancelled; inspect receipts before repeating"
    );
    Ok(result)
}

async fn read_input(
    context: &crate::tools::ToolContext,
    path: std::path::PathBuf,
) -> Result<String> {
    let policy = context.policy.clone();
    let cancel = context.cancellation.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        policy.check_current()?;
        ensure!(!cancel.is_cancelled(), "GitHub input read cancelled");
        let path = policy.resolve_read(&path)?;
        let root = policy
            .effective()
            .rules()
            .read_roots
            .iter()
            .find(|root| path.starts_with(root))
            .ok_or_else(|| anyhow::anyhow!("GitHub input is outside readable roots"))?;
        let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
        let file = directory.open(path.strip_prefix(root)?)?;
        ensure!(
            file.metadata()?.is_file(),
            "GitHub input is not a regular file"
        );
        let mut bytes = Vec::new();
        file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 128 * 1024, "GitHub input exceeds limit");
        policy.check_current()?;
        ensure!(!cancel.is_cancelled(), "GitHub input read cancelled");
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("GitHub input must be UTF-8"))
    })
    .await
    .map_err(|_| anyhow::anyhow!("GitHub input reader failed"))?
}
