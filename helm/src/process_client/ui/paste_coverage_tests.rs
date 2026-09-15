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
async fn plain_paste_is_saved_and_preserves_existing_composer() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert!(app.paste_input(&Event::Paste("\r\nnext".into())).unwrap());
    assert_eq!(app.views[&target].draft.text, "preserved draft\nnext");
    assert!(app.ensure_paste_finished(Destination::Live(target)).is_ok());
    assert!(app.clipboard_pending.is_none());
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
