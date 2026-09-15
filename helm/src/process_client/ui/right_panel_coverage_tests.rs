use super::*;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn disabled_controls_never_gain_hover_or_selection_style() {
    let rect = Rect::new(2, 3, 8, 1);
    for selected in [false, true] {
        assert_eq!(
            control_style(Some(Position::new(3, 3)), rect, selected, false),
            crate::theme::Role::Muted.style()
        );
        let base = if selected {
            crate::theme::Role::Selection.style()
        } else {
            crate::theme::Role::Focus.style()
        };
        assert_eq!(control_style(None, rect, selected, true), base);
        assert_eq!(
            control_style(Some(Position::new(10, 3)), rect, selected, true),
            base
        );
        assert_eq!(
            control_style(Some(Position::new(2, 3)), rect, selected, true),
            base.patch(crate::theme::Role::Hover.style())
        );
    }
}

#[test]
fn buttons_wrap_whole_labels_and_publish_only_enabled_visible_hits() {
    let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
    let mut hits = Vec::new();
    terminal
        .draw(|frame| {
            hits = buttons(
                frame,
                Rect::new(2, 1, 12, 2),
                None,
                &[
                    ("[One]", 1, true),
                    ("[Two]", 2, false),
                    ("[三]", 3, true),
                    ("far too long for area", 4, true),
                    ("[End]", 5, true),
                ],
            );
        })
        .unwrap();
    assert_eq!(
        hits,
        vec![(Rect::new(2, 1, 5, 1), 1), (Rect::new(2, 2, 4, 1), 3)]
    );
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("[One] [Two]"));
    assert!(!text.contains("[End]"));
    terminal
        .draw(|f| assert!(buttons(f, Rect::new(0, 0, 0, 0), None, &[("x", 1, true)]).is_empty()))
        .unwrap();
}
