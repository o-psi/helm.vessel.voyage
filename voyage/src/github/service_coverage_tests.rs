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
