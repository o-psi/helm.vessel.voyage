//! Typed, resumable observations. Remote text is task data, never runtime guidance.
use super::{
    repository::{Object, ObjectKind},
    transport::Client,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Section {
    Details,
    Comments,
    Reviews,
    Files,
    ReviewComments {
        review: u64,
    },
    Statuses,
    Checks,
    CheckSuites,
    SuiteChecks {
        suite: u64,
    },
    WorkflowRuns,
    Jobs {
        run: u64,
    },
    Annotations {
        check: u64,
    },
    Threads {
        after: Option<String>,
    },
    ThreadComments {
        thread: String,
        after: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub object: Object,
    pub section: Section,
    #[serde(default = "first_page")]
    pub page: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_base: Option<Base>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Base {
    pub sha: String,
    pub repository: u64,
    pub reference: String,
}
impl Base {
    pub fn validate(&self) -> Result<()> {
        sha(&Value::String(self.sha.clone()))?;
        positive(self.repository)?;
        ensure!(
            !self.reference.is_empty()
                && self.reference.len() <= 1024
                && !self.reference.chars().any(char::is_control),
            "invalid pull request base reference"
        );
        Ok(())
    }
}
pub(super) fn base(detail: &Value) -> Result<Base> {
    let base = Base {
        sha: sha(&detail["base"]["sha"])?,
        repository: detail["base"]["repo"]["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("pull request base repository identity is missing"))?,
        reference: detail["base"]["ref"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("pull request base branch is missing"))?
            .into(),
    };
    base.validate()?;
    Ok(base)
}
fn first_page() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub object: Object,
    pub section: Section,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
    pub head: Option<String>,
    pub base: Option<Base>,
    pub page: u32,
    pub next: Option<Read>,
    pub incomplete: Vec<String>,
    pub data: Value,
}

fn positive(id: u64) -> Result<()> {
    ensure!(
        id > 0 && id <= i64::MAX as u64,
        "invalid GitHub resource ID"
    );
    Ok(())
}
pub(super) fn sha(value: &Value) -> Result<String> {
    let value = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("GitHub returned no commit identity"))?;
    ensure!(
        value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit()),
        "GitHub returned invalid commit identity"
    );
    Ok(value.to_ascii_lowercase())
}

impl Read {
    pub fn validate(&self) -> Result<()> {
        self.object.validate()?;
        if let Some(head) = &self.expected_head {
            ensure!(
                self.object.kind == ObjectKind::PullRequest,
                "only pull request continuations bind a head"
            );
            sha(&Value::String(head.clone()))?;
        }
        if let Some(base) = &self.expected_base {
            ensure!(
                self.object.kind == ObjectKind::PullRequest,
                "base continuation requires a pull request"
            );
            base.validate()?;
        }
        ensure!(
            self.page > 0 && self.page <= 1_000_000,
            "invalid GitHub page"
        );
        if self.section == Section::Details {
            ensure!(self.page == 1, "details have no page continuation");
        }
        if !matches!(self.section, Section::Details | Section::Comments) {
            ensure!(
                self.object.kind == ObjectKind::PullRequest,
                "this context requires a pull request"
            );
        }
        match self.section {
            Section::ReviewComments { review } => positive(review)?,
            Section::Annotations { check } => positive(check)?,
            Section::SuiteChecks { suite } => positive(suite)?,
            Section::Jobs { run } => positive(run)?,
            _ => (),
        }
        match &self.section {
            Section::Threads { after } | Section::ThreadComments { after, .. } => {
                ensure!(self.page == 1, "GraphQL pagination uses cursors");
                if let Some(cursor) = after {
                    opaque(cursor)?;
                }
                if let Section::ThreadComments { thread, .. } = &self.section {
                    opaque(thread)?;
                }
            }
            _ => (),
        }
        if self.section == Section::Files {
            ensure!(
                self.page <= 30,
                "GitHub files API exposes at most 3000 files"
            );
        }
        if self.section == Section::WorkflowRuns {
            ensure!(
                self.page <= 10,
                "GitHub filtered workflow runs expose at most 1000 results"
            );
        }
        Ok(())
    }
}

pub(super) async fn details(
    client: &Client,
    object: &Object,
    cancel: &CancellationToken,
) -> Result<Value> {
    object.validate()?;
    let route = match object.kind {
        ObjectKind::Issue => "issues",
        ObjectKind::PullRequest => "pulls",
    };
    let value = client
        .get(
            &format!(
                "/repos/{}/{route}/{}",
                object.repository.slug(),
                object.number
            ),
            cancel,
        )
        .await?
        .json()?;
    ensure!(
        value.get("number").and_then(Value::as_u64) == Some(object.number),
        "GitHub returned a different object"
    );
    ensure!(
        value
            .get("html_url")
            .and_then(Value::as_str)
            .is_some_and(|url| url.eq_ignore_ascii_case(&object.url())),
        "GitHub returned a different object URL"
    );
    if object.kind == ObjectKind::Issue {
        ensure!(
            value.get("pull_request").is_none(),
            "this issue is a pull request; select its pull URL explicitly"
        );
    } else {
        sha(&value["head"]["sha"])?;
        sha(&value["base"]["sha"])?;
    }
    Ok(value)
}

pub(super) async fn read(
    client: &Client,
    request: Read,
    cancel: &CancellationToken,
) -> Result<Page> {
    request.validate()?;
    let detail = details(client, &request.object, cancel).await?;
    let head = if request.object.kind == ObjectKind::PullRequest {
        Some(sha(&detail["head"]["sha"])?)
    } else {
        None
    };
    let observed_base = if request.object.kind == ObjectKind::PullRequest {
        Some(base(&detail)?)
    } else {
        None
    };
    ensure!(
        request
            .expected_head
            .as_ref()
            .is_none_or(|expected| Some(expected) == head.as_ref()),
        "pull request continuation is stale; reload from its first page"
    );
    ensure!(
        request
            .expected_base
            .as_ref()
            .is_none_or(|expected| Some(expected) == observed_base.as_ref()),
        "pull request base changed; reload from its first page"
    );
    let mut page = Page {
        object: request.object.clone(),
        section: request.section.clone(),
        fetched_at: chrono::Utc::now(),
        head: head.clone(),
        base: observed_base.clone(),
        page: request.page,
        next: None,
        incomplete: Vec::new(),
        data: Value::Null,
    };
    if request.section == Section::Details {
        page.data = detail;
        return Ok(page);
    }
    if matches!(
        request.section,
        Section::Threads { .. } | Section::ThreadComments { .. }
    ) {
        return threads(client, request, page, cancel).await;
    }
    let slug = request.object.repository.slug();
    let number = request.object.number;
    let path = match request.section {
        Section::Details => unreachable!(),
        Section::Comments => format!("/repos/{slug}/issues/{number}/comments"),
        Section::Reviews => format!("/repos/{slug}/pulls/{number}/reviews"),
        Section::Files => format!("/repos/{slug}/pulls/{number}/files"),
        Section::ReviewComments { review } => {
            format!("/repos/{slug}/pulls/{number}/reviews/{review}/comments")
        }
        Section::Statuses => format!(
            "/repos/{slug}/commits/{}/statuses",
            head.as_deref().unwrap_or_default()
        ),
        Section::Checks => format!(
            "/repos/{slug}/commits/{}/check-runs",
            head.as_deref().unwrap_or_default()
        ),
        Section::CheckSuites => format!(
            "/repos/{slug}/commits/{}/check-suites",
            head.as_deref().unwrap_or_default()
        ),
        Section::SuiteChecks { suite } => {
            let suite_data = client
                .get(&format!("/repos/{slug}/check-suites/{suite}"), cancel)
                .await?
                .json()?;
            ensure!(
                sha(&suite_data["head_sha"])? == head.as_deref().unwrap_or_default(),
                "check suite does not belong to the selected head"
            );
            format!("/repos/{slug}/check-suites/{suite}/check-runs")
        }
        Section::WorkflowRuns => format!("/repos/{slug}/actions/runs"),
        Section::Jobs { run } => {
            let run_data = client
                .get(&format!("/repos/{slug}/actions/runs/{run}"), cancel)
                .await?
                .json()?;
            ensure!(
                sha(&run_data["head_sha"])? == head.as_deref().unwrap_or_default(),
                "workflow run does not belong to the selected head"
            );
            format!("/repos/{slug}/actions/runs/{run}/jobs")
        }
        Section::Annotations { check } => {
            let check_data = client
                .get(&format!("/repos/{slug}/check-runs/{check}"), cancel)
                .await?
                .json()?;
            ensure!(
                sha(&check_data["head_sha"])? == head.as_deref().unwrap_or_default(),
                "check does not belong to the selected pull request head"
            );
            format!("/repos/{slug}/check-runs/{check}/annotations")
        }
        Section::Threads { .. } | Section::ThreadComments { .. } => unreachable!(),
    };
    let extra = match request.section {
        Section::Checks | Section::SuiteChecks { .. } | Section::Jobs { .. } => {
            "&filter=all".to_owned()
        }
        Section::WorkflowRuns => format!("&head_sha={}", head.as_deref().unwrap_or_default()),
        _ => String::new(),
    };
    let response = client
        .get(
            &format!("{path}?per_page=100&page={}{extra}", request.page),
            cancel,
        )
        .await?;
    let next = next_page(&response.headers, request.page, &path, &extra)?;
    let mut data = response.json()?;
    let total = data.get("total_count").and_then(Value::as_u64);
    let wrapper = match request.section {
        Section::Checks | Section::SuiteChecks { .. } => Some("check_runs"),
        Section::CheckSuites => Some("check_suites"),
        Section::WorkflowRuns => Some("workflow_runs"),
        Section::Jobs { .. } => Some("jobs"),
        _ => None,
    };
    if let Some(wrapper) = wrapper {
        data = data
            .get_mut(wrapper)
            .map(Value::take)
            .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed result collection"))?;
    }
    if request.section == Section::Checks {
        page.incomplete.push("Reference check results cover only the latest 1000 check suites; older suites are not proven absent.".into());
    }
    let entries = data
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed collection"))?;
    ensure!(entries.len() <= 100, "GitHub collection exceeds page limit");
    let coverage_total = if request.section == Section::Files {
        detail["changed_files"].as_u64()
    } else {
        total
    };
    if (wrapper.is_some() || request.section == Section::Files)
        && !collection_coverage_consistent(
            coverage_total,
            request.page,
            entries.len(),
            next.is_some(),
        )
    {
        page.incomplete.push("Reported collection totals disagree with available pages, or are missing; complete coverage is not established.".into());
    }
    if request.section == Section::WorkflowRuns && total.is_none_or(|count| count >= 1000) {
        page.incomplete.push("GitHub filtered workflow searches expose at most 1000 runs; total coverage is incomplete or unknown.".into());
    }
    if request.section == Section::Files {
        if detail["changed_files"]
            .as_u64()
            .is_none_or(|count| count >= 3000)
        {
            page.incomplete.push("GitHub exposes at most 3000 changed files; total file coverage is incomplete or unknown.".into());
        }
        if entries
            .iter()
            .any(|file| file.get("patch").and_then(Value::as_str).is_none())
        {
            page.incomplete.push(
                "Some patches are omitted or binary; file metadata is not the complete diff."
                    .into(),
            );
        }
        if entries
            .iter()
            .filter(|file| file.get("patch").is_some())
            .any(|file| {
                file["patch"]
                    .as_str()
                    .and_then(|patch| super::publication::parse_patch(patch).ok())
                    .is_none_or(|parsed| {
                        file["additions"].as_u64() != Some(parsed.additions)
                            || file["deletions"].as_u64() != Some(parsed.deletions)
                    })
            })
        {
            page.incomplete.push("Some returned patches are malformed, truncated, or disagree with reported changed-line totals; complete diff coverage is not established.".into());
        }
    }
    if let Some(next) = next
        && (request.section != Section::Files || next <= 30)
        && (request.section != Section::WorkflowRuns || next <= 10)
    {
        page.next = Some(Read {
            page: next,
            expected_head: page.head.clone(),
            expected_base: page.base.clone(),
            ..request.clone()
        });
    }
    if let Some(head) = head {
        let current = details(client, &request.object, cancel).await?;
        if sha(&current["head"]["sha"])? != head {
            page.incomplete.push("Pull request head changed during observation; reload before reviewing or publishing.".into());
        }
        if Some(base(&current)?) != observed_base {
            page.incomplete.push("Pull request base changed during observation; reload before reviewing or publishing.".into());
        }
    }
    let mut nested = Vec::new();
    for entry in entries {
        let section = match request.section {
            Section::CheckSuites => Some(Section::SuiteChecks {
                suite: entry["id"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("GitHub check suite ID is missing"))?,
            }),
            Section::Checks | Section::SuiteChecks { .. }
                if entry["output"]["annotations_count"]
                    .as_u64()
                    .is_some_and(|count| count > 0) =>
            {
                Some(Section::Annotations {
                    check: entry["id"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("GitHub check ID is missing"))?,
                })
            }
            Section::WorkflowRuns => Some(Section::Jobs {
                run: entry["id"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("GitHub workflow run ID is missing"))?,
            }),
            _ => None,
        };
        if let Some(section) = section {
            nested.push(Read {
                object: request.object.clone(),
                section,
                page: 1,
                expected_head: page.head.clone(),
                expected_base: page.base.clone(),
            });
        }
    }
    page.data =
        serde_json::json!({"items":data,"reported_total":total,"nested_continuations":nested});
    Ok(page)
}

fn opaque(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 1024
            && value.bytes().all(|byte| byte.is_ascii_graphic()),
        "invalid GitHub cursor or node identity"
    );
    Ok(())
}

async fn threads(
    client: &Client,
    request: Read,
    mut page: Page,
    cancel: &CancellationToken,
) -> Result<Page> {
    const COMMENTS: &str = "id fullDatabaseId body url createdAt updatedAt author { login } replyTo { id } commit { oid } originalCommit { oid } path line originalLine diffHunk";
    let (query, variables, pointer) = match &request.section {
        Section::Threads { after } => (
            format!(
                "query($owner:String!,$repo:String!,$number:Int!,$after:String) {{ repository(owner:$owner,name:$repo) {{ pullRequest(number:$number) {{ url headRefOid reviewThreads(first:20,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ id isResolved isOutdated path line originalLine startLine originalStartLine diffSide startDiffSide subjectType comments(first:20) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {COMMENTS} }} }} }} }} }} }} }}"
            ),
            serde_json::json!({"owner":request.object.repository.owner,"repo":request.object.repository.name,"number":request.object.number,"after":after}),
            "/data/repository/pullRequest/reviewThreads",
        ),
        Section::ThreadComments { thread, after } => (
            format!(
                "query($id:ID!,$after:String) {{ node(id:$id) {{ ... on PullRequestReviewThread {{ id pullRequest {{ url headRefOid }} comments(first:100,after:$after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {COMMENTS} }} }} }} }} }}"
            ),
            serde_json::json!({"id":thread,"after":after}),
            "/data/node/comments",
        ),
        _ => unreachable!(),
    };
    let response = client
        .request(
            reqwest::Method::POST,
            "/graphql",
            Some(&serde_json::json!({"query":query,"variables":variables})),
            cancel,
        )
        .await?;
    super::transport::check_status(&response)?;
    let data = response.json()?;
    ensure!(
        data.get("errors")
            .is_none_or(|errors| errors.as_array().is_some_and(Vec::is_empty)),
        "GitHub GraphQL observation is incomplete or denied; retry explicitly"
    );
    let identity = if matches!(request.section, Section::Threads { .. }) {
        &data["data"]["repository"]["pullRequest"]
    } else {
        &data["data"]["node"]["pullRequest"]
    };
    if let Section::ThreadComments { thread, .. } = &request.section {
        ensure!(
            data["data"]["node"]["id"].as_str() == Some(thread),
            "GitHub returned a different review thread"
        );
    }
    ensure!(
        identity["url"]
            .as_str()
            .is_some_and(|url| url.eq_ignore_ascii_case(&request.object.url())),
        "GitHub thread belongs to a different pull request"
    );
    if Some(sha(&identity["headRefOid"])?) != page.head {
        page.incomplete.push(
            "Pull request head changed during thread observation; reload before publication."
                .into(),
        );
    }
    let connection = data
        .pointer(pointer)
        .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed thread data"))?;
    let nodes = connection["nodes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed thread nodes"))?;
    let (limit, first) = match &request.section {
        Section::Threads { after } => (20, after.is_none()),
        Section::ThreadComments { after, .. } => (100, after.is_none()),
        _ => unreachable!(),
    };
    let has_next = graphql_connection(connection, limit, first)?;
    if has_next {
        let cursor = connection["pageInfo"]["endCursor"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("GitHub thread cursor is missing"))?;
        opaque(cursor)?;
        let section = match &request.section {
            Section::Threads { after } => {
                ensure!(
                    after.as_deref() != Some(cursor),
                    "GitHub thread cursor does not advance"
                );
                Section::Threads {
                    after: Some(cursor.into()),
                }
            }
            Section::ThreadComments { thread, after } => {
                ensure!(
                    after.as_deref() != Some(cursor),
                    "GitHub thread cursor does not advance"
                );
                Section::ThreadComments {
                    thread: thread.clone(),
                    after: Some(cursor.into()),
                }
            }
            _ => unreachable!(),
        };
        page.next = Some(Read {
            section,
            expected_head: page.head.clone(),
            expected_base: page.base.clone(),
            ..request.clone()
        });
    }
    let mut nested = Vec::new();
    if matches!(request.section, Section::Threads { .. }) {
        for thread in nodes {
            let id = thread["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("GitHub thread identity is missing"))?;
            opaque(id)?;
            let more = graphql_connection(&thread["comments"], 20, true)?;
            if more {
                let cursor = thread["comments"]["pageInfo"]["endCursor"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("GitHub nested comment cursor is missing"))?;
                opaque(cursor)?;
                nested.push(Read {
                    object: request.object.clone(),
                    section: Section::ThreadComments {
                        thread: id.into(),
                        after: Some(cursor.into()),
                    },
                    page: 1,
                    expected_head: page.head.clone(),
                    expected_base: page.base.clone(),
                });
            }
        }
    }
    if !nested.is_empty() {
        page.incomplete.push(
            "Review threads have additional comments; follow every nested continuation.".into(),
        );
    }
    page.data = serde_json::json!({"connection":connection,"nested_continuations":nested});
    let current = details(client, &request.object, cancel).await?;
    if Some(sha(&current["head"]["sha"])?) != page.head || Some(base(&current)?) != page.base {
        page.incomplete.push(
            "Pull request head or base changed during thread observation; reload before review."
                .into(),
        );
    }
    Ok(page)
}

fn collection_coverage_consistent(total: Option<u64>, page: u32, count: usize, next: bool) -> bool {
    let end = u64::from(page.saturating_sub(1)) * 100 + count as u64;
    total.is_some_and(|total| total >= end && (next || total == end))
}

fn graphql_connection(connection: &Value, limit: usize, first: bool) -> Result<bool> {
    let nodes = connection["nodes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("GitHub connection nodes are malformed"))?;
    let total = connection["totalCount"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("GitHub connection total is missing"))?;
    let more = connection["pageInfo"]["hasNextPage"]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("GitHub connection pagination is missing"))?;
    ensure!(
        nodes.len() <= limit
            && total >= nodes.len() as u64
            && (!more || !nodes.is_empty())
            && (!first
                || if more {
                    total > nodes.len() as u64
                } else {
                    total == nodes.len() as u64
                }),
        "GitHub connection count or continuation is inconsistent"
    );
    Ok(more)
}

fn next_page(
    headers: &reqwest::header::HeaderMap,
    current: u32,
    expected_path: &str,
    extra: &str,
) -> Result<Option<u32>> {
    let Some(link) = headers.get(reqwest::header::LINK) else {
        return Ok(None);
    };
    let link = link
        .to_str()
        .map_err(|_| anyhow::anyhow!("GitHub returned malformed pagination"))?;
    ensure!(link.len() <= 8192, "GitHub pagination exceeds limit");
    let mut next = None;
    for entry in link.split(',') {
        let Some((url, relation)) = entry.trim().split_once(';') else {
            anyhow::bail!("GitHub returned malformed pagination")
        };
        if relation.trim() != "rel=\"next\"" {
            continue;
        }
        ensure!(next.is_none(), "GitHub returned duplicate pagination");
        let url = url
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .ok_or_else(|| anyhow::anyhow!("GitHub returned malformed pagination"))?;
        let url = reqwest::Url::parse(url)
            .map_err(|_| anyhow::anyhow!("GitHub returned malformed pagination"))?;
        ensure!(
            url.scheme() == "https"
                && url.host_str() == Some("api.github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.fragment().is_none(),
            "GitHub returned unsupported pagination origin"
        );
        let alias = url
            .path()
            .strip_prefix("/repositories/")
            .and_then(|path| path.split_once('/'))
            .is_some_and(|(id, tail)| {
                id.parse::<u64>().is_ok_and(|id| id > 0)
                    && tail
                        == expected_path
                            .split('/')
                            .skip(4)
                            .collect::<Vec<_>>()
                            .join("/")
            });
        ensure!(
            url.path() == expected_path || alias,
            "GitHub pagination refers to a different resource"
        );
        // Numeric repository aliases are only hints: the next request is rebuilt
        // from the validated owner/repository, never from this external URL.
        let expected = reqwest::Url::parse(&format!(
            "https://api.github.com{expected_path}?per_page=100&page={}{extra}",
            current + 1
        ))
        .expect("validated operation pagination");
        let expected_pairs = expected
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let actual_pairs = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        let actual_unique = actual_pairs
            .iter()
            .cloned()
            .collect::<std::collections::BTreeMap<_, _>>();
        ensure!(
            actual_pairs.len() == actual_unique.len() && actual_unique == expected_pairs,
            "GitHub pagination changed or skipped its query"
        );
        let pages = url
            .query_pairs()
            .filter(|(key, _)| key == "page")
            .map(|(_, value)| value.parse::<u32>())
            .collect::<Vec<_>>();
        ensure!(pages.len() == 1, "GitHub returned malformed pagination");
        let page = *pages[0]
            .as_ref()
            .map_err(|_| anyhow::anyhow!("GitHub returned invalid page"))?;
        ensure!(
            page == current + 1 && page <= 1_000_000,
            "GitHub pagination does not advance"
        );
        next = Some(page);
    }
    Ok(next)
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    #[test]
    fn reported_totals_cannot_hide_missing_rest_pages() {
        assert!(!collection_coverage_consistent(Some(101), 1, 1, false));
        assert!(!collection_coverage_consistent(None, 1, 1, false));
        assert!(!collection_coverage_consistent(Some(1), 1, 2, false));
        assert!(collection_coverage_consistent(Some(101), 1, 100, true));
        assert!(collection_coverage_consistent(Some(101), 2, 1, false));
    }
    #[test]
    fn graphql_limits_and_nested_missing_continuations_are_explicit() {
        let connection = |count: usize, total: u64, more: bool| serde_json::json!({"nodes":vec![serde_json::json!({});count],"totalCount":total,"pageInfo":{"hasNextPage":more}});
        assert!(graphql_connection(&connection(21, 21, false), 20, true).is_err());
        assert!(graphql_connection(&connection(0, 25, false), 20, true).is_err());
        assert!(graphql_connection(&connection(0, 25, true), 20, true).is_err());
        assert!(graphql_connection(&connection(20, 25, true), 20, true).unwrap());
        assert!(!graphql_connection(&connection(5, 25, false), 100, false).unwrap());
    }
    #[test]
    fn pagination_cannot_skip_pages_or_change_resource_or_query() {
        let path = "/repos/o/r/pulls/1/files";
        for value in [
            "<https://api.github.com/repos/o/r/pulls/1/files?per_page=100&page=99>; rel=\"next\"",
            "<https://api.github.com/repos/o/r/pulls/2/files?per_page=100&page=2>; rel=\"next\"",
            "<https://api.github.com/repos/o/r/pulls/1/files?per_page=1&page=2>; rel=\"next\"",
        ] {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(reqwest::header::LINK, value.parse().unwrap());
            assert!(next_page(&headers, 1, path, "").is_err());
        }
        let mut duplicated = reqwest::header::HeaderMap::new();
        duplicated.insert(reqwest::header::LINK,"<https://api.github.com/repos/o/r/pulls/1/files?per_page=100&page=2&per_page=100>; rel=\"next\"".parse().unwrap());
        assert!(next_page(&duplicated, 1, path, "&filter=all").is_err());
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::LINK,"<https://api.github.com/repositories/123/pulls/1/files?per_page=100&page=2>; rel=\"next\"".parse().unwrap());
        assert_eq!(next_page(&headers, 1, path, "").unwrap(), Some(2));
    }
}
