//! One direct model chooser. Selection is local until Use model; credentials
//! remain in the private account view, with a return to this chooser afterwards.
use super::*;
use crossterm::event::{KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    Search,
    Account,
    Advanced,
    Thinking,
    Service,
    Reset,
    Keep,
    Apply,
    Retry,
    Cancel,
}
#[derive(Clone)]
pub(super) struct Draft {
    model: String,
    thinking: Option<String>,
    service: Option<String>,
    advanced: bool,
    review: bool,
    keep: bool,
    focus: Control,
}
impl Draft {
    pub fn new(settings: &Settings) -> Self {
        Self {
            model: settings.model.clone(),
            thinking: settings.reasoning_effort.clone(),
            service: settings.service_tier.clone(),
            advanced: false,
            review: false,
            keep: false,
            focus: Control::Search,
        }
    }
    fn controls(&self) -> Vec<Control> {
        let mut v = vec![Control::Search, Control::Account, Control::Advanced];
        if self.advanced {
            v.extend([Control::Thinking, Control::Service]);
        }
        if self.review {
            v.extend([Control::Reset, Control::Keep]);
        }
        v.extend([Control::Retry, Control::Cancel, Control::Apply]);
        v
    }
    pub(super) fn select(&mut self, model: String) {
        if self.model != model {
            self.model = model;
            self.review = false;
            self.keep = false;
        }
    }
}
fn cycle(current: &mut Option<String>, values: Vec<String>) {
    let mut choices = vec![None];
    choices.extend(values.into_iter().filter(|s| s != "inherit").map(Some));
    if !choices.contains(current) {
        choices.push(current.clone());
    }
    let i = choices.iter().position(|s| s == current).unwrap_or(0);
    *current = choices[(i + 1) % choices.len()].clone();
}
impl App {
    pub(in crate::process_client::ui) fn open_model_options(&mut self) -> Result<()> {
        let destination = self
            .active_draft
            .map(Destination::Draft)
            .or(self.selected.map(Destination::Live))
            .context("Choose a conversation or start a draft first")?;
        if self.inference_settings(destination).is_err()
            || matches!(destination, Destination::Draft(_))
                && self
                    .inference_settings(destination)
                    .is_ok_and(|s| s.account.is_none())
        {
            self.inference.return_to_model = Some(destination);
            self.open_accounts(destination, "")?;
            self.mark_account_initialization(destination);
            return Ok(());
        }
        self.cancel_paste_for_private_panel();
        self.inference_command(destination, "/model", true)
    }
    pub(super) fn resume_model_after_account(&mut self) {
        if self.accounts.open() {
            return;
        }
        let Some(destination) = self.inference.return_to_model else {
            return;
        };
        let current = self
            .active_draft
            .map(Destination::Draft)
            .or(self.selected.map(Destination::Live));
        if current != Some(destination) {
            self.inference.return_choice = None;
            self.inference.return_to_model = None;
            return;
        }
        if let Destination::Live(t) = destination
            && self.views.get(&t).is_some_and(|v| v.pending.is_some())
        {
            return;
        }
        self.inference.return_to_model = None;
        let saved = self.inference.return_choice.take();
        if self.inference_settings(destination).is_err() {
            return;
        }
        if let Err(e) = self.open_model_options() {
            self.status = safe(&e.to_string());
        } else if let (Some((original, choice)), Some(p)) = (saved, self.inference.picker.as_mut())
        {
            if original.account == p.original.account
                && original.provider == p.original.provider
                && original.model == p.original.model
                && original.reasoning_effort == p.original.reasoning_effort
                && original.service_tier == p.original.service_tier
            {
                p.chooser = choice;
            } else {
                p.notice="Account settings changed. Review models for this account before using a model.".into();
            }
        }
    }
    pub(in crate::process_client::ui) fn model_options_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        if matches!(event, Event::Resize(..)) {
            self.inference.options_hit.set(None);
            self.inference.chooser_hits.borrow_mut().clear();
            return Ok(false);
        }
        if self.inference_picker_open()
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
        if let Event::Mouse(m) = event
            && m.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((r, d)) = self.inference.options_hit.get()
            && r.contains((m.column, m.row).into())
        {
            if self
                .active_draft
                .map(Destination::Draft)
                .or(self.selected.map(Destination::Live))
                == Some(d)
            {
                self.open_model_options()?;
            }
            self.inference.options_hit.set(None);
            return Ok(true);
        }
        Ok(false)
    }
    fn chooser_activate(&mut self, mut p: Picker, c: Control) -> Result<()> {
        match c {
            Control::Cancel => {
                self.cancel_model_catalog();
                return Ok(());
            }
            Control::Retry => {
                self.inference.picker = Some(p);
                return self.load_inference_models();
            }
            Control::Account => {
                self.cancel_model_catalog();
                self.inference.return_to_model = Some(p.destination);
                self.inference.return_choice = Some((p.original.clone(), p.chooser.clone()));
                if let Err(e) = self.open_accounts(p.destination, "") {
                    self.inference.return_to_model = None;
                    p.notice = safe(&e.to_string());
                    self.inference.picker = Some(p);
                }
                return Ok(());
            }
            Control::Advanced => {
                p.chooser.advanced = !p.chooser.advanced;
                p.chooser.focus = Control::Advanced;
            }
            Control::Thinking | Control::Service => {
                let mut candidate = p.original.clone();
                candidate.model = p.chooser.model.clone();
                candidate.resolve(&p.models);
                if c == Control::Thinking {
                    cycle(&mut p.chooser.thinking, candidate.reasoning_efforts);
                } else {
                    cycle(&mut p.chooser.service, candidate.service_tiers);
                    p.notice="Service priority can increase cost. This is a draft choice until Use model.".into();
                }
                p.chooser.review = false;
                p.chooser.keep = false;
            }
            Control::Reset => {
                p.chooser.thinking = None;
                p.chooser.service = None;
                p.chooser.keep = false;
                p.chooser.review = false;
            }
            Control::Keep => {
                p.chooser.keep = true;
                p.chooser.review = false;
            }
            Control::Apply => {
                if p.chooser.model != p.original.model
                    && (p.chooser.thinking.is_some() || p.chooser.service.is_some())
                    && !p.chooser.keep
                {
                    p.chooser.review = true;
                    p.chooser.focus = Control::Reset;
                    p.notice="This model may not support your overrides. Reset or explicitly keep them, then Use model. Nothing applied.".into();
                } else {
                    let latest = self.inference_settings(p.destination)?;
                    ensure!(
                        latest.model == p.original.model
                            && latest.account == p.original.account
                            && latest.provider == p.original.provider
                            && latest.reasoning_effort == p.original.reasoning_effort
                            && latest.service_tier == p.original.service_tier,
                        "Settings changed elsewhere. Reopen this chooser; nothing applied."
                    );
                    ensure!(
                        !p.chooser.model.is_empty()
                            && p.chooser.model.len() <= 256
                            && !p.chooser.model.chars().any(char::is_whitespace),
                        "Select one model ID"
                    );
                    let mut settings = p.original.clone();
                    settings.model = p.chooser.model.clone();
                    settings.reasoning_effort = p.chooser.thinking.clone();
                    settings.service_tier = p.chooser.service.clone();
                    settings.resolve(&p.models);
                    return self.apply_inference(p, settings);
                }
            }
            Control::Search => p.chooser.focus = Control::Search,
        }
        self.inference.picker = Some(p);
        Ok(())
    }
    pub(super) fn model_chooser_input(&mut self, event: &Event) -> Result<bool> {
        if matches!(event, Event::Resize(..)) {
            self.inference.visible.set(false);
            self.inference.choices.borrow_mut().clear();
            self.inference.chooser_hits.borrow_mut().clear();
            return Ok(false);
        }
        let mut p = self.inference.picker.take().expect("model chooser");
        let current = self
            .active_draft
            .map(Destination::Draft)
            .or(self.selected.map(Destination::Live));
        let owner_current = match p.destination {
            Destination::Live(t) => self
                .views
                .get(&t)
                .is_some_and(|v| Some(v.process.incarnation) == p.incarnation),
            Destination::Draft(_) => true,
        };
        if current != Some(p.destination) || !owner_current {
            self.status = "Conversation changed; reopen model chooser. Draft retained.".into();
            return Ok(true);
        }
        if !self.inference.visible.get()
            && !matches!(event,Event::Key(k) if k.code==KeyCode::Esc || (k.modifiers.contains(KeyModifiers::CONTROL)&&matches!(k.code,KeyCode::Char('c'|'q'))))
        {
            self.inference.picker = Some(p);
            return Ok(true);
        }
        let mut activate = None;
        match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if k.code == KeyCode::Esc {
                    self.cancel_model_catalog();
                    return Ok(true);
                }
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c' | 'q'))
                {
                    self.quit = true;
                    return Ok(true);
                }
                let controls = p.chooser.controls();
                let n = controls
                    .iter()
                    .position(|c| *c == p.chooser.focus)
                    .unwrap_or(0);
                match k.code {
                    KeyCode::Tab => p.chooser.focus = controls[(n + 1) % controls.len()],
                    KeyCode::BackTab => {
                        p.chooser.focus = controls[(n + controls.len() - 1) % controls.len()]
                    }
                    KeyCode::Up | KeyCode::Down => {
                        let opts = p.options();
                        let n = opts.len();
                        p.selected = if k.code == KeyCode::Up {
                            p.selected.saturating_sub(1)
                        } else {
                            (p.selected + 1).min(n.saturating_sub(1))
                        };
                        if let Some(v) = opts.get(p.selected) {
                            p.chooser.select(v.clone());
                        }
                        p.chooser.focus = Control::Search;
                    }
                    KeyCode::Enter => {
                        if p.chooser.focus == Control::Search {
                            if let Some(v) = p.options().get(p.selected) {
                                p.chooser.select(v.clone());
                            }
                            p.chooser.focus = Control::Apply;
                        } else {
                            activate = Some(p.chooser.focus);
                        }
                    }
                    KeyCode::Char(' ') if p.chooser.focus != Control::Search => {
                        activate = Some(p.chooser.focus)
                    }
                    KeyCode::Backspace if p.chooser.focus == Control::Search => {
                        p.query.pop();
                        p.selected = 0;
                    }
                    KeyCode::Char(c)
                        if p.chooser.focus == Control::Search
                            && !k
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            && p.query.len() + c.len_utf8() <= 256 =>
                    {
                        p.query.push(c);
                        p.selected = 0;
                    }
                    _ => {}
                }
            }
            Event::Paste(t) if p.chooser.focus == Control::Search => {
                let t = safe(t);
                if p.query.len() + t.len() <= 256 {
                    p.query.push_str(&t);
                    p.selected = 0;
                }
            }
            Event::Mouse(m) if self.inference.visible.get() => {
                if m.kind == MouseEventKind::Down(MouseButton::Left) {
                    if let Some((_, i)) = self
                        .inference
                        .choices
                        .borrow()
                        .iter()
                        .find(|(r, _)| r.contains((m.column, m.row).into()))
                    {
                        if let Some(v) = p.options().get(*i) {
                            p.selected = *i;
                            p.chooser.select(v.clone());
                            p.chooser.focus = Control::Apply;
                        }
                    } else {
                        activate = self
                            .inference
                            .chooser_hits
                            .borrow()
                            .iter()
                            .find(|(r, _)| r.contains((m.column, m.row).into()))
                            .map(|(_, c)| *c);
                    }
                } else if matches!(
                    m.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                ) && self
                    .inference
                    .picker_area
                    .get()
                    .is_some_and(|r| r.contains((m.column, m.row).into()))
                {
                    p.selected = if m.kind == MouseEventKind::ScrollUp {
                        p.selected.saturating_sub(1)
                    } else {
                        (p.selected + 1).min(p.options().len().saturating_sub(1))
                    };
                }
            }
            _ => {}
        }
        self.inference.choices.borrow_mut().clear();
        self.inference.chooser_hits.borrow_mut().clear();
        if let Some(c) = activate {
            let mut backup = p.clone();
            if let Err(e) = self.chooser_activate(p, c) {
                backup.notice = safe(&e.to_string());
                self.inference.picker = Some(backup);
            }
        } else {
            self.inference.picker = Some(p);
        }
        Ok(true)
    }
    pub(super) fn draw_model_chooser(&self, frame: &mut Frame<'_>) {
        let Some(p) = &self.inference.picker else {
            return;
        };
        let screen = frame.area();
        if screen.width < 40 || screen.height < 18 {
            return;
        }
        let w = screen.width.saturating_sub(2).min(86);
        let h = screen.height.saturating_sub(2).min(24);
        let area = Rect::new(
            screen.x + (screen.width - w) / 2,
            screen.y + (screen.height - h) / 2,
            w,
            h,
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Choose a model ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.inference.visible.set(true);
        self.inference.picker_area.set(Some(area));
        let route = match p.destination {
            Destination::Live(t) => t.route,
            Destination::Draft(id) => self.new_drafts[&id].route,
        };
        let mut y = inner.y;
        let mut control = |rect: Rect, c: Control, label: String| {
            frame.render_widget(
                Paragraph::new(label).style(if p.chooser.focus == c {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Focus.style()
                }),
                rect,
            );
            self.inference.chooser_hits.borrow_mut().push((rect, c));
        };
        control(
            Rect::new(inner.x, y, inner.width, 1),
            Control::Search,
            format!(
                "Search: {}{}",
                safe(&p.query),
                if p.loading { " (loading…)" } else { "" }
            ),
        );
        y += 1;

        let extras =
            7 + if p.chooser.advanced { 2 } else { 0 } + if p.chooser.review { 2 } else { 0 };
        let count = inner.height.saturating_sub(extras).max(1) as usize;
        let opts = p.options();
        let first = p.selected.saturating_sub(count - 1);
        for (i, v) in opts.iter().enumerate().skip(first).take(count) {
            let rect = Rect::new(inner.x, y, inner.width, 1);
            let selected = *v == p.chooser.model;
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {}{}",
                    if selected { "●" } else { " " },
                    safe(v),
                    if *v == p.original.model {
                        " (current)"
                    } else {
                        ""
                    }
                ))
                .style(if i == p.selected {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Primary.style()
                }),
                rect,
            );
            self.inference.choices.borrow_mut().push((rect, i));
            y += 1;
        }
        y = inner.y + 1 + count as u16;
        let mut control = |rect: Rect, c: Control, label: String| {
            frame.render_widget(
                Paragraph::new(label).style(if p.chooser.focus == c {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Focus.style()
                }),
                rect,
            );
            self.inference.chooser_hits.borrow_mut().push((rect, c));
        };
        control(
            Rect::new(inner.x, y, inner.width, 1),
            Control::Account,
            format!(
                "Account: {} ▾",
                self.account_control_label(p.destination, &p.original)
            ),
        );
        y += 1;
        control(
            Rect::new(inner.x, y, inner.width, 1),
            Control::Advanced,
            format!(
                "Advanced options {}",
                if p.chooser.advanced { "▾" } else { "▸" }
            ),
        );
        y += 1;
        if p.chooser.advanced {
            control(
                Rect::new(inner.x, y, inner.width, 1),
                Control::Thinking,
                format!(
                    "Thinking: {} ↻",
                    p.chooser.thinking.as_deref().unwrap_or("Inherit")
                ),
            );
            y += 1;
            control(
                Rect::new(inner.x, y, inner.width, 1),
                Control::Service,
                format!(
                    "Service: {} ↻",
                    p.chooser.service.as_deref().unwrap_or("Inherit")
                ),
            );
            y += 1;
        }
        if p.chooser.review {
            control(
                Rect::new(inner.x, y, inner.width, 1),
                Control::Reset,
                "Reset overrides (recommended)".into(),
            );
            y += 1;
            control(
                Rect::new(inner.x, y, inner.width, 1),
                Control::Keep,
                "Keep overrides for validation".into(),
            );
            y += 1;
        }

        frame.render_widget(
            Paragraph::new(format!("{} · Next message only", self.route_label(route))),
            Rect::new(inner.x, y, inner.width, 1),
        );
        if p.chooser.focus == Control::Search {
            let col = unicode_width::UnicodeWidthStr::width(p.query.as_str())
                .saturating_add(8)
                .min(inner.width.saturating_sub(1) as usize) as u16;
            frame.set_cursor_position((inner.x + col, inner.y));
        }
        frame.render_widget(
            Paragraph::new(safe(&p.notice)).wrap(Wrap { trim: false }),
            Rect::new(
                inner.x,
                y + 1,
                inner.width,
                inner.bottom().saturating_sub(y + 2),
            ),
        );
        let cancel = Rect::new(inner.x, inner.bottom() - 1, 10, 1);
        let retry = Rect::new(inner.x + 11, inner.bottom() - 1, 9, 1);
        let apply = Rect::new(inner.right() - 13, inner.bottom() - 1, 13, 1);
        for (r, c, label) in [
            (cancel, Control::Cancel, "[Cancel]"),
            (retry, Control::Retry, "[Retry]"),
            (apply, Control::Apply, "[Use model]"),
        ] {
            frame.render_widget(
                Paragraph::new(label).style(if p.chooser.focus == c {
                    crate::theme::Role::Selection.style()
                } else {
                    crate::theme::Role::Focus.style()
                }),
                r,
            );
            self.inference.chooser_hits.borrow_mut().push((r, c));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent};
    use ratatui::{Terminal, backend::TestBackend};
    fn app_fixture(app: &mut App) -> Target {
        let target = Target {
            route: app.clients.first_route().unwrap(),
            session: Uuid::new_v4(),
        };
        let mut v=super::super::super::state::View::new(serde_json::from_value(serde_json::json!({"session_id":target.session,"incarnation":Uuid::new_v4(),"workspace":"/fixture","state":"live","name":"fixture"})).unwrap());
        v.snapshot=Some(serde_json::from_value(serde_json::json!({"session_id":target.session,"revision":1,"model":"current","inference":{"model":"current","provider":"fixture","reasoning_efforts":["low","high"],"service_tiers":["standard","priority"]},"messages":[],"run":null})).unwrap());
        v.draft.insert_str("keep my draft");
        app.views.insert(target, v);
        app.selected = Some(target);
        target
    }
    fn picker(app: &mut App, t: Target) {
        let original = app.inference_settings(Destination::Live(t)).unwrap();
        app.inference.picker = Some(Picker {
            id: Uuid::new_v4(),
            chooser: Draft::new(&original),
            incarnation: Some(app.views[&t].process.incarnation),
            destination: Destination::Live(t),
            original,
            models: vec![],
            field: Field::Model,
            query: String::new(),
            selected: 0,
            options: vec!["current".into(), "other".into()],
            loading: false,
            notice: String::new(),
            confirmation: None,
            command_text: String::new(),
            preserve_draft: true,
        });
    }
    fn key(c: KeyCode) -> Event {
        Event::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }
    fn click(r: Rect) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x,
            row: r.y,
            modifiers: KeyModifiers::NONE,
        })
    }
    fn paint(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| super::super::super::render::draw(f, app))
            .unwrap();
        t
    }
    fn text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    #[tokio::test]
    async fn missing_reply_times_out_and_late_reply_cannot_repopulate_picker() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        let id = app.inference.picker.as_ref().unwrap().id;
        app.inference.picker.as_mut().unwrap().loading = true;
        let job = tokio::spawn(async { std::future::pending::<()>().await });
        app.inference.catalog_job = Some((
            id,
            std::time::Instant::now() - std::time::Duration::from_secs(1),
            job,
        ));
        app.poll_model_catalog();
        let p = app.inference.picker.as_ref().unwrap();
        assert!(!p.loading);
        assert!(p.notice.contains("timed out"));
        assert_ne!(p.id, id);
        app.inference_models(
            id,
            None,
            None,
            Ok(serde_json::json!([{"id":"late","display_name":"late","description":""}])),
        );
        assert!(
            !app.inference
                .picker
                .as_ref()
                .unwrap()
                .options
                .contains(&"late".into())
        );
        assert_eq!(app.views[&t].draft.text, "keep my draft");
        for job in app.retired_observers {
            let _ = job.await;
        }
    }
    #[tokio::test]
    async fn retry_replaces_owned_read_and_failure_settles_loading() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        app.load_inference_models().unwrap();
        let first = app.inference.picker.as_ref().unwrap().id;
        app.load_inference_models().unwrap();
        let second = app.inference.picker.as_ref().unwrap().id;
        assert_ne!(first, second);
        app.inference_models(first, None, None, Err("old".into()));
        assert!(app.inference.picker.as_ref().unwrap().loading);
        app.inference_models(second, None, None, Err("unavailable".into()));
        assert!(!app.inference.picker.as_ref().unwrap().loading);
        assert!(app.inference.catalog_job.is_none());
        assert!(
            app.inference
                .picker
                .as_ref()
                .unwrap()
                .notice
                .contains("Retry")
        );
        assert!(app.views[&t].pending.is_none());
        for job in app.retired_observers {
            let _ = job.await;
        }
    }
    #[tokio::test]
    async fn direct_list_centered_mouse_select_is_draft_only_cancel_retains_original() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        for (w, h) in [(40, 18), (120, 40)] {
            picker(&mut app, t);
            let screen = paint(&app, w, h);
            let area = app.inference.picker_area.get().unwrap();
            assert!(area.x.abs_diff(w - area.right()) <= 1);
            assert!(area.y.abs_diff(h - area.bottom()) <= 1);
            let rendered = text(&screen);
            assert!(rendered.contains("Choose a model"));
            assert!(!rendered.contains("Model options"));
            assert!(rendered.contains("Use model"));
            assert!(!rendered.contains("Thinking:"));
            let row = app.inference.choices.borrow()[1].0;
            app.input(click(row)).unwrap();
            assert_eq!(
                app.inference.picker.as_ref().unwrap().chooser.model,
                "other"
            );
            assert!(app.views[&t].pending.is_none());
            paint(&app, w, h);
            let cancel = app
                .inference
                .chooser_hits
                .borrow()
                .iter()
                .find(|(_, c)| *c == Control::Cancel)
                .unwrap()
                .0;
            app.input(click(cancel)).unwrap();
            assert!(app.inference.picker.is_none());
            assert_eq!(app.views[&t].draft.text, "keep my draft");
            assert_eq!(
                app.inference_settings(Destination::Live(t)).unwrap().model,
                "current"
            );
        }
    }
    #[tokio::test]
    async fn advanced_and_override_review_are_inline_and_unapplied_until_use() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        let p = app.inference.picker.take().unwrap();
        app.chooser_activate(p, Control::Advanced).unwrap();
        let p = app.inference.picker.take().unwrap();
        app.chooser_activate(p, Control::Thinking).unwrap();
        let mut p = app.inference.picker.take().unwrap();
        assert_eq!(p.chooser.thinking.as_deref(), Some("low"));
        p.chooser.select("other".into());
        app.chooser_activate(p, Control::Apply).unwrap();
        assert!(app.inference.picker.as_ref().unwrap().chooser.review);
        assert!(app.views[&t].pending.is_none());
        assert!(text(&paint(&app, 40, 18)).contains("Reset overrides"));
        let p = app.inference.picker.take().unwrap();
        app.chooser_activate(p, Control::Reset).unwrap();
        assert!(
            app.inference
                .picker
                .as_ref()
                .unwrap()
                .chooser
                .thinking
                .is_none()
        );
        assert!(app.views[&t].pending.is_none());
    }
    #[tokio::test]
    async fn account_returns_to_direct_chooser_without_prompt_or_hidden_apply() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        let mut p = app.inference.picker.take().unwrap();
        p.chooser.select("other".into());
        app.chooser_activate(p, Control::Account).unwrap();
        assert!(app.accounts.open());
        assert!(app.inference.picker.is_none());
        app.input(key(KeyCode::Esc)).unwrap();
        app.resume_model_after_account();
        assert_eq!(
            app.inference.picker.as_ref().unwrap().chooser.model,
            "other"
        );
        assert!(app.views[&t].pending.is_none());
        assert_eq!(app.views[&t].draft.text, "keep my draft");
        for (_, jobs) in app.route_tasks {
            for job in jobs {
                job.abort();
                let _ = job.await;
            }
        }
    }
    #[tokio::test]
    async fn use_model_is_only_admission_and_stale_settings_keep_local_choice() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        let mut p = app.inference.picker.take().unwrap();
        p.chooser.select("other".into());
        app.chooser_activate(p, Control::Apply).unwrap();
        let pending = app.views[&t]
            .pending
            .as_ref()
            .expect("explicit Use admitted once");
        assert!(
            matches!(pending.original.as_deref(),Some(voyage_protocol::vessel::VoyageCommand::SetInference{model,..}) if model=="other")
        );
        assert!(pending.preserve_draft);
        assert_eq!(app.views[&t].draft.text, "keep my draft");
        for (_, jobs) in app.route_tasks {
            for job in jobs {
                job.abort();
                let _ = job.await;
            }
        }
    }
    #[tokio::test]
    async fn typing_filters_without_changing_selection_or_admitting_and_stale_mouse_refuses() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        paint(&app, 80, 24);
        let old = app.inference.choices.borrow()[1].0;
        app.input(key(KeyCode::Char('o'))).unwrap();
        assert!(app.inference.choices.borrow().is_empty());
        app.input(click(old)).unwrap();
        assert_eq!(
            app.inference.picker.as_ref().unwrap().chooser.model,
            "current"
        );
        assert!(app.views[&t].pending.is_none());
        paint(&app, 80, 24);
        app.input(key(KeyCode::Down)).unwrap();
        app.input(key(KeyCode::Enter)).unwrap();
        assert!(app.views[&t].pending.is_none());
        app.views
            .get_mut(&t)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .inference
            .as_mut()
            .unwrap()
            .model = "external-change".into();
        paint(&app, 80, 24);
        app.input(key(KeyCode::Enter)).unwrap();
        assert!(app.views[&t].pending.is_none());
        assert!(
            app.inference
                .picker
                .as_ref()
                .unwrap()
                .notice
                .contains("Settings changed")
        );
    }
    #[tokio::test]
    async fn keyboard_requires_explicit_apply_and_changed_owner_refuses() {
        let f = super::super::super::account_test_support::Fixture::new();
        let mut app = super::super::super::accounts::app_tests::app(f.0.path());
        let t = app_fixture(&mut app);
        picker(&mut app, t);
        paint(&app, 80, 24);
        app.input(key(KeyCode::Down)).unwrap();
        app.input(key(KeyCode::Enter)).unwrap();
        assert!(app.views[&t].pending.is_none());
        app.views.get_mut(&t).unwrap().process.incarnation = Uuid::new_v4();
        app.input(key(KeyCode::Enter)).unwrap();
        assert!(app.inference.picker.is_none());
        assert!(app.views[&t].pending.is_none());
    }
}
