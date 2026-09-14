//! Actions are tied to one saved answer, not whichever reply is newest later.
use super::super::{App, state::Message};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

pub(super) fn eligible(message: &Message) -> bool {
    message.role == "assistant"
        && message.interrupted_attempt.is_none()
        && message.operator_name.is_none()
        && message.tool_calls.is_empty()
        && !message.content.is_empty()
}
impl App {
    pub(super) fn answer_actions_input(&mut self, event: &Event) -> bool {
        let Some(target) = self.selected else {
            return false;
        };
        let Some(view) = self.views.get(&target) else {
            return false;
        };
        let mut state = view.transcript.borrow_mut();
        if state.search.is_some() {
            return false;
        }
        if matches!(event, Event::Resize(..)) {
            state.answer_hits.clear();
            state.answer_focus = None;
            state.answer_selected = None;
            return false;
        }
        let mut action = None;
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((rect, observed, index)) = state
                .answer_hits
                .iter()
                .find(|(r, _, _)| r.contains((mouse.column, mouse.row).into()))
                .copied()
        {
            action = Some((observed, index, mouse.column.saturating_sub(rect.x) >= 8));
        }
        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::ALT)
        {
            if key.code == KeyCode::Char('m') {
                let next = state
                    .answer_focus
                    .map_or(0, |i| (i + 1) % state.answer_hits.len().max(1));
                state.answer_focus = (!state.answer_hits.is_empty()).then_some(next);
                state.answer_selected = state
                    .answer_focus
                    .and_then(|i| state.answer_hits.get(i))
                    .map(|(_, o, index)| (*o, *index));
                self.status=if state.answer_focus.is_some(){"Answer actions focused · Alt+C copy selected answer · Alt+D inspect current workspace changes · Alt+M next visible answer"}else{"No saved answer actions visible; scroll to an answer"}.into();
                return true;
            }
            if matches!(key.code, KeyCode::Char('c' | 'd'))
                && let Some((observed, index)) = state.answer_selected
            {
                action = Some((observed, index, key.code == KeyCode::Char('d')));
            }
        }
        let Some((observed, index, changes)) = action else {
            return false;
        };
        state.answer_focus = None;
        state.answer_selected = None;
        drop(state);
        let valid = self.views.get(&target).is_some_and(|v| {
            v.process.incarnation == observed.incarnation
                && v.snapshot
                    .as_ref()
                    .is_some_and(|s| s.revision == observed.revision)
        });
        if !valid {
            self.status =
                "Answer changed; wait for redraw and select it again. Nothing copied.".into();
            return true;
        }
        let result = if changes {
            self.open_inspection(target)
        } else {
            self.request_answer_copy(observed, index as u64)
        };
        if let Err(error) = result {
            self.status = super::super::safe(&error.to_string());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_client::ui::{
        account_test_support::Fixture,
        accounts::app_tests::app,
        operator::Observation,
        state::{Target, View},
    };
    use crossterm::event::{KeyEvent, MouseEvent};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};
    fn setup(app: &mut App) -> Target {
        let target = Target {
            route: app.clients.first_route().unwrap(),
            session: uuid::Uuid::new_v4(),
        };
        let mut view=View::new(serde_json::from_value(serde_json::json!({"session_id":target.session,"incarnation":uuid::Uuid::new_v4(),"workspace":"/fixture","state":"live","name":"fixture"})).unwrap());
        view.snapshot=Some(serde_json::from_value(serde_json::json!({"session_id":target.session,"revision":17,"model":"fixture","messages":[{"role":"assistant","content":"first answer","message_index":0},{"role":"assistant","content":"newer answer","message_index":1}],"run":null})).unwrap());
        view.draft.insert_str("retained");
        app.views.insert(target, view);
        app.selected = Some(target);
        target
    }
    #[tokio::test]
    async fn selected_saved_answer_not_latest_and_stale_revision_refuses() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let target = setup(&mut app);
        let observed = app.operator_observation(target).unwrap();
        app.views[&target].transcript.borrow_mut().answer_hits =
            vec![(Rect::new(0, 0, 26, 1), observed, 0)];
        assert!(app.answer_actions_input(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 0,
            modifiers: KeyModifiers::NONE
        })));
        assert_eq!(app.inspection.confirmation, Some((observed, 0)));
        assert_eq!(app.views[&target].draft.text, "retained");
        app.inspection.confirmation = None;
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .revision = 18;
        app.answer_actions_input(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(app.inspection.confirmation.is_none());
        assert!(app.status.contains("Answer changed"));
    }
    #[tokio::test]
    async fn keyboard_selection_survives_status_reflow_and_excludes_interrupted_output() {
        let f = Fixture::new();
        let mut app = app(f.0.path());
        let target = setup(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::process_client::ui::render::draw(frame, &app))
            .unwrap();
        app.answer_actions_input(&Event::Key(KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::ALT,
        )));
        terminal
            .draw(|frame| crate::process_client::ui::render::draw(frame, &app))
            .unwrap();
        app.answer_actions_input(&Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::ALT,
        )));
        assert_eq!(app.inspection.confirmation.unwrap().1, 0);
        let mut message = app.views[&target].snapshot.as_ref().unwrap().messages[0].clone();
        message.interrupted_attempt = Some(uuid::Uuid::new_v4());
        assert!(!eligible(&message));
        app.inspection.confirmation = None;
        let o = Observation {
            target,
            incarnation: app.views[&target].process.incarnation,
            revision: 17,
            run_id: None,
        };
        app.views
            .get_mut(&target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .messages[0] = message;
        assert!(app.request_answer_copy(o, 0).is_err());
    }
}
