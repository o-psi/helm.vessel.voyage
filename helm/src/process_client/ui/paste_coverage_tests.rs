use super::super::coverage_support;
use super::*;

#[test]
fn raster_extensions_are_case_insensitive_but_not_arbitrary_files() {
    for name in ["a.PNG", "a.jpg", "a.JPEG", "a.webp"] {
        assert!(raster_path(std::path::Path::new(name)));
    }
    for name in ["png", "a.svg", "a.gif", "a.png.txt", ""] {
        assert!(!raster_path(std::path::Path::new(name)));
    }
}

#[test]
fn text_preparation_normalizes_newlines_and_keeps_missing_paths_as_text() {
    for (input, expected) in [
        ("a\r\nb\rc", "a\nb\nc"),
        (
            "/missing-coverage-fixture/image.png",
            "/missing-coverage-fixture/image.png",
        ),
    ] {
        let Prepared::Text(text) =
            prepare_content(crate::clipboard::Content::Text(input.into()), None).unwrap()
        else {
            panic!("expected text")
        };
        assert_eq!(text, expected);
    }
}

#[test]
fn preparation_rejects_oversize_text_and_invalid_image_material() {
    assert!(prepare_content(crate::clipboard::Content::Text("x".repeat(65537)), None).is_err());
    assert!(
        prepare_content(
            crate::clipboard::Content::Image {
                name: "broken.png".into(),
                bytes: vec![0, 1, 2]
            },
            None
        )
        .is_err()
    );
    assert!(matches!(
        prepare_content(crate::clipboard::Content::Empty, None).unwrap(),
        Prepared::Empty
    ));
}

#[test]
fn file_paste_validates_count_type_and_explicit_fallback() {
    assert!(prepare_content(crate::clipboard::Content::Files(vec![]), None).is_err());
    assert!(
        prepare_content(
            crate::clipboard::Content::Files(vec!["a.png".into(); 5]),
            None
        )
        .is_err()
    );
    assert!(prepare_content(crate::clipboard::Content::Files(vec!["a.svg".into()]), None).is_err());
    assert!(
        matches!(prepare_content(crate::clipboard::Content::Files(vec!["/missing-coverage-fixture/a.png".into()]), Some("original".into())).unwrap(), Prepared::Text(text) if text == "original")
    );
}

#[tokio::test]
async fn modal_and_terminal_focus_never_steal_pastes() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(matches!(app.paste_destination(), Some(Destination::Live(t)) if t == target));
    app.help = true;
    assert!(!app.paste_input(&Event::Paste("hidden".into())).unwrap());
    app.help = false;
    app.views.get_mut(&target).unwrap().terminals.open = true;
    assert!(app.paste_destination().is_none());
    app.views.get_mut(&target).unwrap().terminals.open = false;
    app.views.get_mut(&target).unwrap().panel = Some("status".into());
    assert!(app.paste_destination().is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[tokio::test]
async fn plain_paste_stays_in_memory_and_preserves_existing_composer() {
    let (fixture, mut app, target) = coverage_support::app();
    assert!(app.paste_input(&Event::Paste("\r\nnext".into())).unwrap());
    assert_eq!(app.views[&target].draft.text, "preserved draft\nnext");
    assert!(app.ensure_paste_finished(Destination::Live(target)).is_ok());
    assert!(app.clipboard_pending.is_none());
    assert!(!fixture.0.path().join("helm-views").exists());
    assert!(!fixture.0.path().join("helm-command-receipts").exists());
}

#[tokio::test]
async fn rejected_paste_keeps_draft_and_does_not_create_delivery() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(app.paste_input(&Event::Paste("x".repeat(65536))).is_err());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle = serde_json::json!({"archived":true});
    assert!(app.paste_input(&Event::Paste("no".into())).is_err());
    assert!(app.views[&target].pending.is_none());
}

#[tokio::test]
async fn nonpaste_keys_and_releases_do_not_start_clipboard_workers() {
    let (_fixture, mut app, _) = coverage_support::app();
    assert!(
        !app.paste_input(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::NONE
        )))
        .unwrap()
    );
    let mut key = crossterm::event::KeyEvent::new(KeyCode::Char('V'), KeyModifiers::CONTROL);
    key.kind = crossterm::event::KeyEventKind::Release;
    assert!(app.paste_input(&Event::Key(key)).unwrap());
    key.code = KeyCode::Insert;
    key.modifiers = KeyModifiers::SHIFT;
    assert!(app.paste_input(&Event::Key(key)).unwrap());
    assert!(app.clipboard_pending.is_none());
}

#[tokio::test]
async fn unavailable_destinations_return_errors_without_mutation() {
    let (_fixture, mut app, mut target) = coverage_support::app();
    target.session = Uuid::new_v4();
    assert!(app.copy_paste_draft(Destination::Live(target)).is_err());
    assert!(
        app.ensure_paste_editable(Destination::Live(target))
            .is_err()
    );
    assert!(app.paste_composer_mut(Destination::Live(target)).is_none());
    app.selected = None;
    assert!(app.paste_destination().is_none());
}

#[tokio::test]
async fn supplied_paste_preserves_anchor_and_handles_cancellation_cleanup_failure_and_stale_reply()
{
    let (_fixture, mut app, target) = coverage_support::app();
    app.views
        .get_mut(&target)
        .unwrap()
        .draft
        .set_text("before after".into());
    app.views.get_mut(&target).unwrap().draft.cursor = 7;
    app.begin_paste(
        Destination::Live(target),
        Some((crate::clipboard::Content::Text("inserted ".into()), None)),
    )
    .unwrap();
    assert!(
        app.ensure_paste_finished(Destination::Live(target))
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while app.clipboard_pending.is_some() {
            app.poll_clipboard();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(app.views[&target].draft.text, "before inserted after");
    assert!(app.views[&target].pending.is_none());
    app.begin_paste(
        Destination::Live(target),
        Some((crate::clipboard::Content::Empty, None)),
    )
    .unwrap();
    let id = app.clipboard_pending.as_ref().unwrap().id;
    app.paste_result(Uuid::new_v4(), Ok(Prepared::Text("wrong".into())));
    assert!(app.clipboard_pending.is_some());
    app.clipboard_pending.as_ref().unwrap().cancel.cancel();
    app.paste_result(id, Ok(Prepared::Text("cancelled".into())));
    assert!(!app.views[&target].draft.text.contains("cancelled"));
    assert!(app.status.contains("cancelled"));
    app.begin_paste(
        Destination::Live(target),
        Some((crate::clipboard::Content::Empty, None)),
    )
    .unwrap();
    let id = app.clipboard_pending.as_ref().unwrap().id;
    app.paste_result(id, Err("fixture cleanup failed".into()));
    assert!(app.clipboard_blocked);
    assert!(
        app.begin_paste(
            Destination::Live(target),
            Some((crate::clipboard::Content::Empty, None))
        )
        .is_err()
    );
    assert_eq!(app.views[&target].draft.text, "before inserted after");
}
#[tokio::test]
async fn replaced_draft_cannot_receive_old_paste_and_empty_paste_is_non_destructive() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.begin_paste(
        Destination::Live(target),
        Some((crate::clipboard::Content::Empty, None)),
    )
    .unwrap();
    let id = app.clipboard_pending.as_ref().unwrap().id;
    app.views
        .get_mut(&target)
        .unwrap()
        .draft
        .set_text("replacement".into());
    app.paste_result(id, Ok(Prepared::Text("stale".into())));
    assert_eq!(app.views[&target].draft.text, "replacement");
    assert!(app.status.contains("replaced"));
    app.begin_paste(
        Destination::Live(target),
        Some((crate::clipboard::Content::Empty, None)),
    )
    .unwrap();
    let id = app.clipboard_pending.as_ref().unwrap().id;
    app.paste_result(id, Ok(Prepared::Empty));
    assert_eq!(app.views[&target].draft.text, "replacement");
    assert!(app.status.contains("no image or text"));
}
