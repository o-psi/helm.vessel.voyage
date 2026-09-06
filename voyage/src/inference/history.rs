//! Read-only historical projections from one SQLite snapshot.
use super::*;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Session,
    #[default]
    Model,
    Agent,
    Purpose,
    Day,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GroupKey {
    Session { id: Uuid },
    Model { provider: String, model: String },
    Agent { id: Option<Uuid> },
    Purpose { name: String },
    Day { utc: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub from: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub group_by: GroupBy,
    pub detail: Option<GroupKey>,
    pub offset: u32,
    pub limit: u32,
    pub snapshot: Option<String>,
}
impl Query {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.from < self.until
                && (1..=100).contains(&self.limit)
                && self.offset <= MAX_DETAILS as u32,
            Failure::Invalid
        );
        ensure!(
            self.offset == 0 || self.snapshot.is_some(),
            Failure::Invalid
        );
        ensure!(
            self.snapshot
                .as_ref()
                .is_none_or(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            Failure::Invalid
        );
        if let Some(key) = &self.detail {
            ensure!(
                matches!(
                    (self.group_by, key),
                    (GroupBy::Session, GroupKey::Session { .. })
                        | (GroupBy::Model, GroupKey::Model { .. })
                        | (GroupBy::Agent, GroupKey::Agent { .. })
                        | (GroupBy::Purpose, GroupKey::Purpose { .. })
                        | (GroupBy::Day, GroupKey::Day { .. })
                ),
                Failure::Invalid
            );
            ensure!(serde_json::to_vec(key)?.len() <= 1024, Failure::Invalid);
        }
        Ok(())
    }
}
/// CLI dates must explicitly name UTC; offset-local input is never guessed.
pub fn parse_utc(text: &str) -> std::result::Result<DateTime<Utc>, String> {
    if !(text.ends_with('Z') || text.ends_with("+00:00")) {
        return Err("use an explicit UTC RFC3339 timestamp ending Z or +00:00".into());
    }
    DateTime::parse_from_rfc3339(text)
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| "invalid UTC RFC3339 timestamp".into())
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub reported_sum: Option<u64>,
    pub reported_attempts: u64,
    pub missing_attempts: u64,
}
impl Tokens {
    fn add(&mut self, value: Option<u64>) -> Result<()> {
        if let Some(value) = value {
            self.reported_sum = Some(
                self.reported_sum
                    .unwrap_or(0)
                    .checked_add(value)
                    .ok_or(Failure::Invalid)?,
            );
            self.reported_attempts += 1;
        } else {
            self.missing_attempts += 1;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Totals {
    pub retained_attempts: u64,
    pub completed: u64,
    pub failed: u64,
    pub unknown: u64,
    pub input: Tokens,
    pub output: Tokens,
}
impl Totals {
    fn add(&mut self, attempt: &Attempt) -> Result<()> {
        self.retained_attempts += 1;
        match attempt.outcome {
            AttemptOutcome::Completed => self.completed += 1,
            AttemptOutcome::Failed => self.failed += 1,
            AttemptOutcome::Unknown => self.unknown += 1,
        }
        self.input.add(attempt.input_tokens)?;
        self.output.add(attempt.output_tokens)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub key: GroupKey,
    pub totals: Totals,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cursor {
    pub snapshot: String,
    pub offset: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub schema_version: u32,
    pub scope: Scope,
    pub query: Query,
    pub snapshot: String,
    pub scope_lifetime_permits: u64,
    pub scope_lifetime_omitted_details: u64,
    /// Unknown if any omitted detail cannot be assigned to this time/filter range.
    pub range_permits: Option<u64>,
    pub totals: Totals,
    pub groups: Vec<Group>,
    pub attempts: Vec<Attempt>,
    pub matching_groups: u64,
    pub next: Option<Cursor>,
}
fn key(attempt: &Attempt, at: DateTime<Utc>, by: GroupBy) -> GroupKey {
    let a = &attempt.attribution;
    match by {
        GroupBy::Session => GroupKey::Session { id: a.session },
        GroupBy::Model => GroupKey::Model {
            provider: a.provider.clone(),
            model: a.model.clone(),
        },
        GroupBy::Agent => GroupKey::Agent { id: a.agent },
        GroupBy::Purpose => GroupKey::Purpose {
            name: match a.purpose {
                Purpose::Conversation => "conversation",
                Purpose::Title => "title",
            }
            .into(),
        },
        GroupBy::Day => GroupKey::Day {
            utc: at.format("%Y-%m-%d").to_string(),
        },
    }
}
fn cancelled(cancel: &CancellationToken) -> Result<()> {
    ensure!(!cancel.is_cancelled(), Failure::Unavailable);
    Ok(())
}
impl Store {
    pub fn history(
        &self,
        scope: Scope,
        query: Query,
        cancel: &CancellationToken,
    ) -> Result<History> {
        self.history_inner(scope, query, cancel)
    }
    fn history_inner(
        &self,
        scope: Scope,
        query: Query,
        cancel: &CancellationToken,
    ) -> Result<History> {
        query.validate()?;
        cancelled(cancel)?;
        let tx = self.connection.unchecked_transaction()?;
        // First read fixes the snapshot; counters and every attempt share it.
        let status = status(&tx, scope)?;

        let (column, id) = match scope {
            Scope::Project(id) => ("project", id),
            Scope::Session(id) => ("session", id),
        };
        let sql = format!(
            "SELECT sequence,id,project,session,CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM attempts WHERE {column}=?1 ORDER BY sequence LIMIT 20001"
        );
        let mut statement = tx.prepare(&sql)?;
        let mut rows = statement.query([id.to_string()])?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&(
            scope,
            query.from,
            query.until,
            query.group_by,
            &query.detail,
        ))?);
        digest.update(serde_json::to_vec(&status)?);
        let mut groups: BTreeMap<GroupKey, Totals> = BTreeMap::new();
        let mut totals = Totals::default();
        let mut attempts = Vec::new();
        let mut retained = 0u64;
        let mut matched = 0u64;
        let mut previous = 0;
        while let Some(row) = rows.next()? {
            cancelled(cancel)?;
            retained += 1;
            ensure!(retained <= MAX_DETAILS as u64, Failure::Capacity);
            let sequence = counter(row.get(0)?)?;
            ensure!(sequence > previous, Failure::Invalid);
            previous = sequence;
            let mut attempt: Attempt = decode(row.get(4)?)?;
            ensure!(
                attempt.id.to_string() == row.get::<_, String>(1)?
                    && attempt.project.to_string() == row.get::<_, String>(2)?
                    && attempt.attribution.session.to_string() == row.get::<_, String>(3)?,
                Failure::Invalid
            );
            ensure!(
                match scope {
                    Scope::Project(id) => attempt.project == id,
                    Scope::Session(id) => attempt.attribution.session == id,
                },
                Failure::Invalid
            );
            ensure!(
                !attempt.id.is_nil()
                    && !attempt.project.is_nil()
                    && !attempt.attribution.session.is_nil()
                    && !attempt.attribution.run.is_nil()
                    && attempt.attribution.agent.is_none_or(|id| !id.is_nil()),
                Failure::Invalid
            );
            for label in [&attempt.attribution.provider, &attempt.attribution.model] {
                ensure!(
                    !label.is_empty() && label.len() <= 256 && !label.chars().any(char::is_control),
                    Failure::Invalid
                );
            }
            let at = DateTime::parse_from_rfc3339(&attempt.admitted_at)
                .map_err(|_| Failure::Invalid)?
                .with_timezone(&Utc);
            let finish = attempt
                .finished_at
                .as_deref()
                .map(DateTime::parse_from_rfc3339)
                .transpose()
                .map_err(|_| Failure::Invalid)?;
            ensure!(
                (attempt.outcome == AttemptOutcome::Unknown) == finish.is_none(),
                Failure::Invalid
            );
            // These are independent wall-clock observations, not monotonic
            // durations; clock correction can put completion before admission.
            attempt.sequence = sequence;
            digest.update(serde_json::to_vec(&attempt)?);
            if at < query.from || at >= query.until {
                continue;
            }
            let group = key(&attempt, at, query.group_by);
            if query.detail.as_ref().is_some_and(|wanted| *wanted != group) {
                continue;
            }
            totals.add(&attempt)?;
            groups.entry(group).or_default().add(&attempt)?;
            if query.detail.is_some() {
                if matched >= u64::from(query.offset) && attempts.len() < (query.limit as usize) {
                    attempts.push(attempt);
                }
                matched += 1;
            }
        }
        ensure!(
            retained.checked_add(status.omitted_attempt_details) == Some(status.consumed),
            Failure::Invalid
        );
        let snapshot = hex::encode(digest.finalize());
        ensure!(
            query
                .snapshot
                .as_ref()
                .is_none_or(|expected| *expected == snapshot),
            Failure::HistoryChanged
        );
        let matching_groups = groups.len() as u64;
        let count = if query.detail.is_some() {
            matched
        } else {
            matching_groups
        };
        ensure!(u64::from(query.offset) <= count, Failure::Invalid);
        let groups = if query.detail.is_some() {
            Vec::new()
        } else {
            groups
                .into_iter()
                .skip(query.offset as usize)
                .take(query.limit as usize)
                .map(|(key, totals)| Group { key, totals })
                .collect()
        };
        let end = u64::from(query.offset) + u64::from(query.limit);
        let next = (end < count).then(|| Cursor {
            snapshot: snapshot.clone(),
            offset: end as u32,
        });
        cancelled(cancel)?;
        Ok(History {
            schema_version: 1,
            scope,
            query,
            snapshot,
            scope_lifetime_permits: status.consumed,
            scope_lifetime_omitted_details: status.omitted_attempt_details,
            range_permits: (status.omitted_attempt_details == 0)
                .then_some(totals.retained_attempts),
            totals,
            groups,
            attempts,
            matching_groups,
            next,
        })
    }
}

/// Visible JSON escaping preserves exact decoded keys and avoids terminal
/// direction/zero-width formatting in both structured CLI and TUI projections.
pub fn display_json(value: &impl Serialize) -> Result<String> {
    let text = serde_json::to_string_pretty(value)?;
    Ok(text.chars().flat_map(|ch| {
        if matches!(ch,'\u{061c}'|'\u{200b}'..='\u{200f}'|'\u{2028}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{feff}') {
            format!("\\u{:04x}",ch as u32).chars().collect::<Vec<_>>()
        } else {vec![ch]}
    }).collect())
}

/// Refuse secret-bearing metadata rather than changing immutable group keys.
pub fn ensure_display_safe(history: &History, redactor: &crate::tools::Redactor) -> Result<()> {
    fn contains(value: &serde_json::Value, redactor: &crate::tools::Redactor) -> bool {
        match value {
            serde_json::Value::String(text) => redactor.contains_secret(text),
            serde_json::Value::Array(items) => items.iter().any(|item| contains(item, redactor)),
            serde_json::Value::Object(items) => items
                .iter()
                .any(|(key, value)| redactor.contains_secret(key) || contains(value, redactor)),
            _ => false,
        }
    }
    ensure!(
        !contains(&serde_json::to_value(history)?, redactor),
        Failure::Unavailable
    );
    Ok(())
}
