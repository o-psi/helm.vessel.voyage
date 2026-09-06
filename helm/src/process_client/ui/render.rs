use super::App;
use crate::{
    markdown::{self, RenderOptions},
    process_client::safe,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(2),
        ])
        .split(area);
    let wide = area.width >= 80;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(if wide { 28 } else { 0 }),
            Constraint::Min(1),
        ])
        .split(rows[0]);
    if wide {
        let entries: Vec<_> = app
            .views
            .iter()
            .map(|(target, view)| {
                let selected = app.selected == Some(*target);
                let active = view
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.run.as_ref())
                    .is_some_and(|run| run.active());
                let decision = view
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| !s.decisions.is_empty());
                ListItem::new(format!(
                    "{}{} {} · {}",
                    if selected { ">" } else { " " },
                    if decision {
                        "!"
                    } else if view.unread {
                        "*"
                    } else if active {
                        "~"
                    } else {
                        " "
                    },
                    safe(&view.title()),
                    app.route_label(target.route)
                ))
                .style(if selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                })
            })
            .collect();
        frame.render_widget(
            List::new(entries).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Voyages · Tab switches"),
            ),
            columns[0],
        );
    }
    let view = app.selected.and_then(|key| app.views.get(&key));
    let mut text = Text::default();
    let mut title = "No voyage selected · Ctrl+N creates one".to_owned();
    if let Some(view) = view {
        title = format!("{} · {}", safe(&view.title()), view.process.session_id);
        let width = columns[1].width;
        let mut cache = view.rendered.borrow_mut();
        if let Some((_, cached)) = cache
            .as_ref()
            .filter(|(cached_width, _)| *cached_width == width)
        {
            text = cached.clone();
        } else {
            if let Some(snapshot) = &view.snapshot {
                text.lines.push(Line::from(format!(
                    "{} · revision {}",
                    safe(&snapshot.model),
                    snapshot.revision
                )));
                if snapshot.messages.len() > 400 {
                    text.lines
                        .push(Line::from("[Showing the most recent 400 messages]"));
                }
                if snapshot.history_truncated {
                    text.lines.push(Line::from("[Recent history projection; use connect request with history/message_chunk for earlier or large messages]"));
                }
                for message in snapshot
                    .messages
                    .iter()
                    .skip(snapshot.messages.len().saturating_sub(400))
                {
                    text.lines.push(Line::styled(
                        safe(&message.role),
                        Style::default().fg(Color::Cyan),
                    ));
                    text.lines.extend(
                        markdown::render_markdown(
                            &message.content,
                            RenderOptions {
                                width: width.saturating_sub(2) as usize,
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
                for decision in &snapshot.decisions {
                    text.lines.push(Line::styled(
                        format!(
                            "Decision {} (deadline {}):",
                            decision.decision_id, decision.expires_at_ms
                        ),
                        Style::default().fg(Color::Yellow),
                    ));
                    text.lines
                        .push(Line::from(safe(&decision.request.to_string())));
                    text.lines
                        .push(Line::from("/approve UUID · /deny UUID · /answer UUID text"));
                }
                if let Some(run) = &snapshot.run {
                    text.lines.push(Line::from(format!(
                        "Run {}: {}",
                        run.run_id,
                        safe(&run.state)
                    )));
                    // Completed turns are already present in canonical messages.
                    // Retain provisional output for active/interrupted work only.
                    if run.state != "completed" && !run.partial_text.is_empty() {
                        if run.partial_text_truncated {
                            text.lines.push(Line::from(
                                "[Partial run output; connect request with run_output retrieves the full text]",
                            ));
                        }
                        text.lines.extend(
                            markdown::render_markdown(
                                &run.partial_text,
                                RenderOptions {
                                    width: width.saturating_sub(2) as usize,
                                    ..RenderOptions::default()
                                },
                            )
                            .lines,
                        );
                    }
                }
            }
            *cache = Some((width, text.clone()));
        }
        if let Some(error) = &view.error {
            text.lines.push(Line::styled(
                safe(error),
                Style::default().fg(Color::Yellow),
            ));
        }
        if let Some(observed) = view.observed {
            text.lines.push(Line::from(format!(
                "Last observed {}s ago",
                observed.elapsed().as_secs()
            )));
        }
    }
    let scroll = view.map_or(0, |v| v.scroll);
    let automatic = text
        .lines
        .len()
        .saturating_sub(rows[0].height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((automatic.saturating_sub(scroll), 0))
            .block(Block::default().borders(Borders::ALL).title(title)),
        columns[1],
    );
    let draft = view.map_or("", |v| v.draft.text.as_str());
    let pending = view.is_some_and(|v| v.pending.is_some());
    frame.render_widget(
        Paragraph::new(safe(draft))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(if pending {
                "Delivery pending or unknown · /receipt resolves"
            } else {
                "Enter sends · Alt+Enter newline · /help"
            })),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(safe(&app.status)).wrap(Wrap { trim: false }),
        rows[2],
    );
    if let Some(view) = view {
        let (row, column) = super::composer::cursor_position(
            &view.draft.text[..view.draft.cursor],
            rows[1].width.saturating_sub(2),
        );
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1 + column)
                .min(area.right().saturating_sub(1)),
            rows[1]
                .y
                .saturating_add(1 + row)
                .min(rows[1].bottom().saturating_sub(2)),
        ));
    }
}
