//! Parent wiring: in github/operator.rs add #[cfg(test)]
//! #[path = "operator_final_tests.rs"] mod final_tests;
//! All remote observations use the existing bounded loopback script.
use super::*;
use crate::github::{
    context,
    http_fixture::{Fixture, Reply},
};
use crate::tools::{InteractionMode, Redactor, ToolContext, UnattendedApprover};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

const ISSUE: &str = "https://github.com/example/project/issues/7";
const PULL: &str = "https://github.com/example/project/pull/7";

fn context(root: &std::path::Path) -> ToolContext {
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        github_enabled: false,
        ..Default::default()
    };
    ToolContext {
        tool_call_id: None,
        artifact_scope: None,
        github: None,
        completion: None,
        policy: Arc::new(crate::policy::Policy::new(&config, root.to_owned()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(5),
        max_output_bytes: 65536,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::new(Vec::<String>::new())),
    }
}

fn view(section: SectionArg, url: &str) -> ViewArgs {
    ViewArgs {
        url: url.into(),
        section,
        page: 1,
        after: None,
        resource: None,
        thread: None,
        head: None,
    }
}

fn detail() -> serde_json::Value {
    json!({"number":7,"html_url":PULL,"head":{"sha":"a".repeat(40)},
        "base":{"sha":"b".repeat(40),"repo":{"id":42},"ref":"main"},
        "changed_files":1})
}

#[tokio::test]
async fn operator_help_never_needs_github_authority() {
    let root = tempfile::tempdir().unwrap();
    for words in [
        vec!["--help"],
        vec!["view", "--help"],
        vec!["prepare", "--help"],
        vec!["feedback", "--help"],
        vec!["admin", "--help"],
    ] {
        let result = execute(
            context(root.path()),
            None,
            words.into_iter().map(str::to_owned).collect(),
        )
        .await
        .unwrap();
        assert!(result.display.contains("Usage:"));
        assert!(result.reference.is_none());
        assert!(result.feedback.is_none());
    }
}

#[tokio::test]
async fn operator_argument_failures_are_authored_and_do_not_echo_input() {
    let root = tempfile::tempdir().unwrap();
    for words in [
        vec!["unknown-secret-command"],
        vec!["view"],
        vec!["view", ISSUE, "--page", "not-a-number"],
        vec![
            "prepare",
            ISSUE,
            "--body",
            "sensitive-text",
            "--body-file",
            "secret-path",
        ],
        vec!["prepare", PULL, "--event", "INVALID"],
        vec!["publish", "not-a-uuid", "--digest", "private-digest"],
    ] {
        let error = execute(
            context(root.path()),
            None,
            words.into_iter().map(str::to_owned).collect(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid GitHub arguments; use /github --help"
        );
    }
}

#[tokio::test]
async fn frontend_owned_reference_commands_refuse_without_opening_a_store() {
    let root = tempfile::tempdir().unwrap();
    for command in [
        Command::References,
        Command::Unreference { url: ISSUE.into() },
    ] {
        let error = execute_args(context(root.path()), None, Args { command })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("frontend"));
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn section_conversion_requires_only_the_section_specific_identity() {
    for section in [
        SectionArg::ReviewComments,
        SectionArg::SuiteChecks,
        SectionArg::Annotations,
        SectionArg::Jobs,
    ] {
        let args = view(section, PULL);
        assert!(args.read().unwrap_err().to_string().contains("--resource"));
        let mut args = view(section, PULL);
        args.resource = Some(42);
        assert!(args.read().is_ok());
    }
    let args = view(SectionArg::ThreadComments, PULL);
    assert!(args.read().unwrap_err().to_string().contains("--thread"));
    let mut args = view(SectionArg::ThreadComments, PULL);
    args.thread = Some("PRRT_fixture".into());
    assert!(args.read().is_ok());
}

#[test]
fn section_conversion_preserves_pagination_and_commit_identity() {
    for section in [
        SectionArg::Details,
        SectionArg::Comments,
        SectionArg::Reviews,
        SectionArg::Files,
        SectionArg::Threads,
        SectionArg::Statuses,
        SectionArg::Checks,
        SectionArg::Suites,
        SectionArg::Runs,
    ] {
        let mut args = view(section, PULL);
        args.head = Some("a".repeat(40));
        let read = args.read().unwrap();
        assert_eq!(read.object.url(), PULL);
        assert_eq!(read.page, 1);
        assert_eq!(read.expected_head, Some("a".repeat(40)));
        assert!(read.expected_base.is_none());
    }
    let mut args = view(SectionArg::Comments, ISSUE);
    args.page = 0;
    assert!(args.read().is_err());
    let mut args = view(SectionArg::Details, PULL);
    args.head = Some("not-a-sha".into());
    assert!(args.read().is_err());
}

#[tokio::test]
async fn converted_details_become_valid_saved_references() {
    for (url, path, body, has_head) in [
        (
            ISSUE,
            "/repos/example/project/issues/7",
            json!({"number":7,"html_url":ISSUE}),
            false,
        ),
        (PULL, "/repos/example/project/pulls/7", detail(), true),
    ] {
        let peer = Fixture::start(vec![Reply::json(path, body)]).await;
        let page = context::read(
            &peer.client(),
            view(SectionArg::Details, url).read().unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let reference = Reference {
            object: page.object,
            head: page.head,
            fetched_at: page.fetched_at,
        };
        reference.validate().unwrap();
        assert_eq!(reference.head.is_some(), has_head);
        let encoded = serde_json::to_value(&reference).unwrap();
        assert_eq!(
            serde_json::from_value::<Reference>(encoded).unwrap(),
            reference
        );
        assert_eq!(peer.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn converted_issue_comments_keep_feedback_attribution() {
    let peer = Fixture::start(vec![
        Reply::json(
            "/repos/example/project/issues/7",
            json!({"number":7,"html_url":ISSUE}),
        ),
        Reply::json(
            "/repos/example/project/issues/7/comments?per_page=100&page=1",
            json!([{"id":19,"html_url":format!("{ISSUE}#issuecomment-19"),
            "body":"Please retain the cancellation test", "user":{"login":"reviewer","id":23}}]),
        ),
    ])
    .await;
    let page = context::read(
        &peer.client(),
        view(SectionArg::Comments, ISSUE).read().unwrap(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let feedback = attributed_feedback(
        &page.object,
        &page.data["items"][0],
        page.fetched_at,
        Some(19),
    )
    .unwrap();
    assert_eq!(feedback.source, format!("{ISSUE}#issuecomment-19"));
    assert!(feedback.title.ends_with("feedback 19"));
    assert!(feedback.description.contains("reviewer (GitHub user 23)"));
    assert!(
        feedback
            .description
            .contains("Please retain the cancellation test")
    );
    assert_eq!(peer.finish().await.len(), 2);
}

#[tokio::test]
async fn converted_detail_binds_feedback_to_the_observed_object() {
    let peer = Fixture::start(vec![Reply::json(
        "/repos/example/project/issues/7",
        json!({"number":7,"html_url":ISSUE,"body":"reported defect","user":null}),
    )])
    .await;
    let page = context::read(
        &peer.client(),
        view(SectionArg::Details, ISSUE).read().unwrap(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let feedback = attributed_feedback(&page.object, &page.data, page.fetched_at, None).unwrap();
    assert_eq!(feedback.source, ISSUE);
    assert!(
        feedback
            .description
            .contains("Unavailable in the GitHub response")
    );
    assert!(feedback.description.ends_with("reported defect"));
    let mut unrelated = page.data.clone();
    unrelated["html_url"] = json!("https://github.com/example/project/issues/8");
    assert!(attributed_feedback(&page.object, &unrelated, page.fetched_at, None).is_err());
    peer.finish().await;
}

#[test]
fn attributed_feedback_rejects_unusable_or_cross_object_sources() {
    let object = Object::parse(ISSUE).unwrap();
    let now = Utc::now();
    for source in [
        serde_json::Value::Null,
        json!("not a URL"),
        json!("https://github.com/other/project/issues/7"),
        json!("https://github.com/example/project/pull/7"),
        json!(format!("{ISSUE}#unsupported-1")),
        json!(format!("{ISSUE}#issuecomment-0")),
    ] {
        let entry = json!({"html_url":source,"body":"body"});
        assert!(attributed_feedback(&object, &entry, now, None).is_err());
    }
    assert!(attributed_feedback(&object, &json!({"html_url":ISSUE}), now, None).is_err());
    for user in [
        json!({}),
        json!({"id":0,"login":"reviewer"}),
        json!({"id":1,"login":""}),
        json!({"id":1,"login":"a".repeat(101)}),
    ] {
        let feedback = attributed_feedback(
            &object,
            &json!({"html_url":ISSUE,"body":"body","user":user}),
            now,
            None,
        )
        .unwrap();
        assert!(feedback.description.contains("Unavailable"));
    }
}

#[test]
fn feedback_bounds_are_utf8_byte_bounds_and_sources_are_canonical() {
    let valid = Feedback {
        source: ISSUE.into(),
        title: "title".into(),
        description: "body".into(),
    };
    for fragment in ["issuecomment-1", "pullrequestreview-2", "discussion_r3"] {
        let mut feedback = valid.clone();
        feedback.source = format!("{ISSUE}#{fragment}");
        feedback.validate().unwrap();
    }
    for source in [
        format!("{ISSUE}/"),
        format!("{ISSUE}?q=1"),
        format!("{ISSUE}#discussion_r0"),
        format!("{ISSUE}#issuecomment-18446744073709551615"),
        "http://github.com/example/project/issues/7".into(),
    ] {
        let mut feedback = valid.clone();
        feedback.source = source;
        assert!(feedback.validate().is_err());
    }
    let mut feedback = valid.clone();
    feedback.title = "é".repeat(128);
    feedback.description = "x".repeat(16 * 1024);
    feedback.validate().unwrap();
    feedback.title.push('x');
    assert!(feedback.validate().is_err());
    feedback.title = "ok".into();
    feedback.description.push('x');
    assert!(feedback.validate().is_err());
    feedback.description.clear();
    feedback.title = " \n\t".into();
    assert!(feedback.validate().is_err());
}

#[test]
fn saved_reference_requires_exact_pull_head_and_strict_schema() {
    let now = Utc::now();
    for (url, head, valid) in [
        (ISSUE, None, true),
        (ISSUE, Some("a".repeat(40)), false),
        (PULL, None, false),
        (PULL, Some("a".repeat(40)), true),
        (PULL, Some("bad".into()), false),
    ] {
        let reference = Reference {
            object: Object::parse(url).unwrap(),
            head,
            fetched_at: now,
        };
        assert_eq!(reference.validate().is_ok(), valid);
        let mut value = serde_json::to_value(&reference).unwrap();
        value["untrusted"] = json!(true);
        assert!(serde_json::from_value::<Reference>(value).is_err());
    }
}

#[tokio::test]
async fn input_reader_accepts_relative_utf8_and_exact_size_limit() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    std::fs::write(root.path().join("body.txt"), "résumé\nreview").unwrap();
    assert_eq!(
        read_input(&context, "body.txt".into()).await.unwrap(),
        "résumé\nreview"
    );
    std::fs::write(root.path().join("limit"), vec![b'x'; 128 * 1024]).unwrap();
    assert_eq!(
        read_input(&context, "limit".into()).await.unwrap().len(),
        128 * 1024
    );
    std::fs::write(root.path().join("limit"), vec![b'x'; 128 * 1024 + 1]).unwrap();
    assert!(
        read_input(&context, "limit".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    std::fs::write(root.path().join("invalid"), [255, 254]).unwrap();
    assert!(
        read_input(&context, "invalid".into())
            .await
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
}

#[tokio::test]
async fn input_reader_refuses_directories_missing_paths_and_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let context = context(root.path());
    assert!(read_input(&context, "missing".into()).await.is_err());
    assert!(read_input(&context, root.path().into()).await.is_err());
    std::fs::write(root.path().join("body"), "body").unwrap();
    context.cancellation.cancel();
    assert!(read_input(&context, "body".into()).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn input_reader_does_not_follow_a_link_outside_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), "never returned").unwrap();
    std::os::unix::fs::symlink(outside.path().join("private"), root.path().join("link")).unwrap();
    assert!(
        read_input(&context(root.path()), "link".into())
            .await
            .is_err()
    );
}
