use super::{App, presentation};
use crate::{
    markdown::{self, RenderOptions},
    process_client::safe,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

fn block(title: impl Into<Line<'static>>, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(color))
}

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < 40 || area.height < 18 {
        super::interactions::draw(frame, app, Rect::default());
        if let Some(view) = app.selected.and_then(|t| app.views.get(&t)) {
            view.terminals.clear_displayed();
        }
        frame.render_widget(Paragraph::new(presentation::wrap(Text::raw("HELM\nEnlarge to at least 40 columns x 18 rows.\nCtrl+C detaches; voyages continue."), area.width)), area);
        return;
    }
    if app.help {
        super::interactions::draw(frame, app, Rect::default());
        if let Some(view) = app.selected.and_then(|t| app.views.get(&t)) {
            view.terminals.clear_displayed();
        }
        let text = presentation::wrap(Text::raw(presentation::HELP), area.width.saturating_sub(2));
        let maximum = text
            .lines
            .len()
            .saturating_sub(area.height.saturating_sub(2) as usize)
            .min(u16::MAX as usize) as u16;
        frame.render_widget(
            Paragraph::new(text)
                .scroll((app.help_scroll.min(maximum), 0))
                .block(block(
                    " HELM HELP / PageUp PageDown / Esc back ",
                    Color::Cyan,
                )),
            area,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(4),
        Constraint::Length(5),
        Constraint::Length(3),
    ])
    .split(area);
    let view = app.selected.and_then(|key| app.views.get(&key));
    let heading = view.map_or_else(
        || "HELM / No voyage selected".into(),
        |v| format!(" HELM / {} ", safe(&v.title())),
    );
    let summary = view.map_or_else(
        || "Ctrl+N creates a voyage".into(),
        |v| {
            let model = v
                .snapshot
                .as_ref()
                .map_or("Observing", |s| s.model.as_str());
            let state = v
                .snapshot
                .as_ref()
                .and_then(|s| s.run.as_ref())
                .map_or("Ready", |r| presentation::run_state(&r.state));
            let state = if v.error.is_some() {
                "Needs attention"
            } else if v.snapshot.as_ref().is_some_and(|s| !s.decisions.is_empty()) {
                "Waiting for you"
            } else {
                state
            };
            let host = app
                .selected
                .map_or_else(String::new, |t| app.route_label(t.route));
            format!(
                "{state} | {} | {}\n{}",
                safe(model),
                safe(&host),
                v.terminals.summary()
            )
        },
    );
    frame.render_widget(
        Paragraph::new(summary).block(block(heading, Color::Cyan)),
        rows[0],
    );
    let columns = Layout::horizontal([
        Constraint::Length(if area.width >= 110 { 24 } else { 0 }),
        Constraint::Min(1),
    ])
    .split(rows[1]);
    if columns[0].width > 0 {
        let entries: Vec<_> = app
            .views
            .iter()
            .map(|(target, view)| {
                let state = if view.error.is_some() {
                    "Unavailable"
                } else if view
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| !s.decisions.is_empty())
                {
                    "Needs you"
                } else if view
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.run.as_ref())
                    .is_some_and(|r| r.active())
                {
                    "Working"
                } else if view.unread {
                    "Unread"
                } else {
                    "Ready"
                };
                ListItem::new(vec![
                    Line::styled(
                        safe(&view.title()),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Line::from(format!("{state} / {}", app.route_label(target.route))),
                ])
            })
            .collect();
        let index = app.views.keys().position(|t| Some(*t) == app.selected);
        frame.render_stateful_widget(
            List::new(entries)
                .block(block(" VOYAGES / Tab ", Color::DarkGray))
                .highlight_symbol("> ")
                .highlight_style(Style::default().fg(Color::Cyan)),
            columns[0],
            &mut ListState::default().with_selected(index),
        );
    }
    let terminals_open = view.is_some_and(|v| v.terminals.open);
    let panel_open = view.is_some_and(|v| v.panel.is_some());
    let has_interactions = !terminals_open
        && !panel_open
        && view
            .and_then(|v| v.snapshot.as_ref())
            .is_some_and(|s| !s.decisions.is_empty());
    let content = Layout::default()
        .direction(if columns[1].width >= 100 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if !has_interactions {
            [Constraint::Percentage(100), Constraint::Length(0)]
        } else if columns[1].width >= 100 {
            [Constraint::Percentage(60), Constraint::Percentage(40)]
        } else {
            [Constraint::Min(3), Constraint::Length(17)]
        })
        .split(columns[1]);
    if terminals_open {
        super::terminals::draw(frame, app, columns[1]);
    } else {
        conversation(frame, app, content[0]);
    }
    let draft = view.map_or("", |v| v.draft.text.as_str());
    let pending = view.is_some_and(|v| v.pending.is_some());
    let composer_title = if terminals_open || panel_open {
        " DRAFT SAVED / Esc returns to chat "
    } else if pending {
        " NOT CONFIRMED YET / F4 check status "
    } else {
        " MESSAGE / Enter send / Alt+Enter newline "
    };
    let composer_width = rows[2].width.saturating_sub(2);
    let draft_text = presentation::wrap(Text::raw(safe(draft)), composer_width);
    let cursor = view.map(|v| {
        let (row, column) = super::composer::cursor_position(
            &safe(&v.draft.text[..v.draft.cursor]),
            composer_width,
        );
        (usize::from(row), usize::from(column))
    });
    let composer_scroll = cursor
        .map_or(0, |(row, _)| {
            row.saturating_sub(rows[2].height.saturating_sub(3) as usize)
        })
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(draft_text)
            .scroll((composer_scroll, 0))
            .block(block(
                composer_title,
                if terminals_open || panel_open {
                    Color::DarkGray
                } else {
                    Color::Cyan
                },
            )),
        rows[2],
    );
    let shortcuts = if area.width >= 110 {
        "F1 Help    F2 Questions & approvals    F3 Terminals    F4 Check status    Tab Voyages    Ctrl+C Leave"
    } else if area.width >= 72 {
        "F1 Help  F2 Requests  F3 Terminals  F4 Status  Tab Voyages  Ctrl+C Leave"
    } else {
        "F1 Help  F2 Review  F3 Programs  F4 Status"
    };
    let mut footer = Text::from(Line::styled(shortcuts, Style::default().fg(Color::Cyan)));
    footer.lines.extend(
        presentation::wrap(Text::raw(presentation::notice(&app.status)), rows[3].width).lines,
    );
    frame.render_widget(Paragraph::new(footer), rows[3]);
    if !terminals_open
        && !panel_open
        && let Some((row, column)) = cursor
    {
        frame.set_cursor_position((
            rows[2].x + 1 + column as u16,
            (rows[2].y + 1 + (row as u16).saturating_sub(composer_scroll))
                .min(rows[2].bottom() - 2),
        ));
    }
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

fn conversation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let view = app.selected.and_then(|key| app.views.get(&key));
    let width = area.width.saturating_sub(2);
    let mut text = Text::default();
    let mut title = " CONVERSATION ".to_owned();
    if let Some(view) = view {
        if let Some(panel) = &view.panel {
            title = " OVERVIEW / PageUp PageDown / Esc back ".into();
            text = markdown::render_markdown(
                panel,
                RenderOptions {
                    width: usize::from(width),
                    max_output_lines: 2000,
                    ..RenderOptions::default()
                },
            );
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
                                "YOU"
                            } else {
                                "ASSISTANT"
                            },
                            Style::default()
                                .fg(if message.role == "user" {
                                    Color::Cyan
                                } else {
                                    Color::Green
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
    let height = area.height.saturating_sub(2) as usize;
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
    if maximum > 0 {
        title.push_str(&format!(" {}/{} ", offset + 1, maximum + 1));
    }
    frame.render_widget(
        Paragraph::new(text)
            .scroll((offset, 0))
            .block(block(title, Color::DarkGray)),
        area,
    );
}
