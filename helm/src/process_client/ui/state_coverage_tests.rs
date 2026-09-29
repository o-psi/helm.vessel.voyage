use super::super::coverage_support;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn rendered_sidebar(app: &crate::process_client::ui::App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| crate::process_client::ui::render::draw(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn generated_names_require_a_nonempty_hex_suffix() {
    for name in ["", "session-012aBF"] {
        assert!(default_name(name));
    }
    for name in ["session-", "session-xyz", "Session-ab", "my voyage"] {
        assert!(!default_name(name));
    }
}

#[tokio::test]
async fn title_prefers_snapshot_then_process_and_skips_operator_prompts() {
    let (_fixture, mut app, target) = coverage_support::app();
    let view = app.views.get_mut(&target).unwrap();
    assert_eq!(view.title(), "Synthetic voyage");
    view.snapshot.as_mut().unwrap().name = Some("renamed".into());
    assert_eq!(view.title(), "renamed");
    view.snapshot.as_mut().unwrap().name = Some("session-abc".into());
    view.snapshot.as_mut().unwrap().messages = serde_json::from_value(json!([
        {"role":"assistant","content":"not a title"},
        {"role":"user","content":"Operator tool ignored"},
        {"role":"user","content":"one two three four five six seven eight\nignored"}
    ]))
    .unwrap();
    assert_eq!(view.title(), "one two three four five six seven");
}

#[tokio::test]
async fn title_fallback_distinguishes_loading_saved_and_unavailable() {
    let (_fixture, mut app, target) = coverage_support::app();
    let view = app.views.get_mut(&target).unwrap();
    view.process.name = None;
    assert!(view.title().starts_with("Saved voyage "));
    view.snapshot = None;
    assert!(view.title().starts_with("Loading voyage "));
    view.connection_unavailable = true;
    assert!(view.title().starts_with("Unavailable voyage "));
    view.connection_unavailable = false;
    view.error = Some("offline".into());
    assert!(view.title().starts_with("Unavailable voyage "));
}

#[tokio::test]
async fn lifecycle_and_route_filters_control_navigation() {
    let (_fixture, mut app, target) = coverage_support::app();
    assert_eq!(app.ordered_targets(), vec![target]);
    app.vessel_filter = Some(Uuid::new_v4());
    assert!(app.ordered_targets().is_empty());
    app.vessel_filter = Some(target.route.id);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle = json!({"archived":true});
    assert!(app.views[&target].archived());
    assert!(app.ordered_targets().is_empty());
    app.archives = true;
    assert_eq!(app.ordered_targets(), vec![target]);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle["deleted"] = json!(true);
    assert!(app.views[&target].deleted());
    assert!(app.ordered_targets().is_empty());
}

#[tokio::test]
async fn settled_voyages_have_one_labeled_section_after_active_voyages() {
    let (_fixture, mut app, active) = coverage_support::app();
    let settled = Target {
        session: Uuid::new_v4(),
        ..active
    };
    let mut view = View::new(app.views[&active].process.clone());
    view.snapshot = app.views[&active].snapshot.clone();
    view.process.session_id = settled.session;
    view.snapshot.as_mut().unwrap().session_id = settled.session;
    view.snapshot.as_mut().unwrap().run =
        serde_json::from_value(json!({"run_id":Uuid::new_v4(),"state":"completed"})).unwrap();
    view.observe_settlement(app.presentation_now);
    app.settle_after_secs = 0;
    app.views.insert(settled, view);

    let targets = app.ordered_targets();
    assert_eq!(targets, vec![active, settled]);
    assert_eq!(
        crate::process_client::ui::render::settled_section_start(&app, &targets),
        Some(1)
    );
    let screen = rendered_sidebar(&app);
    assert!(screen.contains("Settled"), "{screen}");
    assert_eq!(
        app.sidebar
            .hits
            .borrow()
            .iter()
            .map(|hit| hit.target)
            .collect::<Vec<_>>(),
        targets
    );

    app.views.remove(&active);
    assert_eq!(
        crate::process_client::ui::render::settled_section_start(&app, &[settled]),
        Some(0)
    );
    assert!(rendered_sidebar(&app).contains("Settled conversations"));
    app.archives = true;
    assert_eq!(
        crate::process_client::ui::render::settled_section_start(&app, &[settled]),
        None
    );
}
