//! Repository configuration is untrusted data, never a network destination.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub remote: String,
    pub repository: Option<Repository>,
    pub unavailable: bool,
}

/// Read Git's local configuration using fixed argv and existing command policy.
/// No hooks, shell, credential helpers, URL rewriting or network access are used.
pub async fn discover(context: &crate::tools::ToolContext) -> Result<Vec<Candidate>> {
    use tokio::io::AsyncReadExt;
    context.policy.check_current()?;
    let description = "git config --local --null --no-includes --get-regexp remote.url";
    match context.policy.command(description) {
        crate::policy::Decision::Deny(_) => anyhow::bail!(
            "GitHub remote discovery is denied by command policy; select an explicit object URL"
        ),
        crate::policy::Decision::Ask(reason) => {
            context
                .approver
                .approve(&context.approval("github.remotes", "local Git configuration", reason))
                .await
                .require_approved()?;
        }
        crate::policy::Decision::Allow => (),
    }
    context.policy.check_current()?;
    let mut command = tokio::process::Command::from(
        context
            .policy
            .process_command("git", context.policy.workspace())?,
    );
    command
        .args([
            "config",
            "--local",
            "--null",
            "--no-includes",
            "--get-regexp",
            "^remote\\..*\\.url$",
        ])
        .current_dir(context.policy.workspace())
        .env_clear()
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped());
    if let Some(path) = context.environment.get("PATH") {
        command.env("PATH", path);
    }
    context
        .policy
        .isolate_process(command.as_std_mut(), context.policy.workspace())?;
    let mut child = command.spawn().map_err(|_| {
        anyhow::anyhow!("GitHub remote discovery could not start Git; select an explicit URL")
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("GitHub remote discovery output unavailable"))?;
    let result = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => Err(anyhow::anyhow!("GitHub remote discovery cancelled")),
        result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut bytes = Vec::new();
            stdout.take(64 * 1024 + 1).read_to_end(&mut bytes).await?;
            ensure!(bytes.len() <= 64 * 1024, "GitHub remote configuration exceeds limit");
            let status = child.wait().await?;
            ensure!(status.success() || status.code() == Some(1), "GitHub remote discovery unavailable for this workspace; select an explicit URL");
            Ok(bytes)
        }) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("GitHub remote discovery deadline elapsed"))),
    };
    if result.is_err() {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
    }
    let bytes = result?;
    context.policy.check_current()?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("GitHub remotes contain unsupported encoding"))?;
    let mut candidates = Vec::new();
    for record in text.split('\0').filter(|record| !record.is_empty()) {
        ensure!(candidates.len() < 64, "GitHub remote count exceeds limit");
        let (key, value) = record
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("Git returned malformed remote configuration"))?;
        let name = key
            .strip_prefix("remote.")
            .and_then(|name| name.strip_suffix(".url"))
            .ok_or_else(|| anyhow::anyhow!("Git returned unexpected configuration key"))?;
        ensure!(
            !name.is_empty() && name.len() <= 128,
            "Git remote name exceeds limit"
        );
        let repository = Repository::from_remote(value).ok();
        candidates.push(Candidate {
            remote: context.redactor.redact(name),
            unavailable: repository.is_none(),
            repository,
        });
    }
    Ok(candidates)
}

impl Repository {
    pub fn new(owner: &str, name: &str) -> Result<Self> {
        ensure!(
            !owner.is_empty()
                && owner.len() <= 39
                && owner
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && owner.as_bytes()[0].is_ascii_alphanumeric()
                && owner.as_bytes()[owner.len() - 1].is_ascii_alphanumeric()
                && !owner.contains("--"),
            "invalid GitHub repository owner"
        );
        ensure!(
            !name.is_empty()
                && name.len() <= 100
                && !matches!(name, "." | "..")
                && !name.starts_with('-')
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "invalid GitHub repository name"
        );
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            Self::new(&self.owner, &self.name)? == *self,
            "noncanonical GitHub repository"
        );
        Ok(())
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
    pub fn url(&self) -> String {
        format!("https://github.com/{}", self.slug())
    }

    /// Only literal github.com is supported; aliases cannot select credential hosts.
    pub fn from_remote(remote: &str) -> Result<Self> {
        ensure!(
            remote.len() <= 512
                && !remote.contains(['%', '\\'])
                && !remote.chars().any(char::is_whitespace)
                && !remote.split('/').any(|part| matches!(part, "." | "..")),
            "invalid GitHub remote"
        );
        let path = if let Some(path) = remote.strip_prefix("git@github.com:") {
            path.to_owned()
        } else {
            let url = reqwest::Url::parse(remote)
                .map_err(|_| anyhow::anyhow!("invalid GitHub remote"))?;
            ensure!(
                url.host_str() == Some("github.com")
                    && url.port().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url.password().is_none(),
                "unsupported GitHub remote host or credentials"
            );
            ensure!(
                (url.scheme() == "https" && url.username().is_empty())
                    || (url.scheme() == "ssh" && url.username() == "git"),
                "unsupported GitHub remote scheme or credentials"
            );
            url.path()
                .strip_prefix('/')
                .unwrap_or(url.path())
                .to_owned()
        };
        let path = path.strip_suffix('/').unwrap_or(&path);
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("GitHub remote needs owner/repository"))?;
        Self::new(owner, name)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Issue,
    PullRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub repository: Repository,
    pub kind: ObjectKind,
    pub number: u64,
}
impl Object {
    pub fn validate(&self) -> Result<()> {
        self.repository.validate()?;
        ensure!(
            self.number > 0 && self.number <= i32::MAX as u64,
            "invalid GitHub object number"
        );
        Ok(())
    }
    pub fn url(&self) -> String {
        format!(
            "{}/{}/{}",
            self.repository.url(),
            match self.kind {
                ObjectKind::Issue => "issues",
                ObjectKind::PullRequest => "pull",
            },
            self.number
        )
    }
    pub fn parse(input: &str) -> Result<Self> {
        ensure!(
            input.len() <= 512
                && !input.contains(['%', '\\'])
                && !input.chars().any(char::is_whitespace)
                && !input.split('/').any(|part| matches!(part, "." | "..")),
            "invalid GitHub object URL"
        );
        let url =
            reqwest::Url::parse(input).map_err(|_| anyhow::anyhow!("invalid GitHub object URL"))?;
        ensure!(
            url.scheme() == "https"
                && url.host_str() == Some("github.com")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "unsupported GitHub object URL"
        );
        let parts = url
            .path()
            .trim_start_matches('/')
            .split('/')
            .collect::<Vec<_>>();
        ensure!(
            parts.len() == 4,
            "GitHub object URL needs owner/repository/type/number"
        );
        let kind = match parts[2] {
            "issues" => ObjectKind::Issue,
            "pull" => ObjectKind::PullRequest,
            _ => anyhow::bail!("unsupported GitHub object type"),
        };
        let object = Self {
            repository: Repository::new(parts[0], parts[1])?,
            kind,
            number: parts[3]
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid GitHub object number"))?,
        };
        object.validate()?;
        Ok(object)
    }
}
