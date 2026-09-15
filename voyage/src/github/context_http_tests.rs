use super::*;
use crate::github::http_fixture::{Fixture, Reply};
use serde_json::json;

const DETAIL: &str = "/repos/example/project/pulls/7";
fn request(section: Section) -> Read {
    Read {
        object: Object::parse("https://github.com/example/project/pull/7").unwrap(),
        section,
        page: 1,
        expected_head: None,
        expected_base: None,
    }
}
fn detail() -> Value {
    json!({"number":7,"html_url":"https://github.com/example/project/pull/7","head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40),"repo":{"id":42},"ref":"main"},"changed_files":1})
}
fn snapshot() -> Reply {
    Reply::json(DETAIL, detail())
}
async fn observe(section: Section, replies: Vec<Reply>) -> Result<Page> {
    let server = Fixture::start(replies).await;
    let result = read(
        &server.client(),
        request(section),
        &CancellationToken::new(),
    )
    .await;
    server.finish().await;
    result
}

#[tokio::test]
async fn issue_and_pull_details_bind_only_pull_commit_identity() {
    let page = observe(Section::Details, vec![snapshot()]).await.unwrap();
    assert_eq!(page.head.as_deref(), Some("a".repeat(40).as_str()));
    assert_eq!(page.base.unwrap().repository, 42);
    assert_eq!(page.data, detail());
    let server = Fixture::start(vec![Reply::json(
        "/repos/example/project/issues/7",
        json!({"number":7,"html_url":"https://github.com/example/project/issues/7","body":"untrusted text"}),
    )])
    .await;
    let mut req = request(Section::Details);
    req.object = Object::parse("https://github.com/example/project/issues/7").unwrap();
    let page = read(&server.client(), req, &CancellationToken::new())
        .await
        .unwrap();
    assert!(page.head.is_none() && page.base.is_none());
    assert_eq!(page.data["body"], "untrusted text");
    server.finish().await;
}

#[tokio::test]
async fn detail_identity_and_continuation_mismatches_stop_before_collection() {
    for (field, value, fragment) in [
        ("number", json!(8), "different object"),
        (
            "head",
            json!({"sha":"c".repeat(40)}),
            "continuation is stale",
        ),
        (
            "base",
            json!({"sha":"c".repeat(40),"repo":{"id":42},"ref":"main"}),
            "base changed",
        ),
    ] {
        let mut data = detail();
        data[field] = value;
        let server = Fixture::start(vec![Reply::json(DETAIL, data)]).await;
        let mut req = request(Section::Comments);
        req.expected_head = Some("a".repeat(40));
        req.expected_base = Some(base(&detail()).unwrap());
        assert!(
            read(&server.client(), req, &CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
                .contains(fragment)
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn rest_sections_select_exact_routes_and_preserve_collections() {
    let head = "a".repeat(40);
    let cases = [
        (
            Section::Comments,
            "/repos/example/project/issues/7/comments".into(),
            json!([{"id":1}]),
            "",
        ),
        (
            Section::Reviews,
            "/repos/example/project/pulls/7/reviews".into(),
            json!([]),
            "",
        ),
        (
            Section::ReviewComments { review: 9 },
            "/repos/example/project/pulls/7/reviews/9/comments".into(),
            json!([]),
            "",
        ),
        (
            Section::Statuses,
            format!("/repos/example/project/commits/{head}/statuses"),
            json!([]),
            "",
        ),
        (
            Section::Checks,
            format!("/repos/example/project/commits/{head}/check-runs"),
            json!({"total_count":1,"check_runs":[{"id":11,"output":{"annotations_count":2}}]}),
            "&filter=all",
        ),
        (
            Section::CheckSuites,
            format!("/repos/example/project/commits/{head}/check-suites"),
            json!({"total_count":1,"check_suites":[{"id":12}]}),
            "",
        ),
        (
            Section::WorkflowRuns,
            "/repos/example/project/actions/runs".into(),
            json!({"total_count":1,"workflow_runs":[{"id":13}]}),
            "",
        ),
    ];
    for (section, path, body, extra) in cases {
        let extra = if section == Section::WorkflowRuns {
            format!("&head_sha={head}")
        } else {
            extra.into()
        };
        let page = observe(
            section.clone(),
            vec![
                snapshot(),
                Reply::json(format!("{path}?per_page=100&page=1{extra}"), body),
                snapshot(),
            ],
        )
        .await
        .unwrap();
        assert!(page.next.is_none());
        assert!(page.data["items"].is_array());
        let nested = &page.data["nested_continuations"];
        match section {
            Section::Checks | Section::CheckSuites | Section::WorkflowRuns => {
                let req: Read = serde_json::from_value(nested[0].clone()).unwrap();
                req.validate().unwrap();
                assert_eq!(req.expected_head, page.head);
                assert_eq!(req.expected_base, page.base);
                assert_eq!(
                    req.section,
                    match section {
                        Section::Checks => Section::Annotations { check: 11 },
                        Section::CheckSuites => Section::SuiteChecks { suite: 12 },
                        _ => Section::Jobs { run: 13 },
                    }
                );
            }
            _ => assert_eq!(nested, &json!([])),
        }
    }
}

#[tokio::test]
async fn nested_resources_require_selected_head_before_reading_results() {
    for (section, identity, collection, body, extra) in [
        (
            Section::SuiteChecks { suite: 5 },
            "/repos/example/project/check-suites/5",
            "/repos/example/project/check-suites/5/check-runs",
            json!({"total_count":0,"check_runs":[]}),
            "&filter=all",
        ),
        (
            Section::Jobs { run: 6 },
            "/repos/example/project/actions/runs/6",
            "/repos/example/project/actions/runs/6/jobs",
            json!({"total_count":0,"jobs":[]}),
            "&filter=all",
        ),
        (
            Section::Annotations { check: 8 },
            "/repos/example/project/check-runs/8",
            "/repos/example/project/check-runs/8/annotations",
            json!([]),
            "",
        ),
    ] {
        let error = observe(
            section.clone(),
            vec![
                snapshot(),
                Reply::json(identity, json!({"head_sha":"c".repeat(40)})),
            ],
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not belong"));
        let page = observe(
            section,
            vec![
                snapshot(),
                Reply::json(identity, json!({"head_sha":"a".repeat(40)})),
                Reply::json(format!("{collection}?per_page=100&page=1{extra}"), body),
                snapshot(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(page.data["items"], json!([]));
        assert!(page.incomplete.is_empty());
    }
}

#[tokio::test]
async fn collection_shapes_limits_and_nested_ids_are_checked() {
    for (section, path, body, after, fragment) in [
        (
            Section::Comments,
            "/repos/example/project/issues/7/comments?per_page=100&page=1",
            json!({}),
            false,
            "malformed collection",
        ),
        (
            Section::Comments,
            "/repos/example/project/issues/7/comments?per_page=100&page=1",
            json!(vec![json!({}); 101]),
            false,
            "page limit",
        ),
        (
            Section::WorkflowRuns,
            "/repos/example/project/actions/runs?per_page=100&page=1&head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            json!({"total_count":0}),
            false,
            "result collection",
        ),
        (
            Section::WorkflowRuns,
            "/repos/example/project/actions/runs?per_page=100&page=1&head_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            json!({"total_count":1,"workflow_runs":[{}]}),
            true,
            "ID is missing",
        ),
    ] {
        let mut replies = vec![snapshot(), Reply::json(path, body)];
        if after {
            replies.push(snapshot());
        }
        assert!(
            observe(section, replies)
                .await
                .unwrap_err()
                .to_string()
                .contains(fragment)
        );
    }
}

#[tokio::test]
async fn file_observation_reports_omitted_truncated_and_inconsistent_diffs() {
    for file in [
        json!({"filename":"binary"}),
        json!({"patch":"not a patch"}),
        json!({"patch":"@@ -1 +1 @@\n-old\n+new","additions":99,"deletions":1}),
    ] {
        let mut data = detail();
        data["changed_files"] = json!(3000);
        let page = observe(
            Section::Files,
            vec![
                Reply::json(DETAIL, data),
                Reply::json(
                    "/repos/example/project/pulls/7/files?per_page=100&page=1",
                    json!([file]),
                ),
                snapshot(),
            ],
        )
        .await
        .unwrap();
        assert!(page.incomplete.iter().any(|s| s.contains("3000")));
        assert!(
            page.incomplete
                .iter()
                .any(|s| s.contains("totals disagree"))
        );
        assert!(page.incomplete.iter().any(|s| s.contains("patches")));
    }
}

#[tokio::test]
async fn pagination_binds_snapshot_and_reports_concurrent_identity_changes() {
    let mut reply = Reply::json(
        "/repos/example/project/issues/7/comments?per_page=100&page=1",
        json!([]),
    );
    reply.headers.push(("Link".into(), "<https://api.github.com/repos/example/project/issues/7/comments?per_page=100&page=2>; rel=\"next\"".into()));
    let mut changed = detail();
    changed["head"]["sha"] = json!("c".repeat(40));
    changed["base"]["ref"] = json!("release");
    let page = observe(
        Section::Comments,
        vec![snapshot(), reply, Reply::json(DETAIL, changed)],
    )
    .await
    .unwrap();
    let next = page.next.unwrap();
    assert_eq!(next.page, 2);
    assert_eq!(next.expected_head, page.head);
    assert_eq!(next.expected_base, page.base);
    assert!(page.incomplete.iter().any(|s| s.contains("head changed")));
    assert!(page.incomplete.iter().any(|s| s.contains("base changed")));
}

fn connection(nodes: Value, total: u64, cursor: Option<&str>) -> Value {
    json!({"nodes":nodes,"totalCount":total,"pageInfo":{"hasNextPage":cursor.is_some(),"endCursor":cursor}})
}
fn graphql(conn: Value) -> Value {
    json!({"data":{"repository":{"pullRequest":{"url":"https://github.com/example/project/pull/7","headRefOid":"a".repeat(40),"reviewThreads":conn}}}})
}
#[tokio::test]
async fn graphql_threads_emit_outer_and_nested_bound_continuations() {
    let comments = connection(json!([{"id":"comment"}]), 2, Some("comments-next"));
    let body = graphql(connection(
        json!([{"id":"thread-1","comments":comments}]),
        2,
        Some("threads-next"),
    ));
    let page = observe(
        Section::Threads { after: None },
        vec![snapshot(), Reply::post(body), snapshot()],
    )
    .await
    .unwrap();
    assert_eq!(
        page.next.unwrap().section,
        Section::Threads {
            after: Some("threads-next".into())
        }
    );
    let nested: Read =
        serde_json::from_value(page.data["nested_continuations"][0].clone()).unwrap();
    assert_eq!(
        nested.section,
        Section::ThreadComments {
            thread: "thread-1".into(),
            after: Some("comments-next".into())
        }
    );
    assert_eq!(nested.expected_head, page.head);
    assert!(
        page.incomplete
            .iter()
            .any(|s| s.contains("additional comments"))
    );
}
#[tokio::test]
async fn graphql_errors_identity_and_cursor_fail_closed() {
    let valid = graphql(connection(json!([]), 0, None));
    let mut wrong_url = valid.clone();
    wrong_url["data"]["repository"]["pullRequest"]["url"] =
        json!("https://github.com/other/project/pull/7");
    let mut repeated = graphql(connection(json!([{"id":"thread"}]), 2, Some("same")));
    repeated["data"]["repository"]["pullRequest"]["headRefOid"] = json!("c".repeat(40));
    for (body, section, fragment) in [
        (
            json!({"errors":[{"message":"private"}]}),
            Section::Threads { after: None },
            "incomplete or denied",
        ),
        (
            wrong_url,
            Section::Threads { after: None },
            "different pull request",
        ),
        (
            repeated,
            Section::Threads {
                after: Some("same".into()),
            },
            "does not advance",
        ),
    ] {
        assert!(
            observe(section, vec![snapshot(), Reply::post(body)])
                .await
                .unwrap_err()
                .to_string()
                .contains(fragment)
        );
    }
}
#[tokio::test]
async fn graphql_thread_comments_verify_node_and_report_head_races() {
    let section = Section::ThreadComments {
        thread: "thread-1".into(),
        after: None,
    };
    let mut body = json!({"data":{"node":{"id":"wrong","pullRequest":{"url":"https://github.com/example/project/pull/7","headRefOid":"c".repeat(40)},"comments":connection(json!([]),0,None)}}});
    assert!(
        observe(section.clone(), vec![snapshot(), Reply::post(body.clone())])
            .await
            .unwrap_err()
            .to_string()
            .contains("different review thread")
    );
    body["data"]["node"]["id"] = json!("thread-1");
    let mut changed = detail();
    changed["base"]["ref"] = json!("release");
    let page = observe(
        section,
        vec![snapshot(), Reply::post(body), Reply::json(DETAIL, changed)],
    )
    .await
    .unwrap();
    assert!(page.next.is_none());
    assert_eq!(page.incomplete.len(), 2);
}

#[tokio::test]
async fn capped_file_and_workflow_pages_do_not_advertise_unreadable_continuations() {
    for (section, page_number, path, extra, body) in [
        (
            Section::Files,
            30,
            "/repos/example/project/pulls/7/files",
            String::new(),
            json!([]),
        ),
        (
            Section::WorkflowRuns,
            10,
            "/repos/example/project/actions/runs",
            format!("&head_sha={}", "a".repeat(40)),
            json!({"total_count":1000,"workflow_runs":[]}),
        ),
    ] {
        let mut reply = Reply::json(
            format!("{path}?per_page=100&page={page_number}{extra}"),
            body,
        );
        reply.headers.push((
            "Link".into(),
            format!(
                "<https://api.github.com{path}?per_page=100&page={}{extra}>; rel=\"next\"",
                page_number + 1
            ),
        ));
        let server = Fixture::start(vec![snapshot(), reply, snapshot()]).await;
        let mut req = request(section);
        req.page = page_number;
        req.expected_head = Some("a".repeat(40));
        req.expected_base = Some(base(&detail()).unwrap());
        let page = read(&server.client(), req, &CancellationToken::new())
            .await
            .unwrap();
        assert!(page.next.is_none());
        assert!(!page.incomplete.is_empty());
        server.finish().await;
    }
}

#[tokio::test]
async fn graphql_request_uses_variables_for_opaque_cursors() {
    let server = Fixture::start(vec![
        snapshot(),
        Reply::post(graphql(connection(json!([]), 0, None))),
        snapshot(),
    ])
    .await;
    read(
        &server.client(),
        request(Section::Threads {
            after: Some("opaque\"cursor".into()),
        }),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let requests = server.finish().await;
    let request = String::from_utf8(requests[1].clone()).unwrap();
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        body["variables"],
        json!({"owner":"example","repo":"project","number":7,"after":"opaque\"cursor"})
    );
    assert!(!body["query"].as_str().unwrap().contains("opaque"));
}
