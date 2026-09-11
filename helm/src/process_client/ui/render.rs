use super::{App, presentation};
use crate::process_client::safe;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};

fn muted() -> Style {
    crate::theme::Role::Muted.style()
}
fn accent() -> Style {
    crate::theme::Role::Focus.style()
}
fn inset(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    area.inner(ratatui::layout::Margin::new(horizontal, vertical))
}
fn state<'a>(view: &super::state::View, working: &'a super::effects::Working) -> &'a str {
    if view.process.archive.is_some() {
        return "Archived · stopped";
    }
    if view.archived() {
        return "Archiving · cleanup pending";
    }
    if view.connection_unavailable {
        return "Reconnecting";
    }
    if view
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.recovery_pending)
    {
        return "Conversation restored · cleanup pending";
    }
    let status = if view.process.state == voyage_protocol::process::ProcessState::Unavailable {
        if view.error.is_some() || view.connection_unavailable {
            "Recovery needed"
        } else {
            "Recovering"
        }
    } else if view.error.is_some() {
        if view.connection_unavailable {
            "Reconnecting"
        } else {
            "Conversation unavailable"
        }
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
    };
    working.label(
        status,
        view.process.state == voyage_protocol::process::ProcessState::Live,
    )
}

/// Status affects styling, never the space reserved for a voyage title.
fn sidebar_style(
    view: &super::state::View,
    now: chrono::DateTime<chrono::Utc>,
    retention_secs: u64,
) -> Style {
    use voyage_protocol::process::ProcessState;
    let attention = crate::theme::Role::AwaitingInput.style();
    let running = crate::theme::Role::Running.style();
    if view.process.archive.is_some() {
        return crate::theme::Role::Muted.style();
    }
    if view
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.recovery_pending)
    {
        return attention;
    }
    if view.archived() || view.process.state == ProcessState::CleanupUnconfirmed {
        return attention;
    }
    if matches!(
        view.process.state,
        ProcessState::Unavailable | ProcessState::Stopped | ProcessState::Relinquished
    ) {
        return attention;
    }
    if view.error.is_some() || view.connection_unavailable {
        return attention;
    }
    if let Some(snapshot) = &view.snapshot {
        if !snapshot.decisions.is_empty() {
            return attention;
        }
        // Cleanup obligations are recorded before work starts.
        if snapshot.pending_cleanup_run.is_some()
            && snapshot.run.as_ref().is_none_or(|run| !run.active())
        {
            return attention;
        }
        if view.sidebar_settled(now, retention_secs) {
            return crate::theme::Role::Muted.style();
        }
        if let Some(run) = &snapshot.run {
            match run.state.as_str() {
                "failed" => return crate::theme::Role::Failed.style(),
                "completed" => return crate::theme::Role::Completed.style(),
                "cancelled" => return crate::theme::Role::Muted.style(),
                "interrupted" | "awaiting_decision" => {
                    return attention;
                }
                _ => {}
            }
        }
    }
    match view.process.state {
        ProcessState::Suspended if view.snapshot.as_ref().is_some_and(|s| s.run.is_none()) => {
            crate::theme::Role::Completed.style()
        }
        ProcessState::Suspended => attention,
        ProcessState::Starting => running,
        ProcessState::Stopped | ProcessState::Relinquished => attention,
        _ => match view.snapshot.as_ref() {
            None => attention,
            Some(snapshot) => match snapshot.run.as_ref().map(|run| run.state.as_str()) {
                Some("accepted" | "running") => running,
                Some("cancel_requested") => running,
                Some("completed") | None => crate::theme::Role::Completed.style(),
                _ => attention,
            },
        },
    }
}

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    app.working.begin_frame();
    app.vessel_sidebar_button.set(Rect::default());
    draw_inner(frame, app);
    app.draw_vessel_control(frame);
    app.draw_workspace_picker(frame, frame.area());
    if let Some(manager) = &app.vessels {
        manager.borrow_mut().render(frame, frame.area());
    }
}

fn draw_inner(frame: &mut Frame<'_>, app: &App) {
    app.clear_inference_hits();
    app.sync_interactions();
    app.sidebar.hits.borrow_mut().clear();
    app.sidebar.action_trigger.set(None);
    app.sidebar.visible.set(None);
    app.draft_hits.borrow_mut().clear();
    let area = frame.area();
    let view = app.selected.and_then(|key| app.views.get(&key));
    if area.width < 40 || area.height < 18 {
        app.sidebar.resize.clear();
        super::interactions::draw(frame, app, Rect::default());
        if let Some(view) = view {
            view.terminals.clear_displayed();
        }
        frame.render_widget(Paragraph::new(presentation::wrap(Text::raw("Helm\nEnlarge to at least 40 columns x 18 rows.\nCtrl+C leaves; your work continues."), area.width)), area);
        return;
    }
    if app.help || app.explore.is_some() {
        app.sidebar.resize.clear();
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
    // Requests and actions own input, but never replace the conversation.
    // Hide voyage navigation first on narrow terminals to retain both panes.
    let reviewing = app.interactions.borrow().focused;
    let actions_open = app.sidebar.menu.is_some();
    let sidebar_open = reviewing || actions_open;
    let right_width = if sidebar_open {
        (area.width / 2).min(64)
    } else {
        0
    };
    let panes =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(right_width)]).split(area);
    let width = app.sidebar.resize.width(panes[0]);
    app.sidebar
        .resize
        .layout((width > 0 && !sidebar_open && !app.inference_picker_open()).then_some(panes[0]));
    let columns =
        Layout::horizontal([Constraint::Length(width), Constraint::Min(1)]).split(panes[0]);
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
    let overlay = terminals_open || panel_open;
    let status = presentation::notice(&app.status);
    let status = if status.starts_with("Your workspace is ready.")
        || status.starts_with("Overview ready.")
        || status.starts_with("Back in Helm.")
        || status == "Connected. Voyage state refreshed."
    {
        String::new()
    } else {
        status
    };
    let preview_rows = app.preview_rows(main.width.saturating_sub(2), main.height);
    let footer = footer(app, main.width, reviewing, overlay, &status);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(if reviewing {
            0
        } else {
            app.completion_height()
        }),
        Constraint::Length(if overlay || reviewing {
            1
        } else {
            5 + app.attachment_details().len() as u16 + preview_rows
        }),
        Constraint::Length(footer.lines.len().min(3) as u16),
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
    let status_prefix = if reviewing || rows[0].width < 70 {
        format!("{host} · ")
    } else {
        String::new()
    };
    let detail = view.map_or_else(
        || "Ctrl+N  Start a voyage".into(),
        |v| {
            if reviewing || rows[0].width < 60 {
                return format!("{host} · {}", state(v, &app.working));
            }
            format!(
                "{}{}   {}",
                if rows[0].width < 70 {
                    format!("{host} · ")
                } else {
                    String::new()
                },
                state(v, &app.working),
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
    // Bound the shimmer to the visible name, after its two-column spinner.
    let prefix_width =
        unicode_width::UnicodeWidthStr::width(status_prefix.as_str()).min(u16::MAX as usize) as u16;
    let word_x = rows[0].x.saturating_add(prefix_width).saturating_add(2);
    let right = rows[0].right().saturating_sub(
        if columns[0].width == 0 && !sidebar_open && view.is_some() {
            9
        } else {
            0
        },
    );
    if rows[0].height >= 2 && word_x < right {
        app.working.record(Rect::new(
            word_x,
            rows[0].y + 1,
            (right - word_x).min(app.working.width()),
            1,
        ));
    }
    if columns[0].width == 0 && !sidebar_open && view.is_some() {
        app.draw_action_trigger(
            frame,
            Rect::new(rows[0].right().saturating_sub(9), rows[0].y + 1, 9, 1),
        );
    }
    if terminals_open {
        super::terminals::draw(frame, app, rows[1]);
    } else {
        conversation(frame, app, rows[1]);
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
        composer(frame, app, rows[3], preview_rows);
    }
    frame.render_widget(Paragraph::new(footer), rows[4]);
    super::interactions::draw(
        frame,
        app,
        if reviewing { panes[1] } else { Rect::default() },
    );
    app.draw_actions(frame, panes[1]);
    app.draw_inference_picker(frame);
}

fn footer(app: &App, width: u16, reviewing: bool, overlay: bool, status: &str) -> Text<'static> {
    let view = app.selected.and_then(|key| app.views.get(&key));
    let pending = view.is_some_and(|view| view.pending.is_some());
    let active = view
        .and_then(|view| view.snapshot.as_ref())
        .is_some_and(|snapshot| snapshot.inference_next_turn);
    let hint = if reviewing || overlay {
        ""
    } else if pending {
        "Pending · Checking automatically · Text preserved"
    } else if active {
        "Next-turn settings · Enter Send · / Commands"
    } else if width >= 60 {
        "Enter Send · Alt+Enter New line · / Commands"
    } else {
        "Enter Send · / Commands"
    };
    let mut line = Line::styled(hint.to_owned(), muted());
    for shortcuts in [
        "F1 Help · F3 Console · F5 Archives · F9 Actions · Ctrl+C Leave",
        "F1 Help · F3 Console · F9 Actions",
        "F1 Help",
    ] {
        let shortcuts = if reviewing { "Ctrl+C Leave" } else { shortcuts };
        let separator = if hint.is_empty() { "" } else { " · " };
        let suffix = format!("{separator}{shortcuts}");
        if line.width() + Line::raw(suffix.clone()).width() <= width as usize {
            line.spans.push(Span::styled(suffix, muted()));
            break;
        }
    }
    let mut text = presentation::wrap(Text::from(line), width);
    if !status.is_empty() {
        let notice = Line::raw(status.to_owned());
        if text.lines.len() == 1 && text.lines[0].width() + 3 + notice.width() <= width as usize {
            text.lines[0].spans.push(Span::styled(" · ", muted()));
            text.lines[0].spans.extend(notice.spans);
        } else {
            text.lines
                .extend(presentation::wrap(Text::from(notice), width).lines);
        }
    }
    text
}

/// Trailing host metadata can be clipped, but must never displace the title.
fn sidebar_text(title: &str, host: Option<&str>) -> Text<'static> {
    let mut line = Line::raw(safe(title));
    if let Some(host) = host {
        line.spans
            .push(Span::styled(format!(" · {}", safe(host)), muted()));
    }
    Text::from(vec![line, Line::default()])
}

fn sidebar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let divider_style = if app.sidebar.resize.highlighted(app.sidebar.pointer) {
        accent().add_modifier(Modifier::BOLD)
    } else {
        muted()
    };
    frame.render_widget(
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(divider_style),
        area,
    );
    frame.render_widget(
        Paragraph::new("↔").style(divider_style),
        Rect::new(area.right() - 1, area.y + area.height / 2, 1, 1),
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
            Line::styled(
                "Vessels  [Ctrl+G]",
                accent()
                    .add_modifier(Modifier::BOLD)
                    .patch(app.vessel_hover_style(Rect::new(
                        rows[0].x,
                        rows[0].y,
                        rows[0].width,
                        1,
                    ))),
            ),
            Line::default(),
            Line::styled("Ctrl+N  New voyage", accent()),
            Line::default(),
        ]),
        rows[0],
    );
    app.vessel_sidebar_button
        .set(Rect::new(rows[0].x, rows[0].y, rows[0].width, 1));
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
    let multiple_vessels = targets.first().is_some_and(|first| {
        targets
            .iter()
            .any(|target| target.route.id != first.route.id)
    });
    let mut heights = Vec::new();
    let list_area = Rect {
        width: rows[1].width.saturating_sub(3),
        ..rows[1]
    };
    let entries = targets
        .iter()
        .map(|target| {
            let view = &app.views[target];
            let style = sidebar_style(view, app.presentation_now, app.settle_after_secs);
            let host = multiple_vessels.then(|| app.route_label(target.route));
            // Every state gets one title row plus the existing divider row.
            let text = sidebar_text(&view.title(), host.as_deref());
            heights.push(text.lines.len() as u16);
            ListItem::new(text).style(style)
        })
        .collect::<Vec<_>>();
    let index = targets.iter().position(|t| Some(*t) == app.selected);
    let mut list_state = ListState::default().with_selected(index);
    frame.render_stateful_widget(
        List::new(entries).highlight_style(crate::theme::Role::Selection.style()),
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
        let button = Rect::new(list_area.right(), y, 3, height.saturating_sub(1));
        let area = Rect::new(rows[1].x, y, rows[1].width, height);
        let over_button = app
            .sidebar
            .pointer
            .is_some_and(|point| button.contains(point));
        if !over_button {
            // Only add hover modifiers: keep title status and muted host colours.
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
                    crate::theme::Role::Selection
                        .style()
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

fn composer(frame: &mut Frame<'_>, app: &App, area: Rect, preview_rows: u16) {
    let view = app.selected.and_then(|t| app.views.get(&t));
    let pending = view.is_some_and(|v| v.pending.is_some());
    let box_area = area;
    let border = Block::default()
        .borders(Borders::ALL)
        .title(app.attachment_summary())
        .border_type(BorderType::Rounded)
        .border_style(if pending {
            crate::theme::Role::AwaitingInput.style()
        } else {
            muted()
        });
    let inner = border.inner(box_area);
    frame.render_widget(border, box_area);
    let draft = view.map_or("", |v| v.draft.text.as_str());
    let details = app.attachment_details();
    let detail_rows = (details.len() as u16).min(inner.height.saturating_sub(2));
    let preview_rows = preview_rows.min(inner.height.saturating_sub(3 + detail_rows));
    let body = Rect {
        height: inner.height.saturating_sub(1 + detail_rows + preview_rows),
        ..inner
    };
    if let Some(view) = view {
        view.draft.viewport_width.set(body.width);
    }
    let cursor = view
        .map(|v| super::composer::cursor_position_at(&v.draft.text, v.draft.cursor, body.width));
    let scroll = cursor.map_or(0, |(row, _)| {
        row.saturating_sub(body.height.saturating_sub(1))
    });
    let text = if draft.is_empty() {
        Text::styled("Ask anything, or describe what you want to do...", muted())
    } else {
        presentation::wrap(
            super::attachments::styled_draft(&view.expect("nonempty draft").draft),
            body.width,
        )
    };
    frame.render_widget(Paragraph::new(text).scroll((scroll, 0)), body);
    app.draw_previews(
        frame,
        Rect::new(inner.x, body.bottom(), inner.width, preview_rows),
    );
    frame.render_widget(
        Paragraph::new(details.join("\n")).style(crate::theme::Role::Focus.style()),
        Rect::new(
            inner.x,
            body.bottom() + preview_rows,
            inner.width,
            detail_rows,
        ),
    );
    app.draw_inference_controls(
        frame,
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
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

#[cfg(test)]
mod sidebar_tests {
    use super::*;
    use crate::process_client::ui::state::View;
    use crate::theme::Role;
    use ratatui::{buffer::Buffer, widgets::StatefulWidget};
    use serde_json::json;
    use uuid::Uuid;

    fn view(run_state: &str) -> View {
        let session = Uuid::new_v4();
        let mut view = View::new(
            serde_json::from_value(json!({
                "session_id": session, "incarnation": Uuid::new_v4(),
                "workspace": "/tmp", "state": "live", "name": "Voyage title"
            }))
            .unwrap(),
        );
        view.snapshot = Some(
            serde_json::from_value(json!({
                "session_id": session, "revision": 1, "model": "fixture", "messages": [],
                "run": {"run_id": Uuid::new_v4(), "state": run_state}
            }))
            .unwrap(),
        );
        view
    }

    #[test]
    fn titles_use_one_content_row_for_every_status_and_preserve_styles() {
        let now = chrono::Utc::now();
        for (status, role) in [
            ("accepted", Role::Running),
            ("running", Role::Running),
            ("cancel_requested", Role::Running),
            ("completed", Role::Completed),
            ("failed", Role::Failed),
            ("cancelled", Role::Muted),
            ("interrupted", Role::AwaitingInput),
            ("awaiting_decision", Role::AwaitingInput),
        ] {
            let view = view(status);
            assert_eq!(sidebar_style(&view, now, 300), role.style(), "{status}");
            let text = sidebar_text(&view.title(), None);
            assert_eq!(text.lines.len(), 2);
            assert_eq!(text.lines[0].to_string(), "Voyage title");
            assert_eq!(text.lines[1].width(), 0);
        }
        let mut view = view("completed");
        view.observe_settlement(now);
        assert_eq!(sidebar_style(&view, now, 0), Role::Muted.style());
        view.connection_unavailable = true;
        assert_eq!(sidebar_style(&view, now, 0), Role::AwaitingInput.style());
    }

    #[test]
    fn process_attention_and_suspension_keep_their_styles() {
        use voyage_protocol::process::ProcessState;
        let now = chrono::Utc::now();
        let mut view = view("running");
        for process in [
            ProcessState::Unavailable,
            ProcessState::Stopped,
            ProcessState::Relinquished,
            ProcessState::CleanupUnconfirmed,
        ] {
            view.process.state = process;
            assert_eq!(sidebar_style(&view, now, 300), Role::AwaitingInput.style());
        }
        view.process.state = ProcessState::Starting;
        assert_eq!(sidebar_style(&view, now, 300), Role::Running.style());
        view.process.state = ProcessState::Suspended;
        view.snapshot.as_mut().unwrap().run = None;
        assert_eq!(sidebar_style(&view, now, 300), Role::Completed.style());
        view.snapshot.as_mut().unwrap().recovery_pending = true;
        assert_eq!(sidebar_style(&view, now, 300), Role::AwaitingInput.style());
    }

    #[test]
    fn narrow_list_prioritizes_unicode_titles_and_keeps_selected_entry_visible() {
        for width in [1, 8, 20, 40] {
            let area = Rect::new(0, 0, width, 4);
            let mut buffer = Buffer::empty(area);
            let items =
                (0..8).map(|_| ListItem::new(sidebar_text("界界界界 title", Some("Remote"))));
            let mut state = ListState::default().with_selected(Some(7));
            StatefulWidget::render(
                List::new(items).highlight_style(Role::Selection.style()),
                area,
                &mut buffer,
                &mut state,
            );
            assert_eq!(state.offset(), 6);
            if width >= 8 {
                assert_eq!(buffer[(0, 2)].symbol(), "界");
                assert!(buffer[(0, 2)].modifier.contains(Modifier::REVERSED));
                assert_eq!(buffer[(6, 2)].symbol(), "界");
            }
        }
        let text = sidebar_text("Title", Some("Remote"));
        assert_eq!(text.lines[0].to_string(), "Title · Remote");
        assert_eq!(text.lines[0].spans[1].style, muted());
        assert_eq!(sidebar_text("Title", None).lines[0].to_string(), "Title");
    }
}
