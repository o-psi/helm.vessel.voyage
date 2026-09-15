use super::super::super::coverage_support;
use super::*;
use serde_json::json;

#[test]
fn json_body_uses_literal_fences_and_trims_trailing_blank_lines() {
    for source in ["{\"key\":\"```\"}", "[1,2,3]", "normal **markdown**\n\n"] {
        let text = body(source, 30);
        assert!(!text.lines.is_empty());
        assert!(!text.lines.last().unwrap().to_string().trim().is_empty());
    }
}

#[tokio::test]
async fn transcript_renders_saved_roles_and_reflows_at_narrow_widths() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .messages = serde_json::from_value(json!([
        {"role":"user","content":"synthetic user question"},
        {"role":"assistant","content":"synthetic assistant answer"},
        {"role":"system","content":"synthetic system note"}
    ]))
    .unwrap();
    for (width, height) in [(100, 30), (35, 12), (1, 1)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, &app, frame.area()))
            .unwrap();
        if width == 100 {
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("synthetic assistant answer"));
        }
    }
}
