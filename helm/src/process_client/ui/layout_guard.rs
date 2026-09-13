//! Unsupported geometry is a paused input scope, never an invisible live form.
use super::*;

pub(super) fn supported(width: u16, height: u16) -> bool {
    width >= 40 && height >= 18
}

impl App {
    pub(super) fn guard_small_layout(&mut self, event: &Event) -> bool {
        if let Event::Resize(width, height) = event {
            self.viewport.set(Some((*width, *height)));
            self.sidebar.hits.borrow_mut().clear();
            self.sidebar.visible.set(None);
            self.sidebar.resize.clear();
        }
        if self.viewport.get().is_none_or(|(w, h)| supported(w, h)) {
            return false;
        }
        // This only receives the Helm event stream, not privately attached PTY input.
        // Do not route paste, confirmation, shortcuts or stale pointer hits to forms
        // that cannot be seen. Resizing restores the same focused state.
        if matches!(event, Event::Key(key)
            if key.kind != crossterm::event::KeyEventKind::Release
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q')))
        {
            self.quit = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend};

    #[tokio::test]
    async fn tiny_frame_pauses_hidden_inputs_and_restores_focus_on_resize() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        app.help = true;
        let mut terminal = Terminal::new(TestBackend::new(39, 17)).unwrap();
        terminal
            .draw(|frame| super::super::render::draw(frame, &app))
            .unwrap();
        for event in [
            Event::Paste("/new\nprivate input".into()),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
        ] {
            app.input(event).unwrap();
        }
        assert!(app.help);
        assert!(!app.quit);
        assert!(app.new_drafts.is_empty());
        app.input(Event::Resize(80, 24)).unwrap();
        app.input(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
            .unwrap();
        assert!(!app.help);
    }

    #[tokio::test]
    async fn tiny_layout_detach_does_not_dispatch_a_voyage_action() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        app.input(Event::Resize(12, 4)).unwrap();
        app.input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
        assert!(app.quit);
        assert!(app.command_checks.is_empty());
        assert!(app.first_send_checks.is_empty());
    }

    #[test]
    fn minimum_geometry_is_explicit() {
        assert!(supported(40, 18));
        assert!(supported(80, 24));
        assert!(!supported(39, 24));
        assert!(!supported(120, 17));
    }
}
