//! Owned App/task/image fixtures; never acquire the native clipboard or send inference.
use super::super::{coverage_support, right_panel::Editor, socket_support_tests::Server};
use super::*;
use base64::Engine as _;
use crossterm::event::KeyEvent;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn image() -> attachments::Image {
    let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    attachments::Image::from_bytes("owned-pixel.png".into(), &bytes).unwrap()
}
fn images(app: &mut App, target: Target) {
    let view = app.views.get_mut(&target).unwrap();
    view.draft.set_text("owned text before after".into());
    view.draft.cursor = 18;
    attachments::insert_images(&mut view.draft, &mut view.images, vec![image()], 18).unwrap();
}
fn rename(app: &mut App, target: Target) -> (Editor, String) {
    app.open_actions(target);
    let incarnation = app.views[&target].process.incarnation;
    app.sidebar.visible.set(Some((target, incarnation, None)));
    app.sidebar_input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();
    app.sidebar
        .visible
        .set(Some((target, incarnation, Some(sidebar::Action::Rename))));
    app.panel_editor().expect("owned rename field")
}
fn ready(
    app: &mut App,
    destination: Destination,
    panel: Option<(Editor, String)>,
    value: Result<Prepared, String>,
) -> Uuid {
    let id = Uuid::new_v4();
    let (sender, result) = tokio::sync::oneshot::channel();
    sender.send(value).ok().unwrap();
    app.clipboard_pending = Some(PendingPaste {
        destination,
        id,
        panel,
        cancel: CancellationToken::new(),
        task: tokio::spawn(async {}),
        result,
    });
    id
}

#[tokio::test]
async fn panel_text_is_staged_sanitized_and_never_dispatches_or_changes_composer() {
    let (_fixture, mut app, target) = coverage_support::app();
    let (editor, before) = rename(&mut app, target);
    let draft = app.views[&target].draft.text.clone();
    ready(
        &mut app,
        Destination::Live(target),
        Some((editor, before.clone())),
        Ok(Prepared::Text("\r\nowned\x1b[31m".into())),
    );
    app.poll_clipboard();
    let (same, after) = app.panel_editor().unwrap();
    assert_eq!(same.target(), target);
    assert!(after.starts_with(&before) && after.contains("owned"));
    assert!(!after.contains('\n') && !after.contains('\r') && !after.contains('\x1b'));
    assert_eq!(app.views[&target].draft.text, draft);
    assert!(app.views[&target].pending.is_none() && app.command_checks.is_empty());
    assert!(app.status.contains("review before confirming"));
    app.finish_clipboard().await.unwrap();
}

#[tokio::test]
async fn panel_late_result_refuses_changed_field_owner_menu_cancel_nontext_and_limit() {
    for case in 0..7 {
        let (_fixture, mut app, target) = coverage_support::app();
        let (editor, before) = rename(&mut app, target);
        let result = if case == 4 {
            Ok(Prepared::Images(vec![image()]))
        } else if case == 5 {
            Err("owned cleanup failed".into())
        } else {
            Ok(Prepared::Text(if case == 6 {
                "x".repeat(257)
            } else {
                "must not apply".into()
            }))
        };
        let id = ready(
            &mut app,
            Destination::Live(target),
            Some((editor, before.clone())),
            result,
        );
        match case {
            0 => {
                app.sidebar_input(&Event::Paste("changed field".into()))
                    .unwrap();
            }
            1 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            2 => app.sidebar.menu = None,
            3 => app.clipboard_pending.as_ref().unwrap().cancel.cancel(),
            _ => (),
        }
        let expected = app.panel_editor().map(|(_, value)| value);
        app.paste_result(
            Uuid::new_v4(),
            Ok(Prepared::Text("foreign observation".into())),
        );
        assert_eq!(app.clipboard_pending.as_ref().unwrap().id, id);
        app.poll_clipboard();
        assert!(app.clipboard_pending.is_none());
        assert_eq!(app.panel_editor().map(|(_, value)| value), expected);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none() && app.command_checks.is_empty());
        assert_eq!(app.clipboard_blocked, case == 5);
        assert!(app.status.starts_with("Paste failed:"));
    }
}

#[tokio::test]
async fn panel_input_cancels_only_matching_owned_panel_reads() {
    for panel in [false, true] {
        for event in [
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            Event::Paste("owned".into()),
            Event::Resize(80, 24),
            Event::FocusGained,
            Event::Mouse(crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            Event::Mouse(crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
        ] {
            let (_fixture, mut app, target) = coverage_support::app();
            let editor = panel.then(|| rename(&mut app, target));
            ready(
                &mut app,
                Destination::Live(target),
                editor,
                Ok(Prepared::Empty),
            );
            app.cancel_panel_paste_on_input(&event);
            assert_eq!(
                app.clipboard_pending
                    .as_ref()
                    .unwrap()
                    .cancel
                    .is_cancelled(),
                panel
                    && !matches!(
                        event,
                        Event::FocusGained
                            | Event::Mouse(crossterm::event::MouseEvent {
                                kind: crossterm::event::MouseEventKind::Moved,
                                ..
                            })
                    )
            );
            app.cancel_paste_for_private_panel();
            assert!(
                app.clipboard_pending
                    .as_ref()
                    .unwrap()
                    .cancel
                    .is_cancelled()
            );
            app.finish_clipboard().await.unwrap();
        }
    }
}

#[tokio::test]
async fn finish_waits_for_actual_owned_task_retirement_and_clears_composer_anchor() {
    for panel in [false, true] {
        let (_fixture, mut app, target) = coverage_support::app();
        let editor = panel.then(|| rename(&mut app, target));
        if !panel {
            app.views.get_mut(&target).unwrap().draft.set_paste_anchor();
        }
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let retired = Arc::new(AtomicUsize::new(0));
        let witness = retired.clone();
        let (sender, result) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            token.cancelled().await;
            witness.fetch_add(1, Ordering::SeqCst);
            sender.send(Err("owned cancelled read".into())).ok();
        });
        app.clipboard_pending = Some(PendingPaste {
            destination: Destination::Live(target),
            id: Uuid::new_v4(),
            panel: editor,
            cancel,
            task,
            result,
        });
        app.poll_clipboard();
        assert!(app.clipboard_pending.is_some());
        app.finish_clipboard().await.unwrap();
        assert_eq!(retired.load(Ordering::SeqCst), 1);
        assert!(app.clipboard_pending.is_none());
        assert!(
            app.views
                .get_mut(&target)
                .unwrap()
                .draft
                .take_paste_range()
                .is_none()
        );
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[tokio::test]
async fn unexpected_reader_close_and_cleanup_failure_block_future_native_acquisition() {
    for case in 0..3 {
        let (_fixture, mut app, target) = coverage_support::app();
        app.views.get_mut(&target).unwrap().draft.set_paste_anchor();
        let (sender, result) = tokio::sync::oneshot::channel();
        drop(sender);
        app.clipboard_pending = Some(PendingPaste {
            destination: Destination::Live(target),
            id: Uuid::new_v4(),
            panel: None,
            cancel: CancellationToken::new(),
            task: tokio::spawn(async {}),
            result,
        });
        if case == 0 {
            app.poll_clipboard();
            assert!(app.clipboard_blocked);
        } else if case == 1 {
            let pending = app.clipboard_pending.take().unwrap();
            ready(
                &mut app,
                pending.destination,
                None,
                Err("owned helper cleanup failed".into()),
            );
            assert!(app.finish_clipboard().await.is_err());
        } else {
            app.clipboard_pending.as_mut().unwrap().task =
                tokio::spawn(async { panic!("owned task failure") });
            assert!(app.finish_clipboard().await.is_err());
        }
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none());
        if case == 0 {
            assert!(app.paste_clipboard_action().is_err());
        }
    }
}

#[tokio::test]
async fn image_paste_retains_order_and_refuses_limits_without_losing_original_text() {
    for over_limit in [false, true] {
        let (_fixture, mut app, target) = coverage_support::app();
        images(&mut app, target);
        let original = app.views[&target].draft.text.clone();
        let original_images = app.views[&target].images.clone();
        app.views.get_mut(&target).unwrap().draft.set_paste_anchor();
        let added = (0..if over_limit { 4 } else { 1 })
            .map(|_| image())
            .collect();
        ready(
            &mut app,
            Destination::Live(target),
            None,
            Ok(Prepared::Images(added)),
        );
        app.poll_clipboard();
        assert_eq!(
            app.views[&target].draft.authored_text(),
            "owned text before after"
        );
        assert!(app.views[&target].pending.is_none());
        if over_limit {
            assert_eq!(app.views[&target].draft.text, original);
            assert_eq!(app.views[&target].images, original_images);
            assert!(app.status.starts_with("Paste failed:"));
        } else {
            assert_eq!(app.views[&target].images.len(), 2);
            assert_eq!(app.views[&target].images[0], original_images[0]);
        }
    }
}

#[tokio::test]
async fn image_send_admission_refusal_matrix_keeps_private_draft_and_has_no_dispatch() {
    for case in 0..9 {
        let (_fixture, mut app, target) = coverage_support::app();
        images(&mut app, target);
        let text = app.views[&target].draft.text.clone();
        let private = app.views[&target].images.clone();
        match case {
            0 => app.clients.mark_unavailable(target.route),
            1 => {
                ready(
                    &mut app,
                    Destination::Live(target),
                    None,
                    Ok(Prepared::Empty),
                );
            }
            2 => {
                let incarnation = app.views[&target].process.incarnation;
                app.views.get_mut(&target).unwrap().pending = Some(state::Pending {
                    account_host: None,
                    command_id: Uuid::new_v4(),
                    incarnation,
                    draft: "earlier receipt".into(),
                    preserve_draft: true,
                    original: None,
                    receipt_only: true,
                })
            }
            3 => app.views.get_mut(&target).unwrap().snapshot = None,
            4 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .run = Some(
                    serde_json::from_value(json!({"run_id":Uuid::new_v4(),"state":"running"}))
                        .unwrap(),
                )
            }
            5 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .recovery_pending = true
            }
            6 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .pending_cleanup_run = Some(Uuid::new_v4())
            }
            7 => {
                app.views.get_mut(&target).unwrap().process.deletion = Some(json!({"deleted":true}))
            }
            _ => {
                app.views.get_mut(&target).unwrap().process.archive =
                    Some(voyage_protocol::process::ArchivedVoyage {
                        name: None,
                        revision: 17,
                        receipt: json!({"archived":true}),
                    })
            }
        }
        let previous = app.views[&target]
            .pending
            .as_ref()
            .map(|pending| pending.command_id);
        assert!(app.send_image_turn(target).is_err());
        assert_eq!(app.views[&target].draft.text, text);
        assert_eq!(app.views[&target].images, private);
        assert_eq!(
            app.views[&target]
                .pending
                .as_ref()
                .map(|pending| pending.command_id),
            previous
        );
        assert!(app.command_checks.is_empty());
        assert!(app.views[&target].transcript.borrow().delivery.is_none());
        app.finish_clipboard().await.unwrap();
    }
}

#[tokio::test]
async fn image_send_receipt_storage_refuses_before_wire_and_keeps_unrelated_sentinel() {
    let (fixture, mut app, target) = coverage_support::app();
    images(&mut app, target);
    let sentinel = fixture.0.path().join("helm-command-receipts");
    std::fs::write(&sentinel, b"unrelated owned sentinel").unwrap();
    let text = app.views[&target].draft.text.clone();
    let private = app.views[&target].images.clone();
    assert!(
        app.send_image_turn(target)
            .unwrap_err()
            .to_string()
            .contains("nothing sent")
    );
    assert!(app.views[&target].pending.is_none() && app.command_checks.is_empty());
    assert_eq!(app.views[&target].draft.text, text);
    assert_eq!(app.views[&target].images, private);
    assert!(app.views[&target].transcript.borrow().delivery.is_none());
    assert_eq!(
        std::fs::read(sentinel).unwrap(),
        b"unrelated owned sentinel"
    );
}

#[tokio::test]
async fn image_send_freezes_exact_public_parts_and_upload_refusal_never_submits() {
    let mut server = Server::new(|command| {
        let voyage_protocol::vessel::VesselCommand::Voyage(request) = command else {
            panic!("only exact image dispatch")
        };
        assert!(matches!(request.command, VoyageCommand::UploadImage { .. }));
        Err("owned upload refusal".into())
    })
    .await;
    let (fixture, mut app, old) = coverage_support::app();
    let route = app.clients.insert(server.client.clone());
    let target = Target {
        route,
        session: server.target.session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process.session_id = target.session;
    view.process.incarnation = server.incarnation;
    view.snapshot.as_mut().unwrap().session_id = target.session;
    app.views.insert(target, view);
    app.selected = Some(target);
    images(&mut app, target);
    let text = app.views[&target].draft.text.clone();
    let private = app.views[&target].images.clone();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    app.sender = sender;
    app.send_image_turn(target).unwrap();
    let pending = app.views[&target].pending.as_ref().unwrap();
    let id = pending.command_id;
    let original = pending.original.as_ref().unwrap();
    let VoyageCommand::SubmitContent {
        command_id,
        expected_revision,
        content,
        ..
    } = original.as_ref()
    else {
        panic!("ordered image submission")
    };
    assert_eq!(*command_id, id);
    assert_eq!(*expected_revision, 17);
    let authored = content
        .iter()
        .filter_map(|part| match part {
            voyage_protocol::content::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(authored, "owned text before after");
    assert_eq!(
        content,
        &attachments::content(&app.views[&target].draft, &private).unwrap()
    );
    let serialized = serde_json::to_string(original).unwrap();
    let encoded = serde_json::to_value(&private[0]).unwrap()["data_base64"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!serialized.contains(&encoded));
    let mut restored_view = state::View::new(app.views[&target].process.clone());
    receipts::load(&app.clients[target.route], &mut restored_view).unwrap();
    let restored = restored_view.pending.unwrap();
    assert_eq!(restored.command_id, id);
    assert_eq!(
        serde_json::to_value(restored.original).unwrap(),
        serde_json::to_value(pending.original.clone()).unwrap()
    );
    let update = tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let super::super::observe::Update::Command {
        command_id, result, ..
    } = update
    else {
        panic!("exact command update")
    };
    assert_eq!(command_id, id);
    assert_eq!(result.unwrap()["status"], "rejected");
    assert!(server.requests.try_recv().is_ok());
    assert!(server.requests.try_recv().is_err());
    assert_eq!(app.views[&target].draft.text, text);
    assert_eq!(app.views[&target].images, private);
    assert_eq!(app.views[&target].pending.as_ref().unwrap().command_id, id);
    assert!(!fixture.0.path().join("sessions").exists());
}
