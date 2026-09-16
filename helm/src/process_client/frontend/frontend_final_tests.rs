use super::*;
#[path = "../../dispatch_fixture_final_tests.rs"]
// Each consumer gets an independent scripted fixture module.
#[allow(clippy::duplicate_mod)]
mod fixture;
use fixture::*;
use serde_json::json;

#[tokio::test]
async fn resume_uses_catalogue_identity_and_never_recovers_unsafe_owners() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    for state in [
        "live",
        "suspended",
        "stopped",
        "unavailable",
        "cleanup_unconfirmed",
        "relinquished",
        "starting",
    ] {
        let mut process = info(s, i);
        process["state"] = json!(state);
        let mut script = vec![(json!({"op":"catalogue"}), json!([process]))];
        if state == "stopped" {
            script.push((
                json!({"op":"restart","session_id":s,"incarnation":i}),
                info(s, i),
            ));
        }
        let peer = Peer::new(script).await;
        let result = resume::open(
            &peer.client,
            &crate::Config::default(),
            None,
            &s.to_string(),
        )
        .await;
        if matches!(state, "live" | "suspended" | "stopped") {
            assert_eq!(result.unwrap().session_id, s);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("owner unavailable")
            );
        }
        peer.finish().await;
    }
    let peer = Peer::new(vec![
        (json!({"op":"catalogue"}), json!([info(s, i)])),
        (
            json!({"op":"snapshot","session_id":s}),
            json!({"name":"named"}),
        ),
    ])
    .await;
    assert_eq!(
        resume::open(&peer.client, &crate::Config::default(), None, "named")
            .await
            .unwrap()
            .session_id,
        s
    );
    peer.finish().await;
    let peer = Peer::new(vec![(
        json!({"op":"catalogue"}),
        json!([info(s, i), info(s, i)]),
    )])
    .await;
    assert!(
        resume::open(
            &peer.client,
            &crate::Config::default(),
            None,
            &s.to_string()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("ambiguous")
    );
    peer.finish().await;
}

#[tokio::test]
async fn discard_retains_active_or_cleanup_pending_voyages() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    for snapshot in [
        json!({"run":{"state":"accepted"}}),
        json!({"run":{"state":"running"}}),
        json!({"run":{"state":"awaiting_decision"}}),
        json!({"run":{"state":"cancel_requested"}}),
        json!({"pending_cleanup_run":Uuid::new_v4()}),
        json!({}),
    ] {
        let peer = Peer::new(vec![(json!({"op":"snapshot","session_id":s}), snapshot)]).await;
        let process = serde_json::from_value(info(s, i)).unwrap();
        assert!(discard(&peer.client, &process).await.is_err());
        peer.finish().await;
    }
}

#[test]
fn launch_persistence_is_private_unique_and_captures_workspace() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let config = crate::Config::default();
    let one = persist_launch(&config, root.path(), root.path()).unwrap();
    let two = persist_launch(&config, root.path(), root.path()).unwrap();
    assert_ne!(one, two);
    assert_eq!(
        std::fs::metadata(&one).unwrap().permissions().mode() & 0o077,
        0
    );
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(one).unwrap()).unwrap();
    assert!(value.is_object());
    std::fs::set_permissions(
        root.path().join("launch"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(persist_launch(&config, root.path(), root.path()).is_err());
}

#[test]
fn github_cli_roundtrips_each_dispatch_family_without_services() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: crate::github::operator::Args,
    }
    let id = Uuid::new_v4().to_string();
    let cases = vec![
        vec!["auth"],
        vec!["remotes"],
        vec!["references"],
        vec!["logs", "https://example.invalid/run", "42"],
        vec!["continue", "request"],
        vec!["reference", "https://example.invalid/pr"],
        vec!["unreference", "https://example.invalid/pr"],
        vec!["admin", "list", "--offset", "3"],
        vec!["admin", "inspect", &id],
        vec!["admin", "cancel", &id, "digest"],
        vec!["admin", "dispose", &id, "digest", "note"],
        vec!["admin", "forget", &id, "digest"],
        vec!["admin", "audit"],
        vec!["admin", "clear-audit", "digest"],
        vec![
            "view",
            "https://example.invalid/pr",
            "--section",
            "threads",
            "--page",
            "3",
            "--after",
            "cursor",
        ],
        vec!["feedback", "https://example.invalid/pr", "--id", "42"],
        vec!["publish", &id, "digest"],
        vec!["inspect", &id],
        vec!["list", "--offset", "4"],
        vec!["cancel", &id, "digest"],
        vec!["forget", &id, "digest"],
        vec!["reconcile", &id, "digest", "42"],
        vec!["dispose", &id, "digest", "note"],
        vec![
            "prepare",
            "https://example.invalid/pr",
            "--body",
            "two words",
            "--event",
            "COMMENT",
            "--commit",
            "sha",
        ],
    ];
    for words in cases {
        let parsed =
            Cli::try_parse_from(std::iter::once("github").chain(words.iter().copied())).unwrap();
        let rendered = github_args::words(parsed.args).unwrap();
        assert_eq!(rendered.first().unwrap(), words[0]);
        let reparsed = Cli::try_parse_from(
            std::iter::once("github").chain(rendered.iter().map(String::as_str)),
        )
        .unwrap();
        assert_eq!(github_args::words(reparsed.args).unwrap(), rendered);
    }
}

#[test]
fn github_prepare_canonicalizes_only_explicit_local_files() {
    use crate::github::operator::{Args, Command, PrepareArgs};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("draft.json");
    std::fs::write(&path, "{}").unwrap();
    for draft in [false, true] {
        let args = Args {
            command: Command::Prepare(PrepareArgs {
                url: None,
                body: None,
                event: None,
                commit: None,
                body_file: (!draft).then(|| path.clone()),
                draft_file: draft.then(|| path.clone()),
            }),
        };
        let words = github_args::words(args).unwrap();
        assert_eq!(
            words,
            vec![
                "prepare".to_owned(),
                if draft { "--draft-file" } else { "--body-file" }.to_owned(),
                path.canonicalize().unwrap().to_str().unwrap().to_owned()
            ]
        );
    }
    let args = Args {
        command: Command::Prepare(PrepareArgs {
            url: None,
            body: None,
            event: None,
            commit: None,
            body_file: Some(root.path().join("missing")),
            draft_file: None,
        }),
    };
    assert!(github_args::words(args).is_err());
}
