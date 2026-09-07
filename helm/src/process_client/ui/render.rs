use super::{App, presentation};
use crate::{
    markdown::{self, RenderOptions},
    process_client::safe,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};

fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}
fn accent() -> Style {
    Style::default().fg(Color::Cyan)
}
fn inset(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    area.inner(ratatui::layout::Margin::new(horizontal, vertical))
}
fn state(view: &super::state::View) -> &'static str {
    if view.error.is_some() {
        "Reconnecting"
    } else if view
        .snapshot
        .as_ref()
        .is_some_and(|s| !s.decisions.is_empty())
    {
        "Needs you"
    } else {
        view.snapshot
            .as_ref()
            .and_then(|s| s.run.as_ref())
            .map_or("Ready", |r| presentation::run_state(&r.state))
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let view = app.selected.and_then(|key| app.views.get(&key));
    if area.width < 40 || area.height < 18 {
        super::interactions::draw(frame, app, Rect::default());
        if let Some(view) = view {
            view.terminals.clear_displayed();
        }
        frame.render_widget(Paragraph::new(presentation::wrap(Text::raw("Helm\nEnlarge to at least 40 columns x 18 rows.\nCtrl+C leaves; your work continues."), area.width)), area);
        return;
    }
    if app.help || app.explore.is_some() {
        super::interactions::draw(frame, app, Rect::default());
        if let Some(view) = view {
            view.terminals.clear_displayed();
        }
        if app.explore.is_some() {
            super::explore::draw(frame, app, inset(area, 2, 1));
        } else {
            let area = inset(area, 2, 1);
            let text = presentation::wrap(Text::raw(presentation::HELP), area.width);
            let maximum = text
                .lines
                .len()
                .saturating_sub(area.height.saturating_sub(2) as usize)
                .min(u16::MAX as usize) as u16;
            frame.render_widget(
                Paragraph::new(text)
                    .scroll((app.help_scroll.min(maximum), 0))
                    .block(
                        Block::default()
                            .borders(Borders::BOTTOM)
                            .title_bottom(" PageUp / PageDown   Esc back ")
                            .border_style(muted()),
                    ),
                area,
            );
        }
        return;
    }
    let columns = Layout::horizontal([
        Constraint::Length(if area.width >= 110 { 30 } else { 0 }),
        Constraint::Min(1),
    ])
    .split(area);
    if columns[0].width > 0 {
        sidebar(frame, app, columns[0]);
    }
    let main = inset(columns[1], if area.width >= 72 { 2 } else { 1 }, 0);
    let terminals_open = view.is_some_and(|v| v.terminals.open);
    let panel_open = view.is_some_and(|v| v.panel.is_some());
    let overlay = terminals_open || panel_open;
    let status = presentation::notice(&app.status);
    let status = if status.starts_with("Your workspace is ready.")
        || status.starts_with("Overview ready.")
        || status.starts_with("Back in Helm.")
    {
        String::new()
    } else {
        status
    };
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(if overlay { 1 } else { 6 }),
        Constraint::Length(if status.is_empty() { 1 } else { 3 }),
    ])
    .split(main);
    let title = view.map_or_else(|| "New conversation".into(), |v| safe(&v.title()));
    let host = app
        .selected
        .map_or_else(|| "Helm".into(), |t| app.route_label(t.route));
    let heading = if rows[0].width >= 70 {
        Line::from(vec![
            Span::styled(format!("{host}  /  "), muted()),
            Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::styled(title, Style::default().add_modifier(Modifier::BOLD))
    };
    let detail = view.map_or_else(
        || "Ctrl+N  Start a voyage".into(),
        |v| {
            if rows[0].width < 60 {
                return format!("{host} · {}", state(v));
            }
            format!(
                "{}{}   {}",
                if rows[0].width < 70 {
                    format!("{host} · ")
                } else {
                    String::new()
                },
                state(v),
                v.terminals.summary()
            )
        },
    );
    frame.render_widget(
        Paragraph::new(vec![heading, Line::styled(detail, muted())]).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(muted()),
        ),
        rows[0],
    );
    let has_interactions = !overlay
        && view
            .and_then(|v| v.snapshot.as_ref())
            .is_some_and(|s| !s.decisions.is_empty());
    let content = Layout::default()
        .direction(if rows[1].width >= 100 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if !has_interactions {
            [Constraint::Percentage(100), Constraint::Length(0)]
        } else if rows[1].width >= 100 {
            [Constraint::Percentage(60), Constraint::Percentage(40)]
        } else {
            [Constraint::Min(0), Constraint::Length(17)]
        })
        .split(rows[1]);
    if terminals_open {
        super::terminals::draw(frame, app, rows[1]);
    } else {
        conversation(frame, app, content[0]);
    }
    if overlay {
        frame.render_widget(
            Paragraph::new(if rows[2].width >= 40 {
                "Esc  Back to conversation · Draft saved"
            } else {
                "Esc Back · Draft saved"
            })
            .style(muted()),
            rows[2],
        );
    } else {
        composer(frame, app, rows[2]);
    }
    let shortcuts = if main.width >= 80 {
        "F1 Help   F2 Requests   F3 Console   F8 Explore   Tab Voyages   Ctrl+C Leave"
    } else if main.width >= 60 {
        "F1 Help  F2 Requests  F3 Console  F8 Explore  Tab Voyages"
    } else {
        "F1 Help F2 Review F3 Console F8 More"
    };
    frame.render_widget(
        Paragraph::new({
            let mut text = Text::from(Line::styled(shortcuts, muted()));
            text.lines
                .extend(presentation::wrap(Text::raw(status), rows[3].width).lines);
            text
        }),
        rows[3],
    );
    super::interactions::draw(
        frame,
        app,
        if has_interactions {
            content[1]
        } else {
            Rect::default()
        },
    );
}

fn sidebar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(muted()),
        area,
    );
    let area = inset(
        Rect {
            width: area.width.saturating_sub(1),
            ..area
        },
        2,
        1,
    );
    let rows = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("HELM", Style::default().add_modifier(Modifier::BOLD)),
            Line::default(),
            Line::styled("Ctrl+N  New voyage", accent()),
            Line::default(),
            Line::styled("Your voyages", muted()),
        ]),
        rows[0],
    );
    let entries = app
        .views
        .iter()
        .map(|(target, view)| {
            let mut title = presentation::wrap(
                Text::raw(safe(&view.title())),
                rows[1].width.saturating_sub(2),
            )
            .lines;
            if title.len() > 2 {
                title.truncate(2);
                if let Some(last) = title.last_mut() {
                    let value = last.to_string();
                    let mut value = value
                        .chars()
                        .take(rows[1].width.saturating_sub(5) as usize)
                        .collect::<String>();
                    value.push_str("...");
                    *last = Line::from(value);
                }
            }
            let mut lines = vec![Line::styled(
                format!(
                    "{}  {}",
                    app.route_label(target.route),
                    if view.unread { "Unread" } else { state(view) }
                ),
                muted(),
            )];
            lines.extend(title);
            lines.push(Line::default());
            ListItem::new(lines)
        })
        .collect::<Vec<_>>();
    let index = app.views.keys().position(|t| Some(*t) == app.selected);
    frame.render_stateful_widget(
        List::new(entries)
            .highlight_symbol("> ")
            .highlight_style(accent().add_modifier(Modifier::BOLD)),
        rows[1],
        &mut ListState::default().with_selected(index),
    );
    frame.render_widget(
        Paragraph::new("Tab  Switch voyage\nF8   Explore").style(muted()),
        rows[2],
    );
}

fn composer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let view = app.selected.and_then(|t| app.views.get(&t));
    let pending = view.is_some_and(|v| v.pending.is_some());
    let box_area = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let border = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if pending {
            Style::default().fg(Color::Yellow)
        } else {
            muted()
        });
    let inner = border.inner(box_area);
    frame.render_widget(border, box_area);
    let draft = view.map_or("", |v| v.draft.text.as_str());
    let body = Rect {
        height: inner.height.saturating_sub(1),
        ..inner
    };
    let cursor = view.map(|v| {
        super::composer::cursor_position(&safe(&v.draft.text[..v.draft.cursor]), body.width)
    });
    let scroll = cursor.map_or(0, |(row, _)| {
        row.saturating_sub(body.height.saturating_sub(1))
    });
    let text = if draft.is_empty() {
        Text::styled("Ask anything, or describe what you want to do...", muted())
    } else {
        presentation::wrap(Text::raw(safe(draft)), body.width)
    };
    frame.render_widget(Paragraph::new(text).scroll((scroll, 0)), body);
    let model = view
        .and_then(|v| v.snapshot.as_ref())
        .map_or("Choose a voyage", |s| s.model.as_str());
    let hint = if pending {
        "Not confirmed yet · F4 Check status"
    } else if area.width >= 60 {
        "Enter Send · Alt+Enter New line"
    } else {
        "Enter Send"
    };
    let controls = format!("{}   |   {hint}", safe(model));
    frame.render_widget(
        Paragraph::new(controls).style(if pending {
            Style::default().fg(Color::Yellow)
        } else {
            muted()
        }),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
    if let Some((row, column)) = cursor
        && body.height > 0
    {
        frame.set_cursor_position((
            body.x + column.min(body.width.saturating_sub(1)),
            body.y + row.saturating_sub(scroll).min(body.height - 1),
        ));
    }
}
fn conversation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let view = app.selected.and_then(|key| app.views.get(&key));
    let area = inset(area, 1, 1);
    let area = if area.width > 100 {
        Rect::new(area.x + (area.width - 100) / 2, area.y, 100, area.height)
    } else {
        area
    };
    let width = area.width;
    let mut text = Text::default();

    if let Some(view) = view {
        if let Some(panel) = &view.panel {
            text = super::panels::display(panel, width);
        } else {
            let mut cache = view.rendered.borrow_mut();
            if let Some((_, cached)) = cache.as_ref().filter(|(w, _)| *w == width) {
                text = cached.clone();
            } else {
                if let Some(snapshot) = &view.snapshot {
                    if snapshot.lifecycle["deleted"] == true {
                        text.lines
                            .push(Line::from("This conversation has been deleted."));
                    } else if snapshot.lifecycle["archived"] == true {
                        text.lines.push(Line::from(
                            "Archived. Use /restore to continue this conversation.",
                        ));
                    }
                    if snapshot.messages.is_empty() {
                        text.lines.extend([
                            Line::styled(
                                "Start a conversation",
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Line::from("Describe what you want to do in the message box below."),
                            Line::from(
                                "F1 shows commands. F3 opens interactive program terminals.",
                            ),
                        ]);
                    }
                    if snapshot.history_truncated || snapshot.messages.len() > 400 {
                        text.lines.push(Line::from(
                            "Showing recent messages. Export the conversation to keep a complete copy.",
                        ));
                    }
                    for message in snapshot
                        .messages
                        .iter()
                        .skip(snapshot.messages.len().saturating_sub(400))
                        .filter(|m| {
                            matches!(m.role.as_str(), "user" | "assistant") && !m.content.is_empty()
                        })
                    {
                        text.lines.push(Line::styled(
                            if message.role == "user" {
                                "You"
                            } else {
                                "Assistant"
                            },
                            Style::default()
                                .fg(if message.role == "user" {
                                    Color::Cyan
                                } else {
                                    Color::Reset
                                })
                                .add_modifier(Modifier::BOLD),
                        ));
                        if let Some(operator) = presentation::operator_message(&message.content)
                            .or_else(|| {
                                (message.role == "assistant")
                                    .then(|| presentation::structured_message(&message.content))
                                    .flatten()
                            })
                        {
                            text.lines
                                .extend(presentation::wrap(Text::raw(operator), width).lines);
                            text.lines.push(Line::default());
                            continue;
                        }
                        text.lines.extend(
                            markdown::render_markdown(
                                &message.content,
                                RenderOptions {
                                    width: usize::from(width),
                                    max_output_lines: 2000,
                                    ..RenderOptions::default()
                                },
                            )
                            .lines,
                        );
                        text.lines.push(Line::default());
                        if text.lines.len() > 20_000 {
                            text.lines.drain(..text.lines.len() - 20_000);
                        }
                    }
                    if let Some(run) = &snapshot.run
                        && run.state != "completed"
                    {
                        text.lines.push(Line::styled(
                            presentation::run_state(&run.state),
                            Style::default().fg(Color::Yellow),
                        ));
                        if run.partial_text_truncated {
                            text.lines
                                .push(Line::from("Showing the latest part of the response."));
                        }
                        if !run.partial_text.is_empty() {
                            text.lines.extend(
                                markdown::render_markdown(
                                    &run.partial_text,
                                    RenderOptions {
                                        width: usize::from(width),
                                        ..RenderOptions::default()
                                    },
                                )
                                .lines,
                            );
                        }
                    }
                } else {
                    text.lines.push(Line::from("Connecting to your voyage..."));
                }
                text = presentation::wrap(text, width);
                *cache = Some((width, text.clone()));
            }
            if let Some(error) = &view.error {
                text.lines.extend(
                    presentation::wrap(
                        Text::styled(
                            presentation::notice(error),
                            Style::default().fg(Color::Yellow),
                        ),
                        width,
                    )
                    .lines,
                );
            }
        }
    } else {
        text = presentation::wrap(
            Text::raw(
                "Welcome to Helm\n\nCtrl+N creates a voyage in the current workspace.\nEach voyage keeps running when you disconnect.\n\nF1 Help / F3 Terminals",
            ),
            width,
        );
    }
    let height = area.height as usize;
    let maximum = text
        .lines
        .len()
        .saturating_sub(height)
        .min(u16::MAX as usize) as u16;
    let scroll = view.map_or(0, |v| v.scroll).min(maximum);
    let offset = if view.is_some_and(|v| v.panel.is_some()) {
        scroll
    } else {
        maximum.saturating_sub(scroll)
    };

    frame.render_widget(Paragraph::new(text).scroll((offset, 0)), area);
}
