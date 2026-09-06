//! Offline wire-level context fixtures; production origins are never configurable.
use super::{
    context::{self, Read, Section},
    repository::Object,
    transport::Client,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "synthetic-context-fixture-token";
const DETAIL: &str = "/repos/o/r/pulls/1";
const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Reply {
    path: String,
    status: u16,
    body: Vec<u8>,
    headers: String,
}
impl Reply {
    fn json(path: &str, value: Value) -> Self {
        Self {
            path: path.into(),
            status: 200,
            body: serde_json::to_vec(&value).unwrap(),
            headers: String::new(),
        }
    }
}

fn detail() -> Value {
    json!({"number":1,"html_url":"https://github.com/o/r/pull/1","state":"open",
        "head":{"sha":HEAD},"base":{"sha":BASE,"ref":"main","repo":{"id":1}},"changed_files":1})
}
fn request(section: Section) -> Read {
    Read {
        object: Object::parse("https://github.com/o/r/pull/1").unwrap(),
        section,
        page: 1,
        expected_head: None,
        expected_base: None,
    }
}

/// Runs every read through reqwest and actual HTTP framing. Consuming all
/// scripted replies and rejecting any extra request catches retry/redirect bugs.
async fn observe(request: Read, replies: Vec<Reply>) -> anyhow::Result<context::Page> {
    observe_operation(request, replies, false)
        .await
        .map(Option::unwrap)
}

async fn observe_operation(
    request: Read,
    replies: Vec<Reply>,
    logs: bool,
) -> anyhow::Result<Option<context::Page>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let server_stop = stop.clone();
    let server = tokio::spawn(async move {
        let mut replies = replies.into_iter();
        loop {
            let accepted = tokio::select! {
                biased;
                _ = server_stop.cancelled() => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let (mut socket, _) = accepted;
            let reply = replies
                .next()
                .expect("unexpected retry or redirected request");
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0u8; 4096];
                let count = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    socket.read(&mut chunk),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(count > 0, "request ended before headers");
                bytes.extend_from_slice(&chunk[..count]);
                assert!(bytes.len() <= 128 * 1024, "fixture request exceeds bound");
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let method = if reply.path == "/graphql" {
                "POST"
            } else {
                "GET"
            };
            assert_eq!(
                headers.lines().next().unwrap(),
                format!("{method} {} HTTP/1.1", reply.path)
            );
            let lower = headers.to_ascii_lowercase();
            assert!(lower.contains(&format!("authorization: bearer {TOKEN}\r\n")));
            assert!(lower.contains("x-github-api-version: 2026-03-10\r\n"));
            let length = lower
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length <= 128 * 1024);
            while bytes.len() - header_end < length {
                let mut chunk = [0u8; 4096];
                let count = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    socket.read(&mut chunk),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(count > 0, "request ended before body");
                bytes.extend_from_slice(&chunk[..count]);
            }
            if method == "POST" {
                let body: Value =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                assert!(body["query"].as_str().unwrap().starts_with("query("));
                assert!(!body.to_string().contains(TOKEN));
            }
            let response = format!(
                "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{}\r\n",
                reply.status,
                reply.body.len(),
                reply.headers
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            // Oversized Content-Length cases deliberately close on the client.
            let _ = socket.write_all(&reply.body).await;
        }
        assert!(replies.next().is_none(), "expected request was not made");
    });
    let client =
        Client::fixture(TOKEN.into(), format!("http://{address}/").parse().unwrap()).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        if logs {
            super::logs::read(&client, request.object, 7, &CancellationToken::new())
                .await
                .map(|_| None)
        } else {
            context::read(&client, request, &CancellationToken::new())
                .await
                .map(Some)
        }
    })
    .await;
    stop.cancel();
    server.await.unwrap();
    result.expect("context fixture exceeded deadline")
}

fn job_metadata() -> Value {
    // These remote-controlled URLs are deliberately unrelated and must never
    // become destinations. Job ID 7 is not its check ID 999.
    json!({"id":7,"run_id":9,"head_sha":HEAD,"run_attempt":2,"status":"completed","conclusion":"failure",
        "run_url":"http://127.0.0.1:1/not-the-run","check_run_url":"https://other.invalid/check-runs/999","html_url":"https://other.invalid/job"})
}
fn run_metadata() -> Value {
    json!({"id":9,"head_sha":HEAD})
}
fn log_provenance() -> Vec<Reply> {
    vec![
        Reply::json(DETAIL, detail()),
        Reply::json("/repos/o/r/actions/jobs/7", job_metadata()),
        Reply::json("/repos/o/r/actions/runs/9", run_metadata()),
        Reply::json(DETAIL, detail()),
    ]
}

#[tokio::test]
async fn job_log_identity_failures_stop_before_download_route() {
    for field in ["id", "head_sha", "run_id"] {
        let mut job = job_metadata();
        job[field] = if field == "head_sha" {
            json!(BASE)
        } else if field == "run_id" {
            json!(0)
        } else {
            json!(8)
        };
        let replies = vec![
            Reply::json(DETAIL, detail()),
            Reply::json("/repos/o/r/actions/jobs/7", job),
        ];
        assert!(
            observe_operation(request(Section::Details), replies, true)
                .await
                .is_err(),
            "job {field}"
        );
    }
    for field in ["id", "head_sha"] {
        let mut run = run_metadata();
        run[field] = if field == "head_sha" {
            json!(BASE)
        } else {
            json!(8)
        };
        let replies = vec![
            Reply::json(DETAIL, detail()),
            Reply::json("/repos/o/r/actions/jobs/7", job_metadata()),
            Reply::json("/repos/o/r/actions/runs/9", run),
        ];
        assert!(
            observe_operation(request(Section::Details), replies, true)
                .await
                .is_err(),
            "run {field}"
        );
    }
}

#[tokio::test]
async fn job_log_stale_head_or_base_stops_before_download_route() {
    for field in ["head", "base_sha", "base_ref", "base_repo"] {
        let mut changed = detail();
        match field {
            "head" => changed["head"]["sha"] = json!(BASE),
            "base_sha" => changed["base"]["sha"] = json!(HEAD),
            "base_ref" => changed["base"]["ref"] = json!("retargeted"),
            _ => changed["base"]["repo"]["id"] = json!(2),
        }
        let mut replies = log_provenance();
        *replies.last_mut().unwrap() = Reply::json(DETAIL, changed);
        let error = observe_operation(request(Section::Details), replies, true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed before log download"));
    }
}

#[tokio::test]
async fn job_log_status_and_location_failures_never_fetch_secondary() {
    for (status, headers) in [
        (200, ""),
        (301, "Location: /forbidden\r\n"),
        (307, "Location: /forbidden\r\n"),
        (401, ""),
        (403, ""),
        (429, "Retry-After: 30\r\n"),
        (302, ""),
        (
            302,
            "Location: https://example.invalid/a\r\nLocation: https://example.invalid/b\r\n",
        ),
        (302, "Location: http://127.0.0.1:1/forbidden\r\n"),
        (302, "Location: https://user@example.invalid/forbidden\r\n"),
    ] {
        let mut replies = log_provenance();
        replies.push(Reply {
            path: "/repos/o/r/actions/jobs/7/logs".into(),
            status,
            body: vec![],
            headers: headers.into(),
        });
        assert!(
            observe_operation(request(Section::Details), replies, true)
                .await
                .is_err(),
            "{status} {headers}"
        );
    }
}

fn connection(nodes: Vec<Value>, total: usize, more: bool) -> Value {
    json!({"nodes":nodes,"totalCount":total,"pageInfo":{"hasNextPage":more,"endCursor":if more {Some("next-cursor")} else {None}}})
}
fn thread(id: &str, comments: Value) -> Value {
    json!({"id":id,"comments":comments})
}
fn threads(value: Value) -> Value {
    json!({"data":{"repository":{"pullRequest":{"url":"https://github.com/o/r/pull/1","headRefOid":HEAD,"reviewThreads":value}}}})
}

#[tokio::test]
async fn missing_rest_link_is_explicitly_incomplete_over_http() {
    let page = observe(
        request(Section::CheckSuites),
        vec![
            Reply::json(DETAIL, detail()),
            Reply::json(
                &format!("/repos/o/r/commits/{HEAD}/check-suites?per_page=100&page=1"),
                json!({"total_count":101,"check_suites":[{"id":7}]}),
            ),
            Reply::json(DETAIL, detail()),
        ],
    )
    .await
    .unwrap();
    assert!(page.next.is_none());
    assert!(
        page.incomplete
            .iter()
            .any(|reason| reason.contains("totals"))
    );
    assert_eq!(page.data["reported_total"], 101);
}

#[tokio::test]
async fn graphql_rejects_outer_overrun_and_nested_omissions_over_http() {
    let empty = || connection(vec![], 0, false);
    for value in [
        connection(
            (0..21)
                .map(|id| thread(&format!("T{id}"), empty()))
                .collect(),
            21,
            false,
        ),
        connection(vec![thread("T", connection(vec![], 25, false))], 1, false),
        connection(
            vec![thread(
                "T",
                connection(
                    (0..21).map(|id| json!({"id":format!("C{id}")})).collect(),
                    21,
                    false,
                ),
            )],
            1,
            false,
        ),
    ] {
        let result = observe(
            request(Section::Threads { after: None }),
            vec![
                Reply::json(DETAIL, detail()),
                Reply::json("/graphql", threads(value)),
            ],
        )
        .await;
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn graphql_wrong_thread_and_partial_errors_never_become_context() {
    let wrong = json!({"data":{"node":{"id":"OTHER","pullRequest":{"url":"https://github.com/o/r/pull/1","headRefOid":HEAD},"comments":connection(vec![],0,false)}}});
    let result = observe(
        request(Section::ThreadComments {
            thread: "EXPECTED".into(),
            after: None,
        }),
        vec![
            Reply::json(DETAIL, detail()),
            Reply::json("/graphql", wrong),
        ],
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("different review thread")
    );
    let mut partial = threads(connection(vec![], 0, false));
    partial["errors"] = json!([{"message":"synthetic server denial"}]);
    let result = observe(
        request(Section::Threads { after: None }),
        vec![
            Reply::json(DETAIL, detail()),
            Reply::json("/graphql", partial),
        ],
    )
    .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("incomplete or denied")
    );
}

#[tokio::test]
async fn continuation_head_and_base_changes_stop_before_collection_fetch() {
    for change in ["head", "base_sha", "base_ref", "base_repository"] {
        let mut changed = detail();
        match change {
            "head" => changed["head"]["sha"] = json!("cccccccccccccccccccccccccccccccccccccccc"),
            "base_sha" => {
                changed["base"]["sha"] = json!("cccccccccccccccccccccccccccccccccccccccc")
            }
            "base_ref" => changed["base"]["ref"] = json!("other"),
            _ => changed["base"]["repo"]["id"] = json!(2),
        }
        let mut read = request(Section::Files);
        read.page = 2;
        read.expected_head = Some(HEAD.into());
        read.expected_base = Some(context::base(&detail()).unwrap());
        assert!(
            observe(read, vec![Reply::json(DETAIL, changed)])
                .await
                .is_err(),
            "{change}"
        );
    }
}

#[tokio::test]
async fn failed_statuses_and_redirects_do_not_retry_or_expose_remote_text() {
    for status in [401, 403, 429, 302] {
        let mut reply = Reply::json(DETAIL, json!({"message":"private-server-diagnostic"}));
        reply.status = status;
        // Any redirect would hit this same listener and violate the script.
        reply.headers = "Location: /forbidden-secondary-fetch\r\nRetry-After: 120\r\n".into();
        let error = observe(request(Section::Details), vec![reply])
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-server-diagnostic"));
        assert!(!error.contains(TOKEN));
    }
}

#[tokio::test]
async fn malformed_and_oversized_http_json_are_not_context() {
    for body in [
        b"{\"number\":".to_vec(),
        vec![b' '; super::transport::MAX_RESPONSE_BYTES + 1],
    ] {
        let reply = Reply {
            path: DETAIL.into(),
            status: 200,
            body,
            headers: String::new(),
        };
        assert!(
            observe(request(Section::Details), vec![reply])
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn nested_continuation_preserves_thread_and_head_base_binding() {
    let comments = connection(vec![json!({"id":"C1"})], 2, true);
    let page = observe(
        request(Section::Threads { after: None }),
        vec![
            Reply::json(DETAIL, detail()),
            Reply::json(
                "/graphql",
                threads(connection(vec![thread("T1", comments)], 1, false)),
            ),
            Reply::json(DETAIL, detail()),
        ],
    )
    .await
    .unwrap();
    let next: Read = serde_json::from_value(page.data["nested_continuations"][0].clone()).unwrap();
    assert_eq!(next.object, page.object);
    assert_eq!(next.expected_head, Some(HEAD.into()));
    assert_eq!(next.expected_base, Some(context::base(&detail()).unwrap()));
    assert!(
        matches!(next.section,Section::ThreadComments { thread,after:Some(after) } if thread == "T1" && after == "next-cursor")
    );
    assert!(!page.incomplete.is_empty());
}

#[tokio::test]
async fn pagination_rejects_skipped_pages_duplicate_queries_and_wrong_resources() {
    for target in [
        "https://api.github.com/repos/o/r/pulls/1/files?per_page=100&page=99",
        "https://api.github.com/repos/o/r/pulls/1/files?per_page=100&page=2&page=2",
        "https://api.github.com/repos/o/r/pulls/2/files?per_page=100&page=2",
        "https://api.github.com/repos/o/r/pulls/1/files?per_page=100&page=2&filter=all",
    ] {
        let mut reply = Reply::json("/repos/o/r/pulls/1/files?per_page=100&page=1", json!([]));
        reply.headers = format!("Link: <{target}>; rel=\"next\"\r\n");
        assert!(
            observe(
                request(Section::Files),
                vec![Reply::json(DETAIL, detail()), reply]
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn missing_truncated_and_omitted_hunks_are_explicitly_incomplete() {
    for file in [
        json!({"filename":"a","additions":2,"deletions":0}),
        json!({"filename":"a","additions":2,"deletions":0,"patch":"@@ -0,0 +1,2 @@\n+a"}),
        json!({"filename":"a","additions":2,"deletions":0,"patch":"@@ -0,0 +1 @@\n+a"}),
    ] {
        let page = observe(
            request(Section::Files),
            vec![
                Reply::json(DETAIL, detail()),
                Reply::json(
                    "/repos/o/r/pulls/1/files?per_page=100&page=1",
                    json!([file]),
                ),
                Reply::json(DETAIL, detail()),
            ],
        )
        .await
        .unwrap();
        assert!(
            page.incomplete
                .iter()
                .any(|reason| reason.contains("patch")),
            "{:?}",
            page.incomplete
        );
    }
}
