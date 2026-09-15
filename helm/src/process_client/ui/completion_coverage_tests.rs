use super::super::coverage_support;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn draft(app: &mut App, target: Target, text: &str) {
    let view = app.views.get_mut(&target).unwrap();
    view.draft = Default::default();
    view.draft.insert_str(text);
    app.completion = Default::default();
}
#[test]
fn command_navigation_completion_and_dismissal_only_edit_drafts() {
    let (_fixture, mut app, target) = coverage_support::app();
    draft(&mut app, target, "/cle");
    assert_eq!(app.completion_menu().unwrap().entries[0].0, "/clear ");
    assert!(
        app.completion_input(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap()
    );
    assert_eq!(app.views[&target].draft.text, "/clear ");
    assert!(app.views[&target].pending.is_none());
    draft(&mut app, target, "/");
    let count = app.completion_menu().unwrap().entries.len();
    app.completion_input(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.completion.selected, count - 1);
    app.completion_input(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.completion.selected, 0);
    assert!(
        !app.completion_input(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap()
    );
    app.completion_input(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.completion_menu().is_none());
    app.sync_completion();
    assert!(app.completion_menu().is_some());
}
#[test]
fn completion_is_hidden_for_noncomposer_contexts_and_midline_edits() {
    let (_fixture, mut app, target) = coverage_support::app();
    for text in ["ordinary text", "/help\n/clear"] {
        draft(&mut app, target, text);
        assert!(app.completion_menu().is_none());
    }
    draft(&mut app, target, "/cle");
    app.views.get_mut(&target).unwrap().draft.cursor = 1;
    assert!(app.completion_menu().is_none());
    app.views.get_mut(&target).unwrap().draft.cursor = 4;
    app.help = true;
    assert!(app.completion_menu().is_none());
    app.help = false;
    app.sidebar.focus = super::super::sidebar::Focus::Voyages;
    assert!(app.completion_menu().is_none());
    app.sidebar.focus = super::super::sidebar::Focus::Composer;
    assert!(app.completion_menu().is_some());
    draft(&mut app, target, "/not-a-command");
    assert!(app.completion_menu().unwrap().entries.is_empty());
    assert!(
        app.completion_input(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap()
    );
    assert_eq!(app.views[&target].draft.text, "/not-a-command");
}
#[test]
fn metadata_completion_filters_unsafe_tokens_and_fences_identity() {
    let (_fixture, mut app, target) = coverage_support::app();
    let incarnation = app.views[&target].process.incarnation;
    for (command, section, value, expected) in [
        (
            "model",
            "models",
            json!([{"id":"good-model","display_name":"Friendly"},{"id":"two words"},{"id":"bad\u{1b}"}]),
            "/model good-model",
        ),
        (
            "tool",
            "tools",
            json!({"inventory":[{"name":"read_file","description":"Read"},{"name":"invalid tool"}]}),
            "/tool read_file ",
        ),
        (
            "terminal",
            "terminals",
            json!([{"id":"terminal-id","title":"Synthetic program"}]),
            "/terminal terminal-id",
        ),
    ] {
        draft(&mut app, target, &format!("/{command} "));
        app.completion.metadata = Some(Metadata {
            target,
            incarnation,
            section,
            value: None,
            failed: false,
        });
        assert!(app.completion_menu().unwrap().hint.starts_with("Loading "));
        app.completion_update(target, Uuid::new_v4(), section, Some(value.clone()));
        assert!(app.completion.metadata.as_ref().unwrap().value.is_none());
        app.completion_update(target, incarnation, "wrong-section", Some(value.clone()));
        assert!(app.completion.metadata.as_ref().unwrap().value.is_none());
        app.completion_update(target, incarnation, section, Some(value));
        let entries = app.completion_menu().unwrap().entries;
        assert_eq!(entries.len(), if command == "model" { 2 } else { 1 });
        assert_eq!(entries[0].0, expected);
        app.completion_update(target, incarnation, section, None);
        assert!(app.completion_menu().unwrap().hint.contains("unavailable"));
    }
}
#[test]
fn builtin_argument_choices_and_overlay_content() {
    let (_fixture, mut app, target) = coverage_support::app();
    for (text, expected) in [
        ("/access r", "/access read-only"),
        ("/thinking in", "/thinking inherit"),
        ("/service in", "/service inherit"),
    ] {
        draft(&mut app, target, text);
        assert!(
            app.completion_menu()
                .unwrap()
                .entries
                .iter()
                .any(|(s, _)| s == expected)
        );
    }
    draft(&mut app, target, "/cle");
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    terminal
        .draw(|f| app.draw_completion(f, Rect::new(0, 0, 80, 15)))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Commands"));
    assert!(text.contains("/clear"));
    assert!(text.contains("Tab complete"));
}
#[test]
fn paths_are_local_sorted_and_respect_hidden_and_directory_filters() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    std::fs::write(dir.path().join("file"), "synthetic").unwrap();
    std::fs::write(dir.path().join(".hidden"), "synthetic").unwrap();
    let prefix = format!("{}/", dir.path().display());
    let all = path_options(&prefix, false, true);
    assert_eq!(all.len(), 2);
    assert!(all[0].0.ends_with("/file"));
    assert!(all[1].0.ends_with("/folder/"));
    let dirs = path_options(&prefix, true, true);
    assert_eq!(dirs.len(), 1);
    assert!(dirs[0].0.ends_with("/folder/"));
    assert_eq!(path_options(&format!("{prefix}."), false, true).len(), 1);
    assert!(path_options("relative", false, true).is_empty());
    assert!(path_options(&format!("{prefix}missing/"), false, true).is_empty());
}
