//! Canonical observations and exact settings receipts never replay remote effects.
use super::*;
use serde_json::json;
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, VoyageCommand};

fn pending(app: &mut App, target: Target, original: Option<VoyageCommand>, preserve: bool) -> Uuid {
    let command_id = original
        .as_ref()
        .and_then(VoyageCommand::mutation_id)
        .unwrap_or_else(Uuid::new_v4);
    let view = app.views.get_mut(&target).unwrap();
    view.pending = Some(state::Pending {
        account_host: None,
        command_id,
        incarnation: view.process.incarnation,
        draft: view.draft.text.clone(),
        preserve_draft: preserve,
        receipt_only: original.is_none(),
        original: original.map(Box::new),
    });
    command_id
}
fn summary(target: Target, revision: u64) -> voyage_protocol::process::CatalogueSummary {
    voyage_protocol::process::CatalogueSummary {
        session_id: target.session,
        revision,
        observation_cursor: revision,
        name: Some("Catalogue name".into()),
        model: "catalogue-model".into(),
        created_at: None,
        last_turn_end: None,
        total_messages: 4,
        run_id: None,
        run_state: None,
        archived: false,
        deleted: false,
        pending_cleanup_run: None,
    }
}
fn with_summary(
    mut process: ProcessInfo,
    summary: voyage_protocol::process::CatalogueSummary,
) -> ProcessInfo {
    process.catalogue = Some(Box::new(voyage_protocol::process::CatalogueMetadata {
        summary: Some(summary),
        observed_at_ms: Some(1),
        stale: false,
        error_code: None,
    }));
    process
}

#[test]
fn account_inference_receipts_require_terminal_status_exact_id_and_preserve_original_settings() {
    for (status, original_status, confirmed) in [
        ("accepted", None, false),
        ("running", None, false),
        ("unknown", None, false),
        ("applied", None, true),
        ("completed", None, true),
        ("failed", None, true),
        ("rejected", None, true),
        ("not_admitted", None, true),
        ("transferred", Some("applied"), true),
        ("transferred", Some("failed"), true),
        ("transferred", Some("accepted"), false),
    ] {
        for account in [false, true] {
            let (_fixture, mut app, target) = coverage_support::app();
            let id = Uuid::new_v4();
            let command = if account {
                VoyageCommand::SetAccountInference {
                    command_id: id,
                    expected_revision: 17,
                    expires_at_ms: 2000000000000,
                    model: "requested-model".into(),
                    reasoning_effort: Some("high".into()),
                    service_tier: None,
                    account: voyage_protocol::accounts::AccountBinding {
                        account_id: Uuid::new_v4(),
                        connection_id: Uuid::new_v4(),
                        identity_generation: 1,
                        connection_revision: 1,
                        transport: voyage_protocol::accounts::Transport::OpenaiResponses,
                    },
                }
            } else {
                VoyageCommand::SetInference {
                    command_id: id,
                    expected_revision: 17,
                    expires_at_ms: 2000000000000,
                    model: "requested-model".into(),
                    reasoning_effort: Some("high".into()),
                    service_tier: None,
                }
            };
            pending(&mut app, target, Some(command), true);
            app.update(Update::Command {
                target,
                command_id: id,
                refused: false,
                result: Ok(
                    json!({"command_id":id,"status":status,"original_status":original_status}),
                ),
            });
            assert_eq!(
                app.views[&target].pending.is_none(),
                confirmed,
                "{status}/{original_status:?}"
            );
            assert_eq!(app.views[&target].draft.text, "preserved draft");
            assert_eq!(
                app.views[&target].snapshot.as_ref().unwrap().model,
                "synthetic-model"
            );
            assert!(app.route_tasks.is_empty());
        }
    }
}

#[test]
fn settings_wrong_receipt_or_route_cannot_resolve_a_frozen_pending_envelope() {
    for retired in [false, true] {
        let (_fixture, mut app, target) = coverage_support::app();
        let id = Uuid::new_v4();
        let original = VoyageCommand::SetInference {
            command_id: id,
            expected_revision: 17,
            expires_at_ms: 2000000000000,
            model: "requested-model".into(),
            reasoning_effort: None,
            service_tier: None,
        };
        pending(&mut app, target, Some(original.clone()), true);
        if retired {
            let client = app.clients[target.route].clone();
            app.clients.insert(client);
        }
        app.update(Update::Command {
            target,
            command_id: id,
            refused: false,
            result: Ok(
                json!({"command_id":if retired {id}else {Uuid::new_v4()},"status":"applied"}),
            ),
        });
        let retained = app.views[&target].pending.as_ref().unwrap();
        assert_eq!(retained.command_id, id);
        assert_eq!(
            serde_json::to_value(retained.original.as_deref()).unwrap(),
            serde_json::to_value(Some(&original)).unwrap()
        );
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert_eq!(
            app.views[&target].snapshot.as_ref().unwrap().model,
            "synthetic-model"
        );
    }
}

#[test]
fn background_setting_receipts_preserve_foreground_selection_status_and_dirty_text() {
    let (_fixture, mut app, target) = coverage_support::app();
    let id = Uuid::new_v4();
    pending(
        &mut app,
        target,
        Some(VoyageCommand::SetInference {
            command_id: id,
            expected_revision: 17,
            expires_at_ms: 2000000000000,
            model: "requested".into(),
            reasoning_effort: None,
            service_tier: None,
        }),
        true,
    );
    app.views
        .get_mut(&target)
        .unwrap()
        .draft
        .set_text("new foreground-safe text".into());
    let elsewhere = Target {
        route: target.route,
        session: Uuid::new_v4(),
    };
    app.selected = Some(elsewhere);
    app.status = "foreground status".into();
    app.update(Update::Command {
        target,
        command_id: id,
        refused: false,
        result: Ok(json!({"command_id":id,"status":"applied"})),
    });
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.selected, Some(elsewhere));
    assert_eq!(app.status, "foreground status");
    assert_eq!(app.views[&target].draft.text, "new foreground-safe text");
}

#[test]
fn wrapped_steering_receipt_uses_only_exact_inner_identity() {
    for matched in [false, true] {
        let (_fixture, mut app, target) = coverage_support::app();
        let id = pending(&mut app, target, None, true);
        app.update(Update::Command {target,command_id:id,refused:false,result:Ok(json!({"command_id":id,"status":"unknown","record":{"request":{"receipt_id":if matched {id}else {Uuid::new_v4()}},"status":"queued"}}))});
        assert_eq!(app.views[&target].pending.is_none(), matched);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.route_tasks.is_empty());
    }
}

#[test]
fn catalogue_lifecycle_confirmation_requires_exact_owner_and_receipt_and_keeps_dirty_composer() {
    for archive in [false, true] {
        for variant in 0..5 {
            let (_fixture, mut app, target) = coverage_support::app();
            let id = pending(&mut app, target, None, variant == 3);
            let mut process = app.views[&target].process.clone();
            process.state = voyage_protocol::process::ProcessState::Stopped;
            if variant == 1 {
                process.incarnation = Uuid::new_v4();
            }
            let receipt =
                json!({"command_id":if variant==2 {Uuid::new_v4()}else {id},"status":"applied"});
            if archive {
                process.archive = Some(voyage_protocol::process::ArchivedVoyage {
                    name: None,
                    revision: 17,
                    receipt,
                });
            } else {
                process.deletion = Some(receipt);
            }
            if variant == 4 {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .draft
                    .set_text("new unrelated draft".into());
            }
            app.update(Update::Catalogue {
                route: target.route,
                processes: vec![process],
            });
            assert_eq!(
                app.views[&target].pending.is_none(),
                !matches!(variant, 1 | 2)
            );
            assert_eq!(
                app.views[&target].draft.text,
                match variant {
                    0 => "",
                    4 => "new unrelated draft",
                    _ => "preserved draft",
                }
            );
            assert!(app.route_tasks.is_empty());
            assert!(app.command_checks.is_empty());
        }
    }
}

#[test]
fn catalogue_projection_never_replaces_selected_full_history_but_hydrates_background_metadata() {
    for selected in [false, true] {
        for revision in [16, 17, 18] {
            let (_fixture, mut app, target) = coverage_support::app();
            if !selected {
                app.selected = None;
            }
            let process = with_summary(
                app.views[&target].process.clone(),
                summary(target, revision),
            );
            app.update(Update::Catalogue {
                route: target.route,
                processes: vec![process],
            });
            let snapshot = app.views[&target].snapshot.as_ref().unwrap();
            let metadata = !selected && revision >= 17;
            assert_eq!(snapshot.catalogue_only, metadata);
            assert_eq!(
                snapshot.model,
                if metadata {
                    "catalogue-model"
                } else {
                    "synthetic-model"
                }
            );
            assert_eq!(app.views[&target].draft.text, "preserved draft");
            assert!(app.route_tasks.is_empty());
        }
    }
}

#[test]
fn catalogue_hydration_and_receipt_failure_are_local_errors_not_remote_dispatch() {
    let (fixture, mut app, target) = coverage_support::app();
    let mut process = app.views[&target].process.clone();
    process.session_id = Uuid::new_v4();
    let other = Target {
        route: target.route,
        session: process.session_id,
    };
    process = with_summary(process, summary(other, 19));
    std::fs::write(
        fixture.0.path().join("helm-command-receipts"),
        b"unrelated file",
    )
    .unwrap();
    app.update(Update::Catalogue {
        route: target.route,
        processes: vec![process],
    });
    let view = &app.views[&other];
    assert!(view.error.as_ref().unwrap().contains("Draft recovery"));
    assert!(view.snapshot.as_ref().unwrap().catalogue_only);
    assert!(view.draft.text.is_empty());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert_eq!(
        std::fs::read(fixture.0.path().join("helm-command-receipts")).unwrap(),
        b"unrelated file"
    );
    assert!(app.route_tasks.is_empty());
}

#[test]
fn snapshot_after_disconnect_retains_cached_state_and_draft_without_claiming_connection() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    app.clients.mark_unavailable(target.route);
    let snapshot = serde_json::from_value(
        json!({"session_id":target.session,"revision":18,"model":"observed-model","messages":[]}),
    )
    .unwrap();
    app.update(Update::Snapshot {
        target,
        incarnation,
        result: Box::new(Ok(snapshot)),
    });
    assert_eq!(app.views[&target].snapshot.as_ref().unwrap().revision, 18);
    assert!(app.views[&target].connection_unavailable);
    assert!(
        app.views[&target]
            .error
            .as_ref()
            .unwrap()
            .contains("unavailable")
    );
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn passive_inbox_attention_never_changes_panel_focus_or_receipt() {
    let (_fixture, mut app, target) = coverage_support::app();
    let id = pending(&mut app, target, None, true);
    app.views.get_mut(&target).unwrap().panel = Some("explicit review".into());
    app.help = true;
    app.update(Update::InboxAttention {
        route: target.route,
        count: 4,
    });
    assert_eq!(app.selected, Some(target));
    assert!(app.help);
    assert_eq!(app.views[&target].panel.as_deref(), Some("explicit review"));
    assert_eq!(app.views[&target].pending.as_ref().unwrap().command_id, id);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.status.contains("4 notification"));
    assert!(app.route_tasks.is_empty());
}
