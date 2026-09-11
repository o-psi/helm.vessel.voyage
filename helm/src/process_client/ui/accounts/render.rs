use super::*;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
impl App {
    pub(in crate::process_client::ui) fn draw_accounts(&self, frame: &mut Frame<'_>) {
        self.accounts.hits.borrow_mut().clear();
        self.accounts.visible.set(false);
        let Some(p) = &self.accounts.picker else {
            return;
        };
        let screen = frame.area();
        let area = Rect::new(
            screen.x + 1,
            screen.y + 1,
            screen.width.saturating_sub(2),
            screen.height.saturating_sub(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Account · private human view · Esc close (not cancel) ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 40 || inner.height < 18 {
            frame.render_widget(
                Paragraph::new("Enlarge to at least 44 columns × 22 rows to review account controls. Esc closes without cancelling."),
                inner,
            );
            return;
        }
        self.accounts.visible.set(true);
        let current = match p.destination {
            Destination::Live(t) => self
                .views
                .get(&t)
                .and_then(|v| v.snapshot.as_ref())
                .and_then(|s| s.inference_current.as_ref())
                .map(|s| p.label(s)),
            _ => None,
        };
        let heading = format!(
            "Host: {} [{}]\nNext run: {} · {}\n{}",
            safe(&self.clients[p.route].label()),
            p.host
                .map(|h| h.to_string())
                .unwrap_or_else(|| "authenticating".into()),
            p.label(&p.original),
            safe(&p.original.provider),
            current
                .map(|c| format!("Running with: {c} (unchanged)"))
                .unwrap_or_else(|| "Account selection never clears conversation history".into())
        );
        frame.render_widget(
            Paragraph::new(heading),
            Rect::new(inner.x, inner.y, inner.width, 3),
        );
        let body = Rect::new(
            inner.x,
            inner.y + 3,
            inner.width,
            inner.height.saturating_sub(7),
        );
        match &p.mode {
            Mode::List => {
                frame.render_widget(
                    Paragraph::new(format!(
                        "Search: {}{}",
                        safe(&p.query),
                        if p.busy { " · loading" } else { "" }
                    )),
                    Rect::new(body.x, body.y, body.width, 1),
                );
                let options = p.choices();
                let visible = body.height.saturating_sub(1) as usize;
                let offset = p.selected.saturating_sub(visible.saturating_sub(1));
                for (row, (i, (label, _))) in
                    (body.y + 1..body.bottom()).zip(options.iter().enumerate().skip(offset))
                {
                    let rect = Rect::new(body.x, row, body.width, 1);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} {}",
                            if p.selected == i { ">" } else { " " },
                            label
                        )),
                        rect,
                    );
                    self.accounts.hits.borrow_mut().push((rect, i));
                }
            }
            Mode::Connections => {
                let connections: Vec<_> = p
                    .catalogue
                    .connections
                    .iter()
                    .filter(|c| {
                        c.transports.contains(&Transport::ChatgptOauth)
                            && c.endpoint == "https://chatgpt.com/backend-api/codex"
                    })
                    .collect();
                let offset = p
                    .selected
                    .saturating_sub(body.height.saturating_sub(1) as usize);
                for (row, (index, c)) in
                    (body.y..body.bottom()).zip(connections.iter().enumerate().skip(offset))
                {
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} {} · ChatGPT subscription",
                            if p.selected == index { ">" } else { " " },
                            safe(&c.label)
                        )),
                        Rect::new(body.x, row, body.width, 1),
                    );
                    self.accounts
                        .hits
                        .borrow_mut()
                        .push((Rect::new(body.x, row, body.width, 1), index));
                }
            }
            Mode::Alias(_) => {
                frame.render_widget(Paragraph::new(format!("Native ChatGPT device sign-in\nNew account alias: {}\nEnter starts explicitly; no browser opens automatically",safe(&p.query))).wrap(Wrap {trim:false}),body);
            }
            Mode::Confirm(s) => {
                let catalog = p
                    .models
                    .iter()
                    .take(5)
                    .map(|m| safe(&m.id))
                    .collect::<Vec<_>>()
                    .join(", ");
                let text = format!(
                    "Switch {} → {}\nRetained conversation/tools may go to the new account and organization. History stays.\n{} Model: {}\n{} Thinking: {}\n{} Service: {}\nRemember for NEW voyages: {} (F4)\nCatalog suggestions (not entitlement): {}\nTab edits field · Ctrl+U clear · F2 clear overrides · Enter confirms",
                    p.label(&p.original),
                    p.label(s),
                    if p.edit == 0 { ">" } else { " " },
                    safe(&s.model),
                    if p.edit == 1 { ">" } else { " " },
                    safe(s.reasoning_effort.as_deref().unwrap_or("inherit")),
                    if p.edit == 2 { ">" } else { " " },
                    safe(s.service_tier.as_deref().unwrap_or("inherit")),
                    p.remember,
                    if catalog.is_empty() {
                        "unknown"
                    } else {
                        &catalog
                    }
                );
                frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), body);
            }
            Mode::Enrollment => {
                let private = p.private.as_ref().filter(|v| {
                    active_material(v)
                        && !p.disconnected
                        && p.connection.borrow().loss_generation == p.loss_generation
                        && self.clients.available(p.route)
                });
                let text = if let Some(v) = private {
                    format!(
                        "PRIVATE SIGN-IN — never copy into chat\nProvider: https://auth.openai.com/codex/device\nCode: {}\nExpires in {} seconds\nO explicitly opens local browser · C cancels sign-in\nR refreshes original operation · Esc hides sensitive material",
                        v.user_code.as_deref().unwrap_or(""),
                        v.status.expires_at.saturating_sub(now())
                    )
                } else {
                    "No current private code displayed.\nR inspect original enrollment · C cancel sign-in\nEsc closes only; host operation may continue.\nOn completion reopen /account to refresh and explicitly select the new profile.".into()
                };
                frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), body);
            }
        }
        frame.render_widget(
            Paragraph::new(safe(&p.notice))
                .wrap(Wrap { trim: false })
                .style(crate::theme::Role::AwaitingInput.style()),
            Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 4),
        );
    }
}
