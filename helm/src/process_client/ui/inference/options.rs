//! One composer entry point for account and next-turn inference preferences.
//! No credential input lives here; Account hands off to the existing private view.
use super::*;
use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

pub(super) struct OptionsPanel {
    destination: Destination,
    incarnation: Option<Uuid>,
    selected: usize,
    notice: String,
}
const FIELDS: &[Field] = &[
    Field::Model,
    Field::Account,
    Field::Thinking,
    Field::Service,
];

impl App {
    pub(in crate::process_client::ui) fn open_model_options(&mut self) -> Result<()> {
        let destination = self
            .active_draft
            .map(Destination::Draft)
            .or(self.selected.map(Destination::Live))
            .context("Choose a conversation or start a draft first")?;
        if self.inference_settings(destination).is_err() {
            return self.open_accounts(destination, "");
        }
        self.inference.options = Some(OptionsPanel {
            destination,
            incarnation: match destination {
                Destination::Live(t) => self.views.get(&t).map(|v| v.process.incarnation),
                _ => None,
            },
            selected: 0,
            notice: String::new(),
        });
        self.inference.options_rows.borrow_mut().clear();
        self.cancel_paste_for_private_panel();
        Ok(())
    }
    fn model_options_current(&self, panel: &OptionsPanel) -> bool {
        match panel.destination {
            Destination::Draft(id) => {
                self.active_draft == Some(id) && self.new_drafts.contains_key(&id)
            }
            Destination::Live(t) => {
                self.active_draft.is_none()
                    && self.selected == Some(t)
                    && self
                        .views
                        .get(&t)
                        .is_some_and(|v| Some(v.process.incarnation) == panel.incarnation)
            }
        }
    }
    pub(in crate::process_client::ui) fn model_options_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        if matches!(event, Event::Resize(..)) {
            self.inference.options_back.set(None);
            self.inference.options_area.set(None);
            self.inference.options_hit.set(None);
            self.inference.options_rows.borrow_mut().clear();
            return Ok(false);
        }
        if self.inference.options.is_none() {
            if self.inference.picker.is_some()
                || self.inference.access.draft.is_some()
                || self.accounts.open()
                || self.vessels_open()
                || self.help
                || self.explore.is_some()
                || self.sidebar.menu.is_some()
                || self.interactions.borrow().focused
                || self.inspection.panel.is_some()
                || self.inspection.confirmation.is_some()
                || self.operator.is_some()
                || self.voyage_picker.is_some()
                || self.workspace_menu_open()
            {
                return Ok(false);
            }
            if let Event::Mouse(mouse) = event
                && mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some((rect, destination)) = self.inference.options_hit.get()
                && rect.contains((mouse.column, mouse.row).into())
            {
                let selected = self
                    .active_draft
                    .map(Destination::Draft)
                    .or(self.selected.map(Destination::Live));
                if selected == Some(destination) {
                    self.open_model_options()?;
                }
                self.inference.options_hit.set(None);
                return Ok(true);
            }
            return Ok(false);
        }
        if self.accounts.open()
            || self.vessels_open()
            || self.interactions.borrow().focused
            || self.help
            || self.stop_review.is_some()
        {
            self.inference.options = None;
            self.inference.options_rows.borrow_mut().clear();
            return Ok(false);
        }
        let mut panel = self.inference.options.take().unwrap();
        if !self.model_options_current(&panel) {
            self.inference.options_rows.borrow_mut().clear();
            self.status =
                "Conversation changed. Reopen Model options; your draft is retained.".into();
            return Ok(true);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .inference
                .options_back
                .get()
                .is_some_and(|r| r.contains((mouse.column, mouse.row).into()))
        {
            self.inference.options_back.set(None);
            self.inference.options_rows.borrow_mut().clear();
            return Ok(true);
        }
        let mut activate = None;
        match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c' | 'q'))
                {
                    self.quit = true;
                    return Ok(true);
                }
                match k.code {
                    KeyCode::Esc => {
                        self.inference.options_rows.borrow_mut().clear();
                        return Ok(true);
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        panel.selected = (panel.selected + FIELDS.len() - 1) % FIELDS.len()
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        panel.selected = (panel.selected + 1) % FIELDS.len()
                    }
                    KeyCode::Enter | KeyCode::Char(' ') if k.modifiers.is_empty() => {
                        activate = Some(FIELDS[panel.selected])
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) && self
                    .inference
                    .options_area
                    .get()
                    .is_some_and(|r| r.contains((mouse.column, mouse.row).into())) =>
            {
                panel.selected = if mouse.kind == MouseEventKind::ScrollUp {
                    panel.selected.saturating_sub(1)
                } else {
                    (panel.selected + 1).min(FIELDS.len() - 1)
                };
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                activate = self
                    .inference
                    .options_rows
                    .borrow()
                    .iter()
                    .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                    .map(|(_, field)| *field);
            }
            _ => {} // Paste and non-activation keys stay in this scope.
        }
        self.inference.options_rows.borrow_mut().clear();
        if let Some(field) = activate {
            let result = self.inference_command(
                panel.destination,
                &format!("/{}", field.name().to_lowercase()),
                true,
            );
            if let Err(error) = result {
                panel.notice = safe(&error.to_string());
                self.inference.options = Some(panel);
            }
        } else {
            self.inference.options = Some(panel);
        }
        Ok(true)
    }
    pub(super) fn draw_model_options(&self, frame: &mut Frame<'_>) {
        self.inference.options_rows.borrow_mut().clear();
        let Some(panel) = &self.inference.options else {
            return;
        };
        if self.accounts.open()
            || self.vessels_open()
            || self.interactions.borrow().focused
            || self.help
            || !self.model_options_current(panel)
        {
            return;
        }
        let Ok(settings) = self.inference_settings(panel.destination) else {
            return;
        };
        let screen = frame.area();
        if screen.width < 40 || screen.height < 18 {
            return;
        }
        let width = screen.width.saturating_sub(2).min(82);
        let height = 16;
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(width) / 2,
            screen.y + screen.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Model options ")
            .title_bottom(" ↑↓ Choose · Enter Open · Esc Back ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.inference.options_area.set(Some(area));
        let back = Rect::new(
            area.right().saturating_sub(10),
            area.y,
            8.min(area.width),
            1,
        );
        frame.render_widget(
            Paragraph::new("[Back]").style(crate::theme::Role::Focus.style()),
            back,
        );
        self.inference.options_back.set(Some(back));
        let route = match panel.destination {
            Destination::Draft(id) => self.new_drafts[&id].route,
            Destination::Live(t) => t.route,
        };
        let header = format!(
            "{}\nNext turn only · current work unchanged",
            self.route_label(route)
        );
        frame.render_widget(
            Paragraph::new(super::super::presentation::wrap(
                ratatui::text::Text::raw(header),
                inner.width,
            )),
            Rect::new(inner.x, inner.y, inner.width, 3),
        );
        let rows: Vec<_> = FIELDS
            .iter()
            .map(|field| {
                let label = if *field == Field::Account {
                    self.account_control_label(panel.destination, &settings)
                } else {
                    settings.label(*field)
                };
                ListItem::new(format!("{}: {}", field.name(), safe(&label)))
            })
            .collect();
        let list = Rect::new(inner.x, inner.y + 3, inner.width, 4);
        frame.render_stateful_widget(
            List::new(rows).highlight_style(crate::theme::Role::Selection.style()),
            list,
            &mut ListState::default().with_selected(Some(panel.selected)),
        );
        for (n, field) in FIELDS.iter().enumerate() {
            self.inference
                .options_rows
                .borrow_mut()
                .push((Rect::new(list.x, list.y + n as u16, list.width, 1), *field));
        }
        let info = if panel.notice.is_empty() {
            "Account opens private sign-in and billing choices. Thinking and Service are optional. Each change uses the existing confirmation and compatibility checks."
        } else {
            &panel.notice
        };
        frame.render_widget(
            Paragraph::new(super::super::presentation::wrap(
                ratatui::text::Text::raw(info.to_owned()),
                inner.width,
            )),
            Rect::new(
                inner.x,
                inner.y + 8,
                inner.width,
                inner.height.saturating_sub(8),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    fn setup(app: &mut App) -> Target {
        let route = app.clients.routes().next().unwrap();
        let session = Uuid::new_v4();
        let target = Target { route, session };
        let mut view =
            super::super::super::state::View::new(voyage_protocol::vessel::ProcessInfo {
                catalogue: None,
                archive: None,
                deletion: None,
                session_id: session,
                incarnation: Uuid::new_v4(),
                workspace: "/synthetic".into(),
                state: voyage_protocol::process::ProcessState::Live,
                name: Some("Focused task".into()),
            });
        view.snapshot=Some(serde_json::from_value(serde_json::json!({"session_id":session,"revision":1,"model":"fixture-model","inference":{"model":"fixture-model","provider":"fixture"},"messages":[],"run":null})).unwrap());
        view.draft.insert_str("retained input");
        app.views.insert(target, view);
        app.selected = Some(target);
        target
    }
    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    fn mouse(kind: MouseEventKind, r: Rect) -> Event {
        Event::Mouse(crossterm::event::MouseEvent {
            kind,
            column: r.x,
            row: r.y,
            modifiers: KeyModifiers::NONE,
        })
    }
    #[tokio::test]
    async fn mouse_wheel_and_back_stay_inside_options_without_changing_settings() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let target = setup(&mut app);
        app.open_model_options().unwrap();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| super::super::super::render::draw(f, &app))
            .unwrap();
        let area = app.inference.options_area.get().unwrap();
        app.input(mouse(
            MouseEventKind::ScrollDown,
            Rect::new(area.x + 2, area.y + 2, 1, 1),
        ))
        .unwrap();
        assert_eq!(app.inference.options.as_ref().unwrap().selected, 1);
        assert!(app.inference.options_rows.borrow().is_empty());
        t.draw(|f| super::super::super::render::draw(f, &app))
            .unwrap();
        app.input(mouse(
            MouseEventKind::Down(MouseButton::Left),
            app.inference.options_back.get().unwrap(),
        ))
        .unwrap();
        assert!(app.inference.options.is_none());
        assert!(!app.accounts.open());
        assert_eq!(app.views[&target].draft.text, "retained input");
    }
    #[tokio::test]
    async fn model_wheel_requires_redraw_and_cancel_never_applies_override() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let target = setup(&mut app);
        let original = app.inference_settings(Destination::Live(target)).unwrap();
        app.inference.picker = Some(Picker {
            id: Uuid::new_v4(),
            destination: Destination::Live(target),
            original,
            models: vec![],
            field: Field::Model,
            query: String::new(),
            selected: 0,
            options: (0..30).map(|n| format!("model-{n}")).collect(),
            loading: false,
            notice: String::new(),
            confirmation: None,
            command_text: String::new(),
            preserve_draft: true,
        });
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| super::super::super::render::draw(f, &app))
            .unwrap();
        let (row, _) = app.inference.choices.borrow()[0];
        app.input(mouse(MouseEventKind::ScrollDown, row)).unwrap();
        assert_eq!(app.inference.picker.as_ref().unwrap().selected, 1);
        assert!(app.inference.choices.borrow().is_empty());
        // Old pointer map cannot apply the previous row before the next paint.
        app.input(mouse(MouseEventKind::Down(MouseButton::Left), row))
            .unwrap();
        assert!(app.views[&target].pending.is_none());
        assert!(app.inference.picker.is_some());
        t.draw(|f| super::super::super::render::draw(f, &app))
            .unwrap();
        app.input(mouse(
            MouseEventKind::Down(MouseButton::Left),
            app.inference.cancel_hit.get().unwrap(),
        ))
        .unwrap();
        assert!(app.inference.picker.is_none());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "retained input");
    }
    #[tokio::test]
    async fn override_review_can_be_cancelled_by_mouse_without_changing_model() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let target = setup(&mut app);
        let original = app.inference_settings(Destination::Live(target)).unwrap();
        let mut next = original.clone();
        next.model = "other-model".into();
        app.inference.picker = Some(Picker {
            id: Uuid::new_v4(),
            destination: Destination::Live(target),
            original,
            models: vec![],
            field: Field::Model,
            query: String::new(),
            selected: 0,
            options: vec![],
            loading: false,
            notice: "Review overrides".into(),
            confirmation: Some(next),
            command_text: String::new(),
            preserve_draft: true,
        });
        let mut t = Terminal::new(TestBackend::new(40, 18)).unwrap();
        t.draw(|f| super::super::super::render::draw(f, &app))
            .unwrap();
        let hit = app.inference.cancel_hit.get().unwrap();
        app.input(mouse(MouseEventKind::Down(MouseButton::Left), hit))
            .unwrap();
        assert!(app.inference.picker.is_none());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(
            app.inference_settings(Destination::Live(target))
                .unwrap()
                .model,
            "fixture-model"
        );
        assert_eq!(app.views[&target].draft.text, "retained input");
    }

    #[tokio::test]
    async fn one_composer_control_and_named_options_preserve_draft_and_effective_model() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let target = setup(&mut app);
        for (w, h) in [(40, 18), (120, 40)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|frame| super::super::super::render::draw(frame, &app))
                .unwrap();
            let screen = text(&t);
            assert!(screen.contains("Model: fixture-model"), "{screen}");
            assert!(!screen.contains("Thinking:"));
            assert!(!screen.contains("Service:"));
            app.open_model_options().unwrap();
            t.draw(|frame| super::super::super::render::draw(frame, &app))
                .unwrap();
            let area = app.inference.options_area.get().unwrap();
            assert!(area.x.abs_diff(w.saturating_sub(area.right())) <= 1);
            assert!(area.y.abs_diff(h.saturating_sub(area.bottom())) <= 1);
            let screen = text(&t);
            for label in ["Model options", "Account:", "Thinking:", "Service:"] {
                assert!(screen.contains(label), "{screen}");
            }
            app.input(Event::Paste("/new forbidden".into())).unwrap();
            app.input(key(KeyCode::Esc)).unwrap();
            assert_eq!(app.views[&target].draft.text, "retained input");
            assert_eq!(
                app.views[&target].snapshot.as_ref().unwrap().model,
                "fixture-model"
            );
        }
    }
    #[tokio::test]
    async fn options_use_existing_private_account_path_and_reject_changed_owner() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let target = setup(&mut app);
        app.open_model_options().unwrap();
        app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4();
        app.model_options_input(&key(KeyCode::Enter)).unwrap();
        assert!(app.inference.options.is_none());
        assert!(app.inference.picker.is_none());
        app.open_model_options().unwrap();
        app.model_options_input(&key(KeyCode::Down)).unwrap();
        app.model_options_input(&key(KeyCode::Enter)).unwrap();
        assert!(app.accounts.open());
        assert!(app.inference.options.is_none());
        assert_eq!(app.views[&target].draft.text, "retained input");
    }
    #[tokio::test]
    async fn options_dont_override_decision_or_private_focus() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        setup(&mut app);
        app.open_model_options().unwrap();
        app.interactions.borrow_mut().focused = true;
        assert!(!app.model_options_input(&key(KeyCode::Enter)).unwrap());
        assert!(app.inference.options.is_none());
        assert!(app.inference.picker.is_none());
    }
}
