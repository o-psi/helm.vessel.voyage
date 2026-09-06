//! Immutable publication parameters, separate from human approval and delivery.
use super::repository::{Object, ObjectKind};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InlineComment {
    pub path: String,
    pub line: u32,
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_side: Option<Side>,
    pub body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Comment {
        body: String,
    },
    Review {
        event: ReviewEvent,
        commit_id: String,
        body: String,
        #[serde(default)]
        comments: Vec<InlineComment>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub id: u64,
    pub login: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub object: Object,
    pub action: Action,
}
impl Draft {
    pub fn validate(&self) -> Result<()> {
        self.object.validate()?;
        let body = match &self.action {
            Action::Comment { body } => {
                ensure!(!body.trim().is_empty(), "GitHub comment body is empty");
                body
            }
            Action::Review {
                event,
                commit_id,
                body,
                comments,
            } => {
                ensure!(
                    self.object.kind == ObjectKind::PullRequest,
                    "review requires a pull request"
                );
                super::context::sha(&serde_json::Value::String(commit_id.clone()))?;
                ensure!(
                    *event == ReviewEvent::Approve || !body.trim().is_empty(),
                    "review body is required for this event"
                );
                ensure!(
                    comments.len() <= 100,
                    "inline review comment limit exceeded"
                );
                for comment in comments {
                    ensure!(
                        !comment.path.is_empty()
                            && comment.path.len() <= 1024
                            && !comment.path.starts_with('/')
                            && !comment.path.contains(['\\', '\0'])
                            && !comment
                                .path
                                .split('/')
                                .any(|part| part.is_empty() || matches!(part, "." | "..")),
                        "invalid inline review path"
                    );
                    ensure!(
                        comment.line > 0 && comment.line <= i32::MAX as u32,
                        "invalid inline review line"
                    );
                    ensure!(
                        comment.start_line.is_some() == comment.start_side.is_some(),
                        "multiline review requires both start line and side"
                    );
                    if let Some(start) = comment.start_line {
                        ensure!(
                            start > 0
                                && start < comment.line
                                && comment.start_side == Some(comment.side),
                            "unsupported inline review range"
                        );
                    }
                    ensure!(
                        !comment.body.trim().is_empty() && comment.body.len() <= 16 * 1024,
                        "inline review body is empty or exceeds limit"
                    );
                }
                body
            }
        };
        ensure!(
            body.len() <= 64 * 1024,
            "GitHub publication body exceeds limit"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= 120 * 1024,
            "GitHub publication exceeds total limit"
        );
        Ok(())
    }
    pub fn body(&self) -> serde_json::Value {
        match &self.action {
            Action::Comment { body } => serde_json::json!({"body":body}),
            Action::Review {
                event,
                commit_id,
                body,
                comments,
            } => {
                serde_json::json!({"event":event,"commit_id":commit_id,"body":body,"comments":comments})
            }
        }
    }
    pub fn digest(&self, actor: &Actor, policy_digest: &str) -> Result<String> {
        self.validate()?;
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&(
            1,
            self,
            actor,
            policy_digest,
        ))?)))
    }
    pub fn path(&self) -> String {
        format!(
            "/repos/{}/{}/{}/{}",
            self.object.repository.slug(),
            match self.action {
                Action::Comment { .. } => "issues",
                Action::Review { .. } => "pulls",
            },
            self.object.number,
            match self.action {
                Action::Comment { .. } => "comments",
                Action::Review { .. } => "reviews",
            }
        )
    }
}

/// Require every selected line to be present on the selected side of the diff.
/// An omitted patch cannot be replaced with guessed source-file coordinates.
pub(super) fn validate_inline(comment: &InlineComment, patch: &str) -> Result<()> {
    ensure!(patch.len() <= 2 * 1024 * 1024, "review patch exceeds limit");
    let mut left = None;
    let mut right = None;
    let mut present = std::collections::BTreeSet::new();
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            let mut fields = line.split_whitespace();
            fields.next();
            let start = |field: Option<&str>, marker| -> Result<u32> {
                field
                    .and_then(|field| field.strip_prefix(marker))
                    .and_then(|field| field.split(',').next())
                    .and_then(|field| field.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("review patch hunk is malformed"))
            };
            left = Some(start(fields.next(), '-')?);
            right = Some(start(fields.next(), '+')?);
            continue;
        }
        let (Some(old), Some(new)) = (left.as_mut(), right.as_mut()) else {
            continue;
        };
        match line.as_bytes().first() {
            Some(b' ') => {
                present.insert(match comment.side {
                    Side::Left => *old,
                    Side::Right => *new,
                });
                *old = old
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("review patch line overflow"))?;
                *new = new
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("review patch line overflow"))?;
            }
            Some(b'-') => {
                if comment.side == Side::Left {
                    present.insert(*old);
                }
                *old = old
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("review patch line overflow"))?;
            }
            Some(b'+') => {
                if comment.side == Side::Right {
                    present.insert(*new);
                }
                *new = new
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("review patch line overflow"))?;
            }
            Some(b'\\') => (),
            _ => anyhow::bail!("review patch is incomplete or malformed"),
        }
    }
    let start = comment.start_line.unwrap_or(comment.line);
    ensure!(
        comment.line.saturating_sub(start) <= 10_000
            && (start..=comment.line).all(|line| present.contains(&line)),
        "inline review coordinates are not fully present in the selected patch"
    );
    Ok(())
}
