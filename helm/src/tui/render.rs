//! Frame composition and overlay rendering; priority stays explicit.

use super::{
    App,
    bridge::ApprovalRequest,
    composer::cursor_position,
    conversation::{transcript, viewport_width_for},
    models::draw_model_picker,
    palette::draw_slash_palette,
    questions::{draw_question, question_height},
    supervisor::draw_supervisor,
    terminals::{draw_attached_terminal, draw_terminal_picker},
    text::{centered, display_safe},
    todos::draw_todos,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(super) fn conversation_layout(area: Rect, app: &App) -> std::rc::Rc<[Rect]> {
    let area = super::recent::content_area(area, app);
    let constraints = if let Some(question) = &app.question {
        let header = if area.height >= 14 { 3 } else { 1 }.min(area.height);
        let status = u16::from(area.height >= 10);
        let available = area.height.saturating_sub(header + status);
        // Grow from the composer upward, but reserve at least a third of the
        // remaining viewport for the conversation, even for oversized prompts.
        let conversation = (available / 3).max(1).min(available);
        let question =
            question_height(question, area.width).min(available.saturating_sub(conversation));
        [
            Constraint::Length(header),
            Constraint::Min(conversation),
            Constraint::Length(question),
            Constraint::Length(status),
        ]
    } else {
        [
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(1),
        ]
    };
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
}

pub(super) fn draw(frame: &mut ratatui::Frame<'_>, app: &App) {
    let area = frame.area();
    if let Some(approval) = app
        .approval
        .as_ref()
        .filter(|request| app.question.is_none() && super::publication::exact(request))
    {
        super::publication::draw(frame, area, approval, app.approval_scroll);
        return;
    }
    if app.github_panel.open && app.approval.is_none() && app.question.is_none() {
        app.github_panel.draw(frame, area);
        return;
    }
    if app.terminal_panel.attached_terminal.is_some() {
        draw_attached_terminal(frame, area, &app.terminal_panel);
        return;
    }
    if app.question.is_none() && (area.width < 32 || area.height < 10) {
        frame.render_widget(
            Paragraph::new("Helm needs a terminal of at least 32×10. Resize the window or use `helm chat --plain`.")
                .wrap(Wrap { trim: true })
                .block(Block::default().title(" Helm ").borders(Borders::ALL)),
            area,
        );
        return;
    }
    if app.question.is_none() && app.shortcut_help && app.approval.is_none() {
        draw_shortcut_help(frame, area, app);
        return;
    }
    if app.question.is_none()
        && app.approval.is_none()
        && !app.voyage_panel.is_open()
        && !app.workflow_panel.is_open()
        && app.policy_panel.open
    {
        super::policy::draw(frame, area, app);
        return;
    }
    if app.question.is_none() && app.model_panel.model_picker {
        draw_model_picker(frame, area, &app.model_panel, &app.session.model);
        return;
    }
    if app.question.is_none() && app.supervisor_panel.supervisor_mode.is_some() {
        draw_supervisor(frame, area, &app.supervisor_panel);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
        return;
    }
    if app.question.is_none() && app.todo_panel.todo_mode.is_some() {
        draw_todos(frame, area, &app.todo_panel);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
        return;
    }
    if app.question.is_none() && app.voyage_panel.is_open() {
        app.voyage_panel.draw(frame, area);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
        return;
    }
    if app.question.is_none() && app.workflow_panel.is_open() {
        app.workflow_panel.draw(frame, area);
        if let Some(approval) = &app.approval {
            draw_approval(frame, area, approval);
        }
        return;
    }
    let chunks = conversation_layout(area, app);
    if let Some(sidebar) = super::recent::sidebar_area(area, app) {
        super::recent::draw(frame, sidebar, app);
    }
    let title = app.session.display_name();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    " HELM ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " access: {} · {}",
                    app.access_mode,
                    display_safe(&title)
                )),
            ]),
            Line::from(display_safe(&format!(
                " {} · {} · {}",
                app.session.model,
                app.provider_label,
                app.session.workspace.display()
            ))),
        ])
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    let transcript = transcript(app, viewport_width_for(chunks[1]));
    let viewport_height = chunks[1].height as usize;
    let viewport_width = chunks[1].width.max(1) as usize;
    let rendered_lines = transcript
        .lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(viewport_width.max(1)))
        .sum::<usize>();
    let bottom = rendered_lines.saturating_sub(viewport_height) as u16;
    let offset = bottom.saturating_sub(app.scroll);
    frame.render_widget(
        Paragraph::new(transcript)
            .wrap(Wrap { trim: false })
            .scroll((offset, 0)),
        chunks[1],
    );
    if let Some(question) = &app.question {
        draw_question(frame, chunks[2], question);
        frame.render_widget(
            Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::Gray)),
            chunks[3],
        );
        // Questions own input; underlying pickers, palette and draft stay
        // intact but must not cover either the question or its conversation.
        return;
    }
    let composer_width = chunks[2].width.max(1);
    let composer_height = chunks[2].height.saturating_sub(1).max(1);
    let (composer_row, composer_column) =
        cursor_position(&app.composer.text[..app.composer.cursor], composer_width);
    let composer_scroll = composer_row.saturating_sub(composer_height - 1);
    frame.render_widget(
        Paragraph::new(app.composer.text.as_str())
            .wrap(Wrap { trim: false })
            .scroll((composer_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(" Enter send · Shift+Enter newline ")
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .style(Style::default()),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(format!("F1 shortcuts  │  {}", app.status))
            .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    draw_slash_palette(frame, chunks[2], &app.palette_context());
    if !app.show_sessions {
        frame.set_cursor_position((
            (chunks[2].x + composer_column).min(chunks[2].right().saturating_sub(1)),
            (chunks[2].y + 1 + composer_row.saturating_sub(composer_scroll))
                .min(chunks[2].bottom().saturating_sub(1)),
        ));
    }
    if app.show_sessions && super::recent::sidebar_area(area, app).is_none() {
        draw_sessions(frame, area, app);
    }
    if app.terminal_panel.terminal_picker {
        draw_terminal_picker(frame, area, &app.terminal_panel);
    }
    if let Some(approval) = &app.approval {
        draw_approval(frame, area, approval);
    }
}

pub(super) fn draw_sessions(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    super::recent::draw(frame, super::recent::drawer_area(area), app);
}

pub(super) fn draw_approval(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    approval: &ApprovalRequest,
) {
    let popup = centered(area, 70, 35);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!(
            "Action: {}\nTarget: {}\nRequest: {}\n\n{}\n\n[y] approve    [n/Esc] deny",
            display_safe(&approval.action),
            display_safe(&approval.target),
            approval.id,
            display_safe(&approval.reason)
        ))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(" Approval required ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        popup,
    );
}

pub(super) fn draw_shortcut_help(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let (context, detailed, compact) = if app.model_panel.model_picker {
        (
            "Models",
            "type: filter\n↑/↓: select\nEnter: switch\nTab: enter model ID manually\nCtrl+R: refresh models\nEsc: return",
            "type filter · ↑↓ select · Enter switch · Tab manual · Ctrl+R refresh",
        )
    } else if app.supervisor_panel.supervisor_mode.is_some() {
        (
            "Agents",
            "↑/↓: select\nEnter: inspect\nm: message\nf: follow-up\nc twice: cancel\nr: refresh\nPageUp/PageDown: progress\nEsc: return",
            "↑↓ select · Enter inspect · m message · f follow-up · c,c cancel · Esc",
        )
    } else if app.todo_panel.todo_mode.is_some() {
        (
            "Todos",
            "↑/↓: select\nEnter: inspect\nn: new\ne: edit\nt: next status\nb/u: block/unblock\na: assign\nd: dependencies\np/o/v: progress/note/evidence\nJ/K: reorder\nx: archive\nr: reload\nEsc: return",
            "↑↓ select · Enter inspect · n new · e edit · t status · Esc",
        )
    } else if app.terminal_panel.terminal_picker {
        (
            "Terminals",
            "↑/↓: select\nEnter: attach\nr: refresh\nEsc: return\n\nWhile attached, Ctrl+T or Ctrl+] detaches without stopping the process.",
            "↑↓ select · Enter attach · r refresh · Esc return",
        )
    } else if app.show_sessions {
        (
            "Sessions",
            "↑/↓: select\nEnter: open\nEsc: return",
            "↑↓ select · Enter open · Esc return",
        )
    } else {
        (
            "Conversation",
            "Enter: send or steer active run\nShift+Enter: newline\nUp/Down: message history\nPageUp/PageDown: scroll\nEsc: cancel active work\nCtrl+D: todos\nCtrl+A: agents\nCtrl+M: models\nCtrl+P: policy\nCtrl+T: terminals\nCtrl+L: activity\nCtrl+O: tool details\nCtrl+S: sessions\nCtrl+N: new session\nCtrl+V: voyage drafts\nCtrl+B: branch\nCtrl+K: compact\nCtrl+E: export\nCtrl+C: cancel or quit\nCtrl+Q: quit",
            "^D todos · ^A agents · ^M models · ^T terminals · ^L activity · ^S sessions · ^N new · ^B branch · ^K compact · ^E export",
        )
    };
    let content = if area.height < 18 { compact } else { detailed };
    let popup = centered(area, 82, if area.height < 18 { 55 } else { 82 });
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(content).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(format!(" {context} shortcuts · F1/Esc close "))
                .borders(Borders::ALL),
        ),
        popup,
    );
}
