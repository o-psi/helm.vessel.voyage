use super::*;
use serde_json::json;
#[tokio::test]
async fn private_database_adapter_preserves_results_and_sanitizes_failures() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let digest = database_at(path.clone(), |store| Ok(store.audit()?.digest))
        .await
        .unwrap();
    assert_eq!(digest.len(), 64);
    assert_eq!(
        database_at(path.clone(), |store| Ok(store.audit()?.digest))
            .await
            .unwrap(),
        digest
    );
    let error = database_at::<()>(path, |_| anyhow::bail!("private fixture diagnostic"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("store unavailable"));
    assert!(!error.contains("private fixture diagnostic"));
}

#[test]
fn projections_remain_bounded_utf8_and_honestly_incomplete() {
    assert_eq!(bounded_projection("exact".into(), 5).unwrap(), "exact");
    assert!(bounded_projection("too long".into(), 0).is_err());
    for maximum in [160, 200, 500] {
        let projected = bounded_projection("é🦀\n\"".repeat(500), maximum).unwrap();
        assert!(projected.len() <= maximum);
        let value: Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(value["incomplete"], true);
        assert!(value["reason"].as_str().unwrap().contains("limit"));
        assert!(
            "é🦀\n\""
                .repeat(500)
                .starts_with(value["excerpt"].as_str().unwrap())
        );
    }
}
#[test]
fn previews_escape_invisible_formatting_without_changing_content() {
    let value = json!({"body":"visible\n\u{200b}\u{202e}\u{2066}\u{feff}é"});
    let preview = json_preview(&value).unwrap();
    assert!(!preview.contains('\u{202e}'));
    assert!(preview.contains("\\u202e"));
    assert_eq!(serde_json::from_str::<Value>(&preview).unwrap(), value);
}
#[test]
fn receipt_validation_binds_actor_content_event_commit_and_object() {
    use crate::github::{publication::ReviewEvent, store::State};
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path().join("journal")).unwrap();
    let owner = Owner::new(root.path(), None, None).unwrap();
    let actor = Actor {
        id: 7,
        login: "offline".into(),
    };
    for event in [
        None,
        Some(ReviewEvent::Comment),
        Some(ReviewEvent::Approve),
        Some(ReviewEvent::RequestChanges),
    ] {
        let object =
            super::super::repository::Object::parse("https://github.com/example/project/pull/4")
                .unwrap();
        let action = match event {
            None => Action::Comment {
                body: "exact".into(),
            },
            Some(event) => Action::Review {
                event,
                commit_id: "a".repeat(40),
                body: "exact".into(),
                comments: vec![],
            },
        };
        let operation = store
            .prepare(
                Draft { object, action },
                actor.clone(),
                "policy".into(),
                Some("a".repeat(40)),
                Some(context::Base {
                    sha: "b".repeat(40),
                    repository: 1,
                    reference: "main".into(),
                }),
                owner.clone(),
            )
            .unwrap();
        assert_eq!(operation.state, State::Prepared);
        let marker = if event.is_some() {
            "pullrequestreview"
        } else {
            "issuecomment"
        };
        let state = match event {
            Some(ReviewEvent::Approve) => "APPROVED",
            Some(ReviewEvent::RequestChanges) => "CHANGES_REQUESTED",
            _ => "COMMENTED",
        };
        let value = json!({"id":11,"user":{"id":7},"body":"exact","commit_id":"a".repeat(40),"state":state,"html_url":format!("{}#{marker}-11",operation.draft.object.url())});
        assert_eq!(validate_receipt(&operation, &value).unwrap().id, 11);
        for (pointer, bad) in [
            ("/id", json!(0)),
            ("/user/id", json!(8)),
            ("/body", json!("changed")),
            (
                "/html_url",
                json!("https://github.com/other/repo/issues/4#issuecomment-11"),
            ),
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = bad;
            assert!(validate_receipt(&operation, &invalid).is_err());
        }
        if event.is_some() {
            for field in ["commit_id", "state"] {
                let mut invalid = value.clone();
                invalid[field] = json!("wrong");
                assert!(validate_receipt(&operation, &invalid).is_err());
            }
        }
    }
}

use crate::github::http_fixture::{Fixture, Reply};
use crate::tools::{InteractionMode, Redactor, UnattendedApprover};
use std::sync::Arc;

fn offline_service(root: &std::path::Path, peer: &Fixture) -> Service {
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        github_enabled: true,
        ..Default::default()
    };
    Service {
        client: peer.client(),
        directory: root.join("journal"),
        owner: Owner::new(root, None, None).unwrap(),
        context: ToolContext {
            tool_call_id: None,
            artifact_scope: None,
            github: Some(crate::github::Credential(Arc::new(
                zeroize::Zeroizing::new("offline-test-token".into()),
            ))),
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, root.to_owned()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: true }),
            timeout: Duration::from_secs(5),
            max_output_bytes: 65536,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: InteractionMode::Attended,
            redactor: Arc::new(Redactor::new(["offline-test-token".into()])),
        },
    }
}
fn issue() -> Draft {
    Draft {
        object: super::super::repository::Object::parse(
            "https://github.com/example/project/issues/7",
        )
        .unwrap(),
        action: Action::Comment {
            body: "offline exact body".into(),
        },
    }
}
fn issue_detail() -> Reply {
    Reply::json(
        "/repos/example/project/issues/7",
        json!({"number":7,"html_url":"https://github.com/example/project/issues/7"}),
    )
}
fn actor_reply() -> Reply {
    Reply::json("/user", json!({"id":7,"login":"offline-bot"}))
}
fn posted() -> Reply {
    Reply {
        method: "POST",
        status: 201,
        ..Reply::json(
            "/repos/example/project/issues/7/comments",
            json!({"id":11,"user":{"id":7},"body":"offline exact body","html_url":"https://github.com/example/project/issues/7#issuecomment-11"}),
        )
    }
}

#[tokio::test]
async fn offline_publication_is_durable_exact_and_non_replayable() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![
        actor_reply(),
        issue_detail(),
        actor_reply(),
        issue_detail(),
        actor_reply(),
        issue_detail(),
        posted(),
    ])
    .await;
    let service = offline_service(root.path(), &peer);
    let prepared = service.prepare(issue()).await.unwrap();
    assert_eq!(prepared.state, State::Prepared);
    assert_eq!(service.list(0).await.unwrap().len(), 1);
    let preview = exact_preview(&prepared, &service.context.redactor).unwrap();
    assert!(preview.contains("offline exact body"));
    assert!(preview.contains(&prepared.digest));
    let published = service
        .publish(prepared.id, &prepared.digest)
        .await
        .unwrap();
    assert_eq!(published.state, State::Published);
    assert_eq!(
        service.inspect(prepared.id).await.unwrap().state,
        State::Published
    );
    assert!(
        service
            .publish(prepared.id, &prepared.digest)
            .await
            .is_err()
    );
    assert!(
        service
            .cancel(prepared.id, prepared.digest.clone())
            .await
            .is_err()
    );
    let requests = peer.finish().await;
    assert_eq!(requests.len(), 7);
    let post = String::from_utf8_lossy(&requests[6]);
    assert!(post.ends_with("{\"body\":\"offline exact body\"}"));
    assert!(!post.contains("offline-test-token"));
}

#[tokio::test]
async fn failed_send_stays_uncertain_and_can_only_be_explicitly_disposed() {
    for code in [500, 403, 200] {
        let root = tempfile::tempdir().unwrap();
        let mut response = posted();
        response.status = code;
        let peer = Fixture::start(vec![
            actor_reply(),
            issue_detail(),
            actor_reply(),
            issue_detail(),
            actor_reply(),
            issue_detail(),
            response,
        ])
        .await;
        let service = offline_service(root.path(), &peer);
        let prepared = service.prepare(issue()).await.unwrap();
        assert!(
            service
                .publish(prepared.id, &prepared.digest)
                .await
                .is_err()
        );
        assert_eq!(
            service.inspect(prepared.id).await.unwrap().state,
            State::Sending
        );
        assert!(
            service
                .publish(prepared.id, &prepared.digest)
                .await
                .is_err()
        );
        assert!(
            service
                .dispose(prepared.id, "wrong", "investigated".into())
                .await
                .is_err()
        );
        assert!(
            service
                .dispose(prepared.id, &prepared.digest, " ".into())
                .await
                .is_err()
        );
        assert!(
            service
                .dispose(prepared.id, &prepared.digest, "offline-test-token".into())
                .await
                .is_err()
        );
        service
            .dispose(
                prepared.id,
                &prepared.digest,
                "operator accepts uncertainty; do not resend".into(),
            )
            .await
            .unwrap();
        assert_ne!(
            service.inspect(prepared.id).await.unwrap().state,
            State::Sending
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn preparing_and_cancelling_do_not_need_attended_write_authority() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![actor_reply(), issue_detail()]).await;
    let mut service = offline_service(root.path(), &peer);
    service.context.interaction = InteractionMode::Unattended;
    let prepared = service.prepare(issue()).await.unwrap();
    assert!(
        service
            .publish(prepared.id, &prepared.digest)
            .await
            .is_err()
    );
    assert!(
        service
            .cancel(prepared.id, "incorrect digest".into())
            .await
            .is_err()
    );
    let cancelled = service
        .cancel(prepared.id, prepared.digest.clone())
        .await
        .unwrap();
    assert_eq!(cancelled.state, State::Cancelled);
    service
        .forget(prepared.id, prepared.digest.clone())
        .await
        .unwrap();
    assert!(service.list(0).await.unwrap().is_empty());
    assert!(service.inspect(prepared.id).await.is_err());
    peer.finish().await;
}

#[tokio::test]
async fn exact_preview_secret_checks_and_actor_validation_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![]).await;
    let service = offline_service(root.path(), &peer);
    let mut secret = issue();
    secret.action = Action::Comment {
        body: "contains offline-test-token".into(),
    };
    assert!(
        service
            .prepare(secret)
            .await
            .unwrap_err()
            .to_string()
            .contains("contains a current configured secret")
    );
    assert!(service.list(513).await.is_err());
    peer.finish().await;
    for actor in [
        json!({}),
        json!({"id":0,"login":"bot"}),
        json!({"id":7,"login":""}),
        json!({"id":7,"login":"bad name"}),
        json!({"id":7,"login":"x".repeat(101)}),
    ] {
        let peer = Fixture::start(vec![Reply::json("/user", actor)]).await;
        let service = offline_service(root.path(), &peer);
        assert!(service.actor().await.is_err());
        peer.finish().await;
    }
}

#[tokio::test]
async fn changed_actor_and_denied_approval_leave_the_draft_prepared() {
    for changed_actor in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let mut replies = vec![actor_reply(), issue_detail()];
        if changed_actor {
            replies.push(Reply::json("/user", json!({"id":8,"login":"other"})));
        } else {
            replies.extend([actor_reply(), issue_detail()]);
        }
        let peer = Fixture::start(replies).await;
        let mut service = offline_service(root.path(), &peer);
        let prepared = service.prepare(issue()).await.unwrap();
        if !changed_actor {
            service.context.approver = Arc::new(UnattendedApprover { allow: false });
        }
        assert!(
            service
                .publish(prepared.id, &prepared.digest)
                .await
                .is_err()
        );
        assert_eq!(
            service.inspect(prepared.id).await.unwrap().state,
            State::Prepared
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn reconciliation_adopts_one_exact_receipt_without_repeating_a_post() {
    let root = tempfile::tempdir().unwrap();
    let mut response = posted();
    response.method = "GET";
    response.status = 200;
    response.path = "/repos/example/project/issues/comments/11".into();
    let peer = Fixture::start(vec![actor_reply(), issue_detail(), actor_reply(), response]).await;
    let service = offline_service(root.path(), &peer);
    let prepared = service.prepare(issue()).await.unwrap();
    let expected = prepared.clone();
    service
        .database(move |store| store.begin_send(&expected))
        .await
        .unwrap();
    assert!(
        service
            .reconcile(prepared.id, &prepared.digest, 0)
            .await
            .is_err()
    );
    let published = service
        .reconcile(prepared.id, &prepared.digest, 11)
        .await
        .unwrap();
    assert_eq!(published.state, State::Published);
    assert!(
        peer.finish()
            .await
            .iter()
            .all(|request| request.starts_with(b"GET "))
    );
}

fn pull_detail() -> Reply {
    Reply::json(
        "/repos/example/project/pulls/7",
        json!({
            "number":7,"html_url":"https://github.com/example/project/pull/7",
            "head":{"sha":"a".repeat(40)},
            "base":{"sha":"b".repeat(40),"repo":{"id":42},"ref":"main"},
            "state":"open","merged":false
        }),
    )
}
fn review() -> Draft {
    Draft {
        object: super::super::repository::Object::parse(
            "https://github.com/example/project/pull/7",
        )
        .unwrap(),
        action: Action::Review {
            event: super::super::publication::ReviewEvent::Comment,
            commit_id: "a".repeat(40),
            body: "offline exact review".into(),
            comments: vec![],
        },
    }
}

#[tokio::test]
async fn pull_review_revalidates_head_and_base_before_and_after_exact_approval() {
    let root = tempfile::tempdir().unwrap();
    let response = Reply {
        method: "POST",
        ..Reply::json(
            "/repos/example/project/pulls/7/reviews",
            json!({
                "id":12,"user":{"id":7},"body":"offline exact review",
                "commit_id":"a".repeat(40),"state":"COMMENTED",
                "html_url":"https://github.com/example/project/pull/7#pullrequestreview-12"
            }),
        )
    };
    let peer = Fixture::start(vec![
        actor_reply(),
        pull_detail(),
        pull_detail(),
        actor_reply(),
        pull_detail(),
        pull_detail(),
        actor_reply(),
        pull_detail(),
        pull_detail(),
        response,
    ])
    .await;
    let service = offline_service(root.path(), &peer);
    let operation = service.prepare(review()).await.unwrap();
    assert_eq!(
        operation.observed_head.as_deref(),
        Some("a".repeat(40).as_str())
    );
    assert_eq!(operation.observed_base.as_ref().unwrap().repository, 42);
    let result = service
        .publish(operation.id, &operation.digest)
        .await
        .unwrap();
    assert_eq!(result.state, State::Published);
    let requests = peer.finish().await;
    let body = requests.last().unwrap();
    let offset = body.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let value: Value = serde_json::from_slice(&body[offset..]).unwrap();
    assert_eq!(value["event"], "COMMENT");
    assert_eq!(value["commit_id"], "a".repeat(40));
}

#[tokio::test]
async fn closed_merged_or_changed_pull_requests_refuse_preparation_without_journaling() {
    for (field, value) in [
        ("state", json!("closed")),
        ("merged", json!(true)),
        ("head", json!({"sha":"c".repeat(40)})),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut detail = pull_detail();
        let mut body: Value = serde_json::from_slice(&detail.body).unwrap();
        body[field] = value;
        detail.body = serde_json::to_vec(&body).unwrap();
        let peer = Fixture::start(vec![actor_reply(), detail]).await;
        let service = offline_service(root.path(), &peer);
        assert!(service.prepare(review()).await.is_err());
        assert!(service.list(0).await.unwrap().is_empty());
        peer.finish().await;
    }
}

#[tokio::test]
async fn cancellation_stops_local_and_remote_service_actions_without_requests() {
    let root = tempfile::tempdir().unwrap();
    let peer = Fixture::start(vec![]).await;
    let service = offline_service(root.path(), &peer);
    service.context.cancellation.cancel();
    assert!(service.list(0).await.is_err());
    assert!(service.inspect(Uuid::new_v4()).await.is_err());
    assert!(service.prepare(issue()).await.is_err());
    assert!(service.publish(Uuid::new_v4(), "digest").await.is_err());
    assert!(
        service
            .reconcile(Uuid::new_v4(), "digest", 1)
            .await
            .is_err()
    );
    assert!(
        service
            .dispose(Uuid::new_v4(), "digest", "note".into())
            .await
            .is_err()
    );
    assert!(
        service
            .cancel(Uuid::new_v4(), "digest".into())
            .await
            .is_err()
    );
    assert!(
        service
            .forget(Uuid::new_v4(), "digest".into())
            .await
            .is_err()
    );
    assert!(peer.finish().await.is_empty());
}
