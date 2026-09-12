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
        // Keep account choices near their actions; enrollment needs more room for its code.
        let width = screen.width.saturating_sub(2).min(92);
        let height = screen.height.saturating_sub(2).min(26);
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(width) / 2,
            screen.y + screen.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let title = match p.mode {
            Mode::Confirm(_) => " Settings for next run ",
            Mode::Enrollment | Mode::Connections | Mode::Alias(_) => " Sign in to an account ",
            Mode::List => " Choose account ",
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 40 || inner.height < 18 {
            frame.render_widget(
                Paragraph::new(
                    "Enlarge to 44 × 22 to review accounts. Esc closes; running work continues.",
                ),
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
            "For the next run · {}\nCurrent: {}{}",
            safe(&self.clients[p.route].label()),
            p.label(&p.original),
            current
                .map(|c| format!(" · Running: {c} (unchanged)"))
                .unwrap_or_default(),
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
                let visible = body.height.saturating_sub(6) as usize;
                let offset = p.selected.saturating_sub(visible.saturating_sub(1));
                for (n, (i, (label, binding))) in options
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(visible)
                    .enumerate()
                {
                    let rect = Rect::new(body.x, body.y + 1 + n as u16, body.width, 1);
                    let default = binding
                        .as_ref()
                        .is_some_and(|b| p.catalogue.default_account.as_ref() == Some(b));
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} {}{}",
                            if p.selected == i { "›" } else { " " },
                            safe(label),
                            if default { " · Default" } else { "" }
                        )),
                        rect,
                    );
                    self.accounts.hits.borrow_mut().push((rect, i));
                }
                let observation = options
                    .get(p.selected)
                    .and_then(|(_, b)| b.as_ref())
                    .and_then(|b| p.usage.get(&b.account_id));
                frame.render_widget(
                    Paragraph::new(usage::usage_text(
                        observation,
                        chrono::Utc::now().timestamp(),
                    ))
                    .wrap(Wrap { trim: false }),
                    Rect::new(body.x, body.bottom().saturating_sub(4), body.width, 4),
                );
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
                let mut resolved = s.clone();
                resolved.resolve(&p.models);
                let fields = [
                    ("Model", safe(&s.model)),
                    (
                        "Reasoning",
                        safe(s.reasoning_effort.as_deref().unwrap_or("")),
                    ),
                    (
                        "Service tier",
                        safe(s.service_tier.as_deref().unwrap_or("")),
                    ),
                ];
                for (index, (label, value)) in fields.iter().enumerate() {
                    let rect = Rect::new(body.x, body.y + index as u16 * 2, body.width, 2);
                    let effective = match index {
                        1 => resolved.label(super::super::inference::Field::Thinking),
                        2 => resolved.label(super::super::inference::Field::Service),
                        _ => {
                            if p.models.is_empty() {
                                "Enter a model ID · host validates access".into()
                            } else {
                                format!(
                                    "Suggestions: {}",
                                    p.models
                                        .iter()
                                        .take(3)
                                        .map(|m| safe(&m.id))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            }
                        }
                    };
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} {label:<13} [ {} ]\n  {}",
                            if p.edit == index { "›" } else { " " },
                            if value.is_empty() {
                                "Use default"
                            } else {
                                value
                            },
                            safe(&effective)
                        )),
                        rect,
                    );
                    self.accounts.hits.borrow_mut().push((rect, index));
                }
                // Identity, not alias text, determines whether sharing context changes.
                let changed = p
                    .original
                    .account
                    .as_ref()
                    .zip(s.account.as_ref())
                    .is_some_and(|(a, b)| {
                        a.account_id != b.account_id
                            || a.connection_id != b.connection_id
                            || a.identity_generation != b.identity_generation
                    });
                let warning = if changed {
                    "Different account: retained conversation/tool context may be sent to its organization. History stays."
                } else {
                    "Current run and conversation history stay unchanged."
                };
                frame.render_widget(
                    Paragraph::new(format!("Account: {}\n{warning}", p.label(s)))
                        .wrap(Wrap { trim: false }),
                    Rect::new(body.x, body.y + 6, body.width, 3),
                );
                for (index, label, x, width) in
                    [(3, "Apply", body.x, 16), (4, "Cancel", body.x + 18, 16)]
                {
                    let rect = Rect::new(x, body.bottom().saturating_sub(1), width, 1);
                    frame.render_widget(
                        Paragraph::new(format!(
                            "{} [ {label} ]",
                            if p.edit == index { "›" } else { " " }
                        )),
                        rect,
                    );
                    self.accounts.hits.borrow_mut().push((rect, index));
                }
            }
            Mode::Enrollment => {
                let private = p.private.as_ref().filter(|v| {
                    active_material(v)
                        && !p.disconnected
                        && p.connection.borrow().loss_generation == p.loss_generation
                        && self.clients.available(p.route)
                });
                if let Some(v) = private {
                    use ratatui::{
                        style::Modifier,
                        text::{Line, Span, Text},
                    };
                    let text = Text::from(vec![
                        Line::from("Sign in with ChatGPT"),
                        Line::from(""),
                        Line::from(vec![
                            Span::raw("Device code: "),
                            Span::styled(
                                v.user_code.as_deref().unwrap_or(""),
                                crate::theme::Role::AwaitingInput
                                    .style()
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]),
                        Line::from(""),
                        Line::from("O opens https://auth.openai.com/codex/device"),
                        Line::from("Enter this code in that browser page."),
                        Line::from(format!(
                            "Code expires in {} seconds. Waiting for approval…",
                            v.status.expires_at.saturating_sub(now())
                        )),
                        Line::from("C cancels · Esc hides this view"),
                        Line::from("Keep this code private; never paste it into chat."),
                    ]);
                    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), body);
                } else {
                    let state = p.private.as_ref().map(|v| v.status.state).or(p.outcome);
                    let action = match state {
                        Some(EnrollmentState::Starting) => "Requesting your device code…",
                        Some(EnrollmentState::Exchanging) => "Completing sign-in…",
                        Some(EnrollmentState::Succeeded) => {
                            "Signed in. R refreshes the account list."
                        }
                        Some(
                            EnrollmentState::Uncertain
                            | EnrollmentState::Cancelled
                            | EnrollmentState::Denied
                            | EnrollmentState::Expired,
                        ) => {
                            "No usable code for this attempt.
N new sign-in · R check status"
                        }
                        _ => {
                            "Reading the original sign-in attempt…
R refreshes its status"
                        }
                    };
                    let text = format!(
                        "Sign in with ChatGPT\n\n{action}\n\n{}\n\nC cancels the attempt · Esc closes this view",
                        safe(&p.notice)
                    );
                    // Use the otherwise unused notice rows so recovery details remain visible.
                    frame.render_widget(
                        Paragraph::new(text).wrap(Wrap { trim: false }),
                        Rect::new(body.x, body.y, body.width, body.height + 4),
                    );
                }
            }
        }
        if matches!(p.mode, Mode::Enrollment) {
            return;
        }
        let notice = if matches!(p.mode, Mode::List) {
            let default = if p.catalogue.default_account.is_none() {
                "Default account required before starting a new voyage.\n"
            } else {
                ""
            };
            format!(
                "{default}{}\n↑↓ Navigate · Enter Continue · F5 Refresh usage{} · Esc Close",
                safe(&p.notice),
                if p.catalogue.can_set_default {
                    " · F6 Set default"
                } else {
                    ""
                }
            )
        } else {
            safe(&p.notice)
        };
        frame.render_widget(
            Paragraph::new(notice)
                .wrap(Wrap { trim: false })
                .style(crate::theme::Role::AwaitingInput.style()),
            Rect::new(inner.x, inner.bottom().saturating_sub(4), inner.width, 4),
        );
    }
}
