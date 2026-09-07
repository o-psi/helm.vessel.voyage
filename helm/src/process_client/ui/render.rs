use super::{App, presentation};
use crate::process_client::safe;
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
    if view.process.archive.is_some() {
        return "Archived · stopped";
    }
    if view.archived() {
        return "Archiving · cleanup pending";
    }
    if view.process.state == voyage_protocol::process::ProcessState::Unavailable {
        if view.error.is_some() {
            "Recovery needed"
        } else {
            "Recovering"
        }
    } else if view.error.is_some() {
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
            .map_or("Ready", presentation::voyage_state)
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    app.clear_inference_hits();
    app.sync_interactions();
    app.sidebar.hits.borrow_mut().clear();
    app.sidebar.visible.set(None);
    app.draft_hits.borrow_mut().clear();
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
    if app.active_draft.is_some() {
        app.draw_new_draft(frame, columns[1]);
        app.draw_inference_picker(frame);
        return;
    }
    let main = inset(columns[1], if area.width >= 72 { 2 } else { 1 }, 0);
    let terminals_open = view.is_some_and(|v| v.terminals.open);
    let panel_open = view.is_some_and(|v| v.panel.is_some());
    let reviewing = app.interactions.borrow().focused;
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
        Constraint::Length(if reviewing {
            0
        } else {
            app.completion_height()
        }),
        Constraint::Length(if overlay || reviewing { 1 } else { 6 }),
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
            if reviewing || rows[0].width < 60 {
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
                if v.archived() {
                    "History preserved".into()
                } else {
                    v.terminals.summary()
                }
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
    let has_interactions = reviewing;
    let content = Layout::default()
        .direction(if rows[1].width >= 100 {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if !has_interactions {
            [Constraint::Percentage(100), Constraint::Length(0)]
        } else {
            [Constraint::Length(0), Constraint::Percentage(100)]
        })
        .split(rows[1]);
    if terminals_open {
        super::terminals::draw(frame, app, rows[1]);
    } else {
        conversation(frame, app, content[0]);
    }
    if !reviewing {
        app.draw_completion(frame, rows[2]);
    }
    if overlay || reviewing {
        frame.render_widget(
            Paragraph::new(if reviewing {
                "Your conversation draft is saved."
            } else if rows[3].width >= 40 {
                "Esc  Back to conversation · Draft saved"
            } else {
                "Esc Back · Draft saved"
            })
            .style(muted()),
            rows[3],
        );
    } else {
        composer(frame, app, rows[3]);
    }
    let shortcuts = if reviewing {
        "Ctrl+C Leave"
    } else if main.width >= 80 {
        "F1 Help   F3 Console   F5 Archives   F9 Actions   Ctrl+C Leave"
    } else if main.width >= 60 {
        "F1 Help  F3 Console  F5 Archives  F9 Actions"
    } else {
        "F1 Help F3 Console F9 Menu"
    };
    frame.render_widget(
        Paragraph::new({
            let mut text = Text::from(Line::styled(shortcuts, muted()));
            text.lines
                .extend(presentation::wrap(Text::raw(status), rows[4].width).lines);
            text
        }),
        rows[4],
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
    app.draw_actions(frame);
    app.draw_inference_picker(frame);
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
        Constraint::Length(5 + app.new_drafts.len().min(4) as u16),
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
        ]),
        rows[0],
    );
    app.draw_draft_links(
        frame,
        Rect::new(
            rows[0].x,
            rows[0].y + 3,
            rows[0].width,
            app.new_drafts.len().min(4) as u16,
        ),
    );
    frame.render_widget(
        Paragraph::new(if app.archives {
            "Archived voyages"
        } else {
            "Your voyages"
        })
        .style(muted()),
        Rect::new(
            rows[0].x,
            rows[0].bottom().saturating_sub(1),
            rows[0].width,
            1,
        ),
    );
    let targets = app.ordered_targets();
    let mut heights = Vec::new();
    let list_area = Rect {
        width: rows[1].width.saturating_sub(3),
        ..rows[1]
    };
    let entries = targets
        .iter()
        .map(|target| {
            let view = &app.views[target];
            let mut title = presentation::wrap(
                Text::raw(safe(&view.title())),
                list_area.width.saturating_sub(2),
            )
            .lines;
            if title.len() > 2 {
                title.truncate(2);
                if let Some(last) = title.last_mut() {
                    let value = last.to_string();
                    let mut value = value
                        .chars()
                        .take(list_area.width.saturating_sub(5) as usize)
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
            heights.push(lines.len() as u16);
            ListItem::new(lines)
        })
        .collect::<Vec<_>>();
    let index = targets.iter().position(|t| Some(*t) == app.selected);
    let mut list_state = ListState::default().with_selected(index);
    frame.render_stateful_widget(
        List::new(entries)
            .highlight_symbol("> ")
            .highlight_style(accent().add_modifier(Modifier::BOLD)),
        list_area,
        &mut list_state,
    );
    let mut y = rows[1].y;
    for (index, target) in targets.iter().enumerate().skip(list_state.offset()) {
        if y >= rows[1].bottom() {
            break;
        }
        // List excludes entries that do not fit; never expose phantom hits.
        let height = heights[index];
        if height > rows[1].bottom() - y {
            break;
        }
        let button = Rect::new(list_area.right(), y, 3, height.min(2));
        let area = Rect::new(rows[1].x, y, rows[1].width, height);
        let over_button = app
            .sidebar
            .pointer
            .is_some_and(|point| button.contains(point));
        if !over_button {
            // A wrapped row includes padding; underlining it draws horizontal rules.
            frame.buffer_mut().set_style(
                area,
                app.hover_style(area, false)
                    .remove_modifier(Modifier::UNDERLINED),
            );
        }
        frame.render_widget(
            Paragraph::new(" ⋮ ").style(
                (if app.selected == Some(*target)
                    && app.sidebar.focus == super::sidebar::Focus::Button
                {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    accent()
                })
                .patch(app.hover_style(button, false)),
            ),
            button,
        );
        if index + 1 < targets.len() {
            // Reuse the padding row without changing list or hit geometry. Draw
            // after selection/hover so the rule stays muted across the full row.
            frame.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .style(Style::reset())
                    .border_style(muted()),
                Rect::new(rows[1].x, y + height - 1, rows[1].width, 1),
            );
        }
        app.sidebar.hits.borrow_mut().push(super::sidebar::Hit {
            area,
            button,
            target: *target,
            incarnation: app.views[target].process.incarnation,
        });
        y = y.saturating_add(height);
    }
    frame.render_widget(
        Paragraph::new("↑↓ Voyages · → Actions\nF5 Archives · F9 Actions").style(muted()),
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
    app.draw_inference_controls(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
    let active = view
        .and_then(|v| v.snapshot.as_ref())
        .is_some_and(|s| s.inference_next_turn);
    frame.render_widget(
        Paragraph::new(if pending {
            "Pending · F4 Check status · Text preserved"
        } else if active {
            "Next-turn settings · Enter Send · / Commands"
        } else {
            "Enter Send · Alt+Enter New line · / Commands"
        })
        .style(muted()),
        Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
    );
    if let Some((row, column)) = cursor
        && app.sidebar.menu.is_none()
        && app.sidebar.focus == super::sidebar::Focus::Composer
        && body.height > 0
    {
        frame.set_cursor_position((
            body.x + column.min(body.width.saturating_sub(1)),
            body.y + row.saturating_sub(scroll).min(body.height - 1),
        ));
    }
}
fn conversation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    super::transcript::draw(frame, app, area);
}
