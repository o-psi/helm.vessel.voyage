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
        self.history_inner(
            scope,
            query,
            cancel,
            #[cfg(test)]
            None,
        )
    }
    fn history_inner(
        &self,
        scope: Scope,
        query: Query,
        cancel: &CancellationToken,
        #[cfg(test)] observer: Option<Box<dyn FnOnce()>>,
    ) -> Result<History> {
        query.validate()?;
        cancelled(cancel)?;
        let tx = self.connection.unchecked_transaction()?;
        // First read fixes the snapshot; counters and every attempt share it.
        let status = status(&tx, scope)?;
        #[cfg(test)]
        if let Some(observer) = observer {
            observer();
        }
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
pub(crate) fn display_json(value: &impl Serialize) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: tempfile::TempDir,
        store: Store,
        project: Uuid,
        session: Uuid,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let mut store = Store::open(root.path().join("history.sqlite3")).unwrap();
            let project = store.project(root.path()).unwrap();
            let session = Uuid::new_v4();
            store.bind_session(project, session).unwrap();
            Self {
                root,
                store,
                project,
                session,
            }
        }
        fn row(
            &mut self,
            at: &str,
            model: &str,
            input: Option<u64>,
            output: Option<u64>,
            outcome: AttemptOutcome,
        ) -> Uuid {
            let permit = self
                .store
                .admit(&Attribution {
                    session: self.session,
                    run: Uuid::new_v4(),
                    agent: None,
                    provider: "fixture".into(),
                    model: model.into(),
                    purpose: Purpose::Conversation,
                })
                .unwrap();
            let text: String = self
                .store
                .connection
                .query_row(
                    "SELECT record FROM attempts WHERE id=?1",
                    [permit.id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            let mut row: Attempt = serde_json::from_str(&text).unwrap();
            row.admitted_at = at.into();
            row.finished_at = (outcome != AttemptOutcome::Unknown).then(|| at.into());
            row.outcome = outcome;
            row.input_tokens = input;
            row.output_tokens = output;
            self.store
                .connection
                .execute(
                    "UPDATE attempts SET record=?2 WHERE id=?1",
                    params![permit.id.to_string(), serde_json::to_string(&row).unwrap()],
                )
                .unwrap();
            permit.id
        }
        fn read(&self, query: Query) -> History {
            self.store
                .history(
                    Scope::Project(self.project),
                    query,
                    &CancellationToken::new(),
                )
                .unwrap()
        }
    }
    fn query() -> Query {
        Query {
            from: parse_utc("2026-01-01T00:00:00Z").unwrap(),
            until: parse_utc("2026-01-02T00:00:00Z").unwrap(),
            group_by: GroupBy::Model,
            detail: None,
            offset: 0,
            limit: 100,
            snapshot: None,
        }
    }
    #[test]
    fn exact_utc_half_open_range_keeps_missing_sides_independent() {
        let mut f = Fixture::new();
        f.row(
            "2025-12-31T23:59:59.999999999Z",
            "old",
            Some(999),
            Some(999),
            AttemptOutcome::Completed,
        );
        f.row(
            "2026-01-01T00:00:00Z",
            "a",
            Some(0),
            None,
            AttemptOutcome::Unknown,
        );
        f.row(
            "2026-01-01T12:00:00+00:00",
            "a",
            None,
            Some(4),
            AttemptOutcome::Failed,
        );
        f.row(
            "2026-01-02T00:00:00Z",
            "new",
            Some(999),
            Some(999),
            AttemptOutcome::Completed,
        );
        let h = f.read(query());
        assert_eq!(h.range_permits, Some(2));
        assert_eq!(h.totals.retained_attempts, 2);
        assert_eq!(h.totals.unknown, 1);
        assert_eq!(h.totals.failed, 1);
        assert_eq!(
            h.totals.input,
            Tokens {
                reported_sum: Some(0),
                reported_attempts: 1,
                missing_attempts: 1
            }
        );
        assert_eq!(
            h.totals.output,
            Tokens {
                reported_sum: Some(4),
                reported_attempts: 1,
                missing_attempts: 1
            }
        );
        assert_eq!(h.scope_lifetime_permits, 4);
        assert_eq!(h.groups.len(), 1);
        assert!(parse_utc("2026-01-01T00:00:00-06:00").is_err());
        assert!(parse_utc("2026-01-01").is_err());
    }
    #[test]
    fn group_and_attempt_pages_are_stable_and_restart_verifiable() {
        let mut f = Fixture::new();
        for model in ["b", "a", "c", "a"] {
            f.row(
                "2026-01-01T10:00:00Z",
                model,
                None,
                None,
                AttemptOutcome::Unknown,
            );
        }
        let mut q = query();
        q.limit = 1;
        let first = f.read(q.clone());
        assert_eq!(first.matching_groups, 3);
        assert_eq!(
            first.groups[0].key,
            GroupKey::Model {
                provider: "fixture".into(),
                model: "a".into()
            }
        );
        let cursor = first.next.unwrap();
        q.offset = cursor.offset;
        q.snapshot = Some(cursor.snapshot);
        let reopened = Store::open(f.root.path().join("history.sqlite3")).unwrap();
        let second = reopened
            .history(Scope::Project(f.project), q, &CancellationToken::new())
            .unwrap();
        assert_eq!(
            second.groups[0].key,
            GroupKey::Model {
                provider: "fixture".into(),
                model: "b".into()
            }
        );
        let mut q = query();
        q.detail = Some(first.groups[0].key.clone());
        q.limit = 1;
        let a = f.read(q.clone());
        assert_eq!(a.totals.retained_attempts, 2);
        assert_eq!(a.attempts.len(), 1);
        assert!(a.groups.is_empty());
        let cursor = a.next.unwrap();
        q.offset = cursor.offset;
        q.snapshot = Some(cursor.snapshot);
        let b = f.read(q);
        assert!(a.attempts[0].sequence < b.attempts[0].sequence);
        assert!(b.next.is_none());
    }
    #[test]
    fn changed_reports_or_query_refuse_snapshot_continuations() {
        let mut f = Fixture::new();
        let id = f.row(
            "2026-01-01T10:00:00Z",
            "a",
            None,
            None,
            AttemptOutcome::Unknown,
        );
        f.row(
            "2026-01-01T11:00:00Z",
            "b",
            None,
            None,
            AttemptOutcome::Unknown,
        );
        let mut q = query();
        q.limit = 1;
        let old = f.read(q.clone());
        q.offset = 1;
        q.snapshot = Some(old.snapshot);
        f.store.report(id, Some(8), None).unwrap();
        assert!(
            f.store
                .history(Scope::Project(f.project), q, &CancellationToken::new())
                .is_err()
        );
        let mut q = query();
        q.snapshot = Some(f.read(q.clone()).snapshot);
        q.group_by = GroupBy::Session;
        assert!(
            f.store
                .history(Scope::Project(f.project), q, &CancellationToken::new())
                .is_err()
        );
    }
    #[test]
    fn omitted_lifetime_detail_never_becomes_zero_in_a_filtered_range() {
        let mut f = Fixture::new();
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            Some(1),
            None,
            AttemptOutcome::Unknown,
        );
        for scope in [Scope::Project(f.project), Scope::Session(f.session)] {
            f.store
                .connection
                .execute(
                    "UPDATE limits SET consumed=consumed+5,omitted=omitted+5 WHERE scope=?1",
                    [scope.key()],
                )
                .unwrap();
        }
        let mut q = query();
        q.from = parse_utc("2027-01-01T00:00:00Z").unwrap();
        q.until = parse_utc("2027-01-02T00:00:00Z").unwrap();
        let h = f.read(q);
        assert_eq!(h.range_permits, None);
        assert_eq!(h.scope_lifetime_omitted_details, 5);
        assert_eq!(h.totals.retained_attempts, 0);
        assert_eq!(h.totals.input.reported_sum, None);
    }
    #[test]
    fn malformed_identity_time_or_counter_and_token_overflow_fail_closed() {
        for field in ["id", "admitted_at", "finished_at"] {
            let mut f = Fixture::new();
            let id = f.row(
                "2026-01-01T10:00:00Z",
                "a",
                None,
                None,
                AttemptOutcome::Unknown,
            );
            let text: String = f
                .store
                .connection
                .query_row(
                    "SELECT record FROM attempts WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            let mut row: serde_json::Value = serde_json::from_str(&text).unwrap();
            row[field] = serde_json::json!("invalid");
            f.store
                .connection
                .execute(
                    "UPDATE attempts SET record=?2 WHERE id=?1",
                    params![id.to_string(), row.to_string()],
                )
                .unwrap();
            assert!(
                f.store
                    .history(
                        Scope::Project(f.project),
                        query(),
                        &CancellationToken::new()
                    )
                    .is_err()
            );
        }
        let mut f = Fixture::new();
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            Some(u64::MAX),
            None,
            AttemptOutcome::Unknown,
        );
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            Some(1),
            None,
            AttemptOutcome::Unknown,
        );
        assert!(
            f.store
                .history(
                    Scope::Project(f.project),
                    query(),
                    &CancellationToken::new()
                )
                .is_err()
        );
        let mut f = Fixture::new();
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            None,
            None,
            AttemptOutcome::Unknown,
        );
        f.store
            .connection
            .execute("DELETE FROM attempts", [])
            .unwrap();
        assert!(
            f.store
                .history(
                    Scope::Project(f.project),
                    query(),
                    &CancellationToken::new()
                )
                .is_err()
        );
    }
    #[test]
    fn all_group_dimensions_and_session_scope_keep_attribution() {
        let mut f = Fixture::new();
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            None,
            None,
            AttemptOutcome::Unknown,
        );
        for (by, key) in [
            (GroupBy::Session, GroupKey::Session { id: f.session }),
            (GroupBy::Agent, GroupKey::Agent { id: None }),
            (
                GroupBy::Purpose,
                GroupKey::Purpose {
                    name: "conversation".into(),
                },
            ),
            (
                GroupBy::Day,
                GroupKey::Day {
                    utc: "2026-01-01".into(),
                },
            ),
        ] {
            let mut q = query();
            q.group_by = by;
            let h = f
                .store
                .history(Scope::Session(f.session), q, &CancellationToken::new())
                .unwrap();
            assert_eq!(h.groups[0].key, key);
        }
    }
    #[test]
    fn snapshot_reader_does_not_mix_concurrent_counter_or_report_revisions() {
        let mut f = Fixture::new();
        f.row(
            "2026-01-01T10:00:00Z",
            "a",
            None,
            None,
            AttemptOutcome::Unknown,
        );
        f.store
            .connection
            .execute_batch("PRAGMA journal_mode=WAL")
            .unwrap();
        let path = f.root.path().join("history.sqlite3");
        let session = f.session;
        let before = f.read(query());
        let h = f
            .store
            .history_inner(
                Scope::Project(f.project),
                query(),
                &CancellationToken::new(),
                Some(Box::new(move || {
                    let mut writer = Store::open(path).unwrap();
                    writer
                        .admit(&Attribution {
                            session,
                            run: Uuid::new_v4(),
                            agent: None,
                            provider: "fixture".into(),
                            model: "other".into(),
                            purpose: Purpose::Title,
                        })
                        .unwrap();
                })),
            )
            .unwrap();
        assert_eq!(h.snapshot, before.snapshot);
        assert_eq!(h.scope_lifetime_permits, 1);
        assert_eq!(f.read(query()).scope_lifetime_permits, 2);
    }
    #[test]
    fn quoted_secret_metadata_is_refused_without_changing_group_identity() {
        let mut f = Fixture::new();
        let secret = "private-quoted\"credential";
        f.row(
            "2026-01-01T10:00:00Z",
            secret,
            Some(0),
            None,
            AttemptOutcome::Unknown,
        );
        let h = f.read(query());
        let exact = serde_json::to_value(&h).unwrap();
        let redactor = crate::tools::Redactor::new([secret.to_owned()]);
        assert!(
            !redactor.contains_secret(&serde_json::to_string(&h).unwrap()),
            "raw secret is hidden by JSON escaping"
        );
        assert!(ensure_display_safe(&h, &redactor).is_err());
        assert_eq!(serde_json::to_value(&h).unwrap(), exact);
        assert_eq!(
            h.groups[0].key,
            GroupKey::Model {
                provider: "fixture".into(),
                model: secret.into()
            }
        );
    }
    #[test]
    fn invisible_label_json_is_visible_and_exactly_round_trips() {
        let mut f = Fixture::new();
        let label = "label\u{061c}\u{202e}\u{200b}\u{2066}\u{feff}尾";
        f.row(
            "2026-01-01T10:00:00Z",
            label,
            None,
            None,
            AttemptOutcome::Unknown,
        );
        let h = f.read(query());
        let displayed = display_json(&h).unwrap();
        for ch in ['\u{061c}', '\u{202e}', '\u{200b}', '\u{2066}', '\u{feff}'] {
            assert!(!displayed.contains(ch));
            assert!(displayed.contains(&format!("\\u{:04x}", ch as u32)));
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&displayed).unwrap(),
            serde_json::to_value(&h).unwrap()
        );
        assert_eq!(
            h.groups[0].key,
            GroupKey::Model {
                provider: "fixture".into(),
                model: label.into()
            }
        );
    }

    #[test]
    fn reversed_wall_clock_keeps_exact_timestamps_without_inventing_duration() {
        let mut f = Fixture::new();
        let id = f.row(
            "2026-01-01T10:00:00Z",
            "a",
            Some(2),
            Some(3),
            AttemptOutcome::Completed,
        );
        let text: String = f
            .store
            .connection
            .query_row(
                "SELECT record FROM attempts WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut record: Attempt = serde_json::from_str(&text).unwrap();
        record.finished_at = Some("2026-01-01T09:00:00Z".into());
        f.store
            .connection
            .execute(
                "UPDATE attempts SET record=?2 WHERE id=?1",
                params![id.to_string(), serde_json::to_string(&record).unwrap()],
            )
            .unwrap();
        let mut q = query();
        q.detail = Some(GroupKey::Model {
            provider: "fixture".into(),
            model: "a".into(),
        });
        let h = f.read(q);
        assert_eq!(h.totals.completed, 1);
        assert_eq!(h.attempts[0].admitted_at, "2026-01-01T10:00:00Z");
        assert_eq!(
            h.attempts[0].finished_at.as_deref(),
            Some("2026-01-01T09:00:00Z")
        );
    }

    #[test]
    fn cancelled_query_returns_no_partial_history() {
        let f = Fixture::new();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        assert!(
            f.store
                .history_inner(
                    Scope::Project(f.project),
                    query(),
                    &cancel,
                    Some(Box::new(move || token.cancel()))
                )
                .is_err()
        );
    }
}
