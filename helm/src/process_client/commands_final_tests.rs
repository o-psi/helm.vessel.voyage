use super::{admin::AdminCommand as A, cli::ConnectedCommand as C, transport::Client};
#[path = "../dispatch_fixture_final_tests.rs"]
// Each consumer gets an independent scripted fixture module.
#[allow(clippy::duplicate_mod)]
mod fixture;
use fixture::*;
use serde_json::json;
use uuid::Uuid;
use voyage_protocol::vessel::{VesselCommand as V, VoyageCommand as Q};

#[tokio::test]
async fn connected_cli_dispatches_exact_read_and_mutation_identities() {
    let s = Uuid::new_v4();
    let i = Uuid::from_u128(2);
    let c = Uuid::new_v4();
    let r = Uuid::new_v4();
    let cases = vec![
        (C::Inspect { session: s }, Q::Snapshot),
        (
            C::Submit {
                session: s,
                expected_revision: 7,
                command_id: c,
                expires_at_ms: 900,
                prompt: vec!["one".into(), "two".into()],
            },
            Q::Submit {
                coordination: None,
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
                prompt: "one two".into(),
            },
        ),
        (
            C::Cancel {
                session: s,
                run: r,
                expected_revision: 7,
                command_id: c,
                expires_at_ms: 900,
            },
            Q::Cancel {
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
                run_id: r,
            },
        ),
        (
            C::Receipt {
                session: s,
                command_id: c,
            },
            Q::Receipt { command_id: c },
        ),
        (
            C::Archive {
                session: s,
                restore: false,
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
            },
            Q::Archive {
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
                archived: true,
            },
        ),
        (
            C::Archive {
                session: s,
                restore: true,
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
            },
            Q::Archive {
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
                archived: false,
            },
        ),
        (
            C::Delete {
                session: s,
                confirm_session_id: s,
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
            },
            Q::Delete {
                command_id: c,
                expected_revision: 7,
                expires_at_ms: 900,
                confirm_session_id: s,
            },
        ),
        (
            C::Request {
                session: s,
                request: serde_json::to_string(&Q::Decisions).unwrap(),
            },
            Q::Decisions,
        ),
    ];
    for (command, operation) in cases {
        let mut expected = voyage(s, i, operation);
        expected.as_object_mut().unwrap().remove("incarnation");
        let peer = Peer::new(vec![
            (wire(V::Inspect { session_id: s }), info(s, i)),
            (expected, json!({"sentinel":42})),
        ])
        .await;
        assert_eq!(
            super::commands::execute(&peer.client, command)
                .await
                .unwrap(),
            json!({"sentinel":42})
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn direct_lifecycle_and_admin_dispatch_preserve_arguments() {
    let s = Uuid::new_v4();
    let i = Uuid::new_v4();
    let c = Uuid::new_v4();
    let r = Uuid::new_v4();
    let cases = vec![
        (C::List, V::Catalogue),
        (
            C::Stop {
                session: s,
                incarnation: i,
            },
            V::Stop {
                session_id: s,
                incarnation: i,
            },
        ),
        (
            C::Restart {
                session: s,
                incarnation: i,
                command_id: c,
            },
            V::Restart {
                session_id: s,
                incarnation: i,
                command_id: c,
            },
        ),
        (
            C::Import {
                session: s,
                command_id: c,
                workspace: "/destination".into(),
                source_directory: "/source".into(),
                expected_revision: 9,
                source_sha256: "digest".into(),
                config_path: None,
            },
            V::Import {
                session_id: s,
                command_id: c,
                workspace: "/destination".into(),
                source_directory: "/source".into(),
                expected_revision: 9,
                source_sha256: "digest".into(),
                config_path: None,
            },
        ),
        (
            C::Admin {
                command: A::Identity,
            },
            V::Identity,
        ),
        (
            C::Admin {
                command: A::RemoveParticipant {
                    binding_id: r,
                    command_id: c,
                    expected_revision: 9,
                    cancel: true,
                },
            },
            V::RemoveParticipant {
                binding_id: r,
                command_id: c,
                expected_revision: 9,
                cancel: true,
            },
        ),
        (
            C::Admin {
                command: A::Recover {
                    session: s,
                    incarnation: i,
                    command_id: c,
                    acknowledge_cleanup: Some(r),
                    reconcile_tools: Some(r),
                    expected_revision: Some(9),
                    acknowledge_resources: vec![r],
                },
            },
            V::Recover {
                session_id: s,
                incarnation: i,
                command_id: c,
                acknowledge_cleanup: Some(r),
                reconcile_tools: Some(r),
                expected_revision: Some(9),
                acknowledge_resources: vec![r],
            },
        ),
    ];
    for (command, expected) in cases {
        let peer = Peer::new(vec![(wire(expected), json!({"ok":true}))]).await;
        assert_eq!(
            super::commands::execute(&peer.client, command)
                .await
                .unwrap(),
            json!({"ok":true})
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn new_branch_and_assignment_use_observed_owner() {
    let s = Uuid::new_v4();
    let i = Uuid::new_v4();
    let c = Uuid::new_v4();
    let b = Uuid::new_v4();
    for configured in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let config_path = configured.then(|| std::path::PathBuf::from("/host/config"));
        let expected = if let Some(path) = &config_path {
            V::StartConfigured {
                command_id: c,
                session_id: s,
                workspace: workspace.path().canonicalize().unwrap(),
                config_path: path.clone(),
            }
        } else {
            V::Start {
                command_id: c,
                session_id: s,
                workspace: workspace.path().canonicalize().unwrap(),
            }
        };
        let peer = Peer::new(vec![(wire(expected), info(s, i))]).await;
        super::commands::execute(
            &peer.client,
            C::New {
                id: Some(s),
                command_id: Some(c),
                workspace: workspace.path().into(),
                config_path,
            },
        )
        .await
        .unwrap();
        peer.finish().await;
    }
    let peer = Peer::new(vec![
        (wire(V::Inspect { session_id: s }), info(s, i)),
        (
            wire(V::Branch {
                command_id: c,
                session_id: s,
                incarnation: i,
                expected_revision: 9,
                expires_at_ms: 900,
                branch_id: b,
                name: Some("fork".into()),
                through_message: None,
            }),
            json!({}),
        ),
    ])
    .await;
    super::commands::execute(
        &peer.client,
        C::Branch {
            session: s,
            branch_id: b,
            command_id: c,
            expected_revision: 9,
            expires_at_ms: 900,
            name: Some("fork".into()),
        },
    )
    .await
    .unwrap();
    peer.finish().await;
    let peer = Peer::new(vec![
        (wire(V::Inspect { session_id: s }), info(s, i)),
        (
            voyage(
                s,
                i,
                Q::AssignmentObserve {
                    run_id: c,
                    assignment_id: b,
                    participant: "worker".into(),
                    cancel: true,
                },
            ),
            json!({"observed":true}),
        ),
    ])
    .await;
    let result = super::admin::execute(
        &peer.client,
        A::Assignment {
            session: s,
            run: c,
            assignment: b,
            participant: "worker".into(),
            cancel: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(result["observed"], true);
    peer.finish().await;
}

#[tokio::test]
async fn invalid_requests_fail_before_network_and_tombstone_receipts_are_local() {
    let client = Client::local(std::path::PathBuf::from("/nonexistent-final-fixture"));
    for request in ["{".to_owned(), "x".repeat(256 * 1024 + 1)] {
        assert!(
            super::commands::execute(
                &client,
                C::Request {
                    session: Uuid::new_v4(),
                    request
                }
            )
            .await
            .is_err()
        );
    }
    let s = Uuid::new_v4();
    let i = Uuid::new_v4();
    let c = Uuid::new_v4();
    let receipt = json!({"command_id":c,"deleted":true});
    let mut process = info(s, i);
    process["deletion"] = receipt.clone();
    let peer = Peer::new(vec![(wire(V::Inspect { session_id: s }), process)]).await;
    assert_eq!(
        super::commands::execute(
            &peer.client,
            C::Receipt {
                session: s,
                command_id: c
            }
        )
        .await
        .unwrap(),
        receipt
    );
    peer.finish().await;
}
