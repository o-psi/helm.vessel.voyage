use super::*;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    text::{Line, Text},
    widgets::Paragraph,
};

impl App {
    pub(in crate::process_client::ui) fn draw_actions(&self, frame: &mut Frame<'_>, area: Rect) {
        self.sidebar.menu_hits.borrow_mut().clear();
        self.sidebar.controls.borrow_mut().clear();
        self.sidebar.visible.set(None);
        self.sidebar.area.set(area);
        self.sidebar.scroll_bounds.set((0, 0));
        let Some(menu) = &self.sidebar.menu else {
            return;
        };
        if area.width < 20 || area.height < 10 {
            frame.render_widget(Paragraph::new("Enlarge the window to use Actions."), area);
            return;
        }
        self.sidebar
            .visible
            .set(Some((menu.target, menu.incarnation, menu.editor)));
        let block = super::super::right_panel::block("Voyage actions");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let body = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(5),
        );
        let footer = Rect::new(
            inner.x,
            body.bottom(),
            inner.width,
            inner.height - body.height,
        );
        let view = self.views.get(&menu.target);
        let title = view.map_or_else(
            || "Unavailable voyage".into(),
            |view| super::super::safe(&view.title()),
        );
        let mut lines =
            super::super::presentation::wrap(Text::raw(format!("{title}\n")), body.width).lines;
        if !menu.error.is_empty() {
            lines.extend(
                super::super::presentation::wrap(
                    Text::raw(format!("{}\n\n", super::super::safe(&menu.error))),
                    body.width,
                )
                .lines,
            );
        }
        let mut ranges = Vec::new();
        let mut editor_start = None;
        let access = matches!(
            menu.editor,
            Some(Action::Access | Action::ReadOnly | Action::Approval | Action::Unrestricted)
        );
        if access {
            self.draw_access(frame, menu, body);
        } else {
            if let Some(action) = menu.editor {
                let text = match action {
                    Action::Details => view.map_or_else(|| "Voyage no longer available".into(), |v| {
                        let state = v.snapshot.as_ref().map_or("Unavailable", super::super::presentation::voyage_state);
                        let cleanup = if v.process.state == voyage_protocol::process::ProcessState::Stopped { "Confirmed stopped" } else { v.snapshot.as_ref().map_or("Unknown", |s| if s.pending_cleanup_run.is_some() { "Unconfirmed run cleanup" } else { "No pending run cleanup" }) };
                        format!("Vessel: {}\nWorkspace: {}\nModel: {}\nState: {state}\nArchived: {}\nCleanup: {cleanup}\nPending command: {}\nVoyage: {}\n{}", self.route_label(menu.target.route), v.process.workspace.display(), v.snapshot.as_ref().map_or("Unavailable", |s| s.model.as_str()), v.archived(), v.pending.is_some(), menu.target.session, v.error.as_deref().map(super::super::presentation::notice).unwrap_or_default())
                    }),
                    Action::Rename => "Rename voyage\nEnter a new name:".into(),
                    Action::Branch => "Branch conversation\nOptional name for the new voyage:".into(),
                    Action::Clear => "Clear conversation\nRemove this voyage’s current messages and provider continuation. The voyage identity and prior run/receipt evidence remain. This is not forensic erasure. Export or branch first if you need a copy. Your unsent draft is preserved.\nType CLEAR to confirm:".into(),
                    Action::Compact => "Compact older messages\nRemove older conversation content with an omission marker, not a generated summary. The recent-message target preserves tool-call groups and may differ from the exact retained count. Export or branch first if needed. Your unsent draft is preserved.\nType KEEP followed by a number (1–100000), for example KEEP 128:".into(),
                    Action::Delete => format!("Delete permanently\nThis removes the selected voyage's conversation history.\nVoyage: {}\nType DELETE to confirm:", menu.target.session),
                    _ => String::new(),
                };
                lines.extend(
                    super::super::presentation::wrap(
                        Text::raw(super::super::safe(&text)),
                        body.width,
                    )
                    .lines,
                );
                if action != Action::Details {
                    lines.push(Line::default());
                    editor_start = Some(lines.len());
                    lines.extend(
                        super::super::presentation::wrap(Text::raw(&menu.text.text), body.width)
                            .lines,
                    );
                }
            } else {
                for (index, action) in menu.actions.iter().enumerate() {
                    if *action == Action::Delete {
                        lines.push(Line::default());
                    }
                    let reason = self.action_reason(menu, *action);
                    let label = format!(
                        "{}{}{}",
                        if index == menu.selected { "> " } else { "  " },
                        action.label(),
                        reason
                            .map(|reason| format!(" — {reason}"))
                            .unwrap_or_default()
                    );
                    let start = lines.len();
                    lines.extend(
                        super::super::presentation::wrap(Text::raw(label), body.width).lines,
                    );
                    ranges.push((start, lines.len(), index, reason.is_none()));
                }
            }
            let max = lines
                .len()
                .saturating_sub(body.height as usize)
                .min(u16::MAX as usize) as u16;
            let mut scroll = menu.scroll.get().min(max);
            if menu.follow_selection.replace(false) {
                let focus_row = if let Some(start) = editor_start {
                    start
                        + crate::composer::cursor_position(
                            &menu.text.text[..menu.text.cursor],
                            body.width,
                        )
                        .0 as usize
                } else {
                    ranges
                        .iter()
                        .find(|(_, _, index, _)| *index == menu.selected)
                        .map_or(0, |(start, _, _, _)| *start)
                };
                if focus_row < scroll as usize {
                    scroll = focus_row as u16;
                } else if focus_row >= scroll as usize + body.height as usize {
                    scroll = focus_row
                        .saturating_sub(body.height.saturating_sub(1) as usize)
                        .min(max as usize) as u16;
                }
            }
            menu.scroll.set(scroll);
            self.sidebar.scroll_bounds.set((scroll, max));
            for (start, end, index, enabled) in ranges {
                let top = start.max(scroll as usize);
                let bottom = end.min(scroll as usize + body.height as usize);
                if top < bottom {
                    let rect = Rect::new(
                        body.x,
                        body.y + (top - scroll as usize) as u16,
                        body.width,
                        (bottom - top) as u16,
                    );
                    let style = super::super::right_panel::control_style(
                        self.sidebar.pointer,
                        rect,
                        index == menu.selected,
                        enabled,
                    );
                    let style = if enabled && menu.actions[index] == Action::Delete {
                        style.patch(crate::theme::Role::Failed.style())
                    } else {
                        style
                    };
                    for line in &mut lines[top..bottom] {
                        *line = line.clone().style(style);
                    }
                    if enabled {
                        self.sidebar.menu_hits.borrow_mut().push((
                            rect,
                            menu.target,
                            menu.incarnation,
                            index,
                        ));
                    }
                }
            }
            if let Some(start) = editor_start {
                let (row, col) = crate::composer::cursor_position(
                    &menu.text.text[..menu.text.cursor],
                    body.width,
                );
                let row = start + row as usize;
                if row >= scroll as usize && row < scroll as usize + body.height as usize {
                    frame.set_cursor_position((
                        body.x + col.min(body.width.saturating_sub(1)),
                        body.y + (row - scroll as usize) as u16,
                    ));
                }
            }
            frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body);
        }
        let enabled =
            self.sidebar.visible.get() == Some((menu.target, menu.incarnation, menu.editor));
        let action = menu
            .editor
            .unwrap_or(menu.actions[menu.selected.min(menu.actions.len() - 1)]);
        let confirm_enabled = enabled
            && (!access
                || menu.editor == Some(Action::Access)
                || self.sidebar.scroll_bounds.get().0 == self.sidebar.scroll_bounds.get().1)
            && self
                .action_reason(menu, if access { Action::Access } else { action })
                .is_none();
        let editing = matches!(
            menu.editor,
            Some(
                Action::Rename | Action::Branch | Action::Delete | Action::Clear | Action::Compact
            )
        );
        let (scroll, max) = self.sidebar.scroll_bounds.get();
        let mut controls = vec![
            (
                if menu.editor == Some(Action::Access) {
                    "[Review]"
                } else if menu.editor == Some(Action::Details) {
                    "[Close]"
                } else if menu.editor.is_some() {
                    "[Confirm]"
                } else {
                    "[Open]"
                },
                KeyCode::Enter,
                confirm_enabled,
            ),
            (
                if menu.editor.is_some() {
                    "[Back]"
                } else {
                    "[Close]"
                },
                KeyCode::Esc,
                true,
            ),
            ("[Up]", KeyCode::PageUp, scroll > 0),
            ("[Down]", KeyCode::PageDown, scroll < max),
        ];
        if editing {
            controls.push(("[Paste]", KeyCode::Insert, confirm_enabled));
            controls.push(("[Clear]", KeyCode::Null, confirm_enabled));
        }
        let hint = if menu.error.is_empty() {
            "Click · Wheel scroll · Esc back"
        } else {
            "Action unavailable · Read details"
        };
        frame.render_widget(
            Paragraph::new(super::super::safe(hint)).style(if menu.error.is_empty() {
                crate::theme::Role::Muted.style()
            } else {
                crate::theme::Role::AwaitingInput.style()
            }),
            Rect::new(footer.x, footer.y, footer.width, 1),
        );
        let hits = super::super::right_panel::buttons(
            frame,
            Rect::new(
                footer.x,
                footer.y + 1,
                footer.width,
                footer.height.saturating_sub(1),
            ),
            self.sidebar.pointer,
            &controls,
        );
        self.sidebar.controls.borrow_mut().extend(hits);
    }
}
