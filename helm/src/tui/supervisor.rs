//! Agent supervision panel input, actions and rendering.

use super::{
    bridge::UiEvent,
    composer::{Composer, cursor_position},
    text::one_line,
};
use crate::supervision::{
    AgentEvent as SupervisionEvent, AgentEventKind as SupervisionEventKind, AgentId, AgentStatus,
    AgentSupervisor, AgentView,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct SupervisorPanel {
    pub(super) supervisor_mode: Option<SupervisorMode>,
    pub(super) agents: Vec<AgentView>,
    pub(super) selected_agent: usize,
    pub(super) inspected_events: Vec<SupervisionEvent>,
    pub(super) inspected_agent: Option<AgentView>,
    pub(super) history_status: Option<crate::subagent::HistoryStatus>,
    pub(super) replay_pending: bool,
    pub(super) live_skipped: u64,
    pub(super) adapter_skipped: u64,
    pub(super) history_notices: std::collections::BTreeSet<String>,
    pub(super) supervisor_input: Composer,
    pub(super) cancel_armed: Option<AgentId>,
    pub(super) supervisor_scroll: usize,
}

impl SupervisorPanel {
    pub(super) fn record_lag(&mut self, count: u64) {
        self.live_skipped = self.live_skipped.saturating_add(count);
    }
    pub(super) fn append_event(&mut self, event: SupervisionEvent) {
        if self
            .inspected_events
            .last()
            .is_none_or(|last| event.sequence > last.sequence)
        {
            self.inspected_events.push(event);
        }
        self.bound_events();
    }
    fn bound_events(&mut self) {
        if self.inspected_events.len() > 500 {
            let count = self.inspected_events.len() - 500;
            self.inspected_events.drain(..count);
            self.history_notices
                .insert("Inspector retains the latest 500 events; earlier events omitted.".into());
        }
    }
    pub(super) fn apply_inspection(&mut self, inspection: crate::supervision::AgentInspection) {
        if !matches!(self.supervisor_mode, Some(SupervisorMode::Inspect(id) | SupervisorMode::Message { target: id, .. }) if id == inspection.agent.id)
        {
            return;
        }
        self.adapter_skipped = self.adapter_skipped.max(inspection.transport_skipped);
        // A late response cannot move an already displayed replay backwards.
        if self
            .inspected_agent
            .as_ref()
            .is_some_and(|a| a.id == inspection.agent.id)
            && let (Some(old), Some(new)) = (&self.history_status, &inspection.history)
            && old.cursor.epoch == new.cursor.epoch
            && (old.cursor.sequence > new.cursor.sequence
                || self
                    .inspected_events
                    .last()
                    .is_some_and(|event| event.sequence > new.cursor.sequence))
        {
            return;
        }
        // Live delivery can precede the first replay. Keep its newer events
        // while establishing the initial history metadata for this agent.
        let initial_live = if self.history_status.is_none() {
            inspection.history.as_ref().map(|history| {
                self.inspected_events
                    .iter()
                    .filter(|event| {
                        event.agent_id == inspection.agent.id
                            && event.sequence > history.cursor.sequence
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };
        self.inspected_agent = Some(inspection.agent);
        self.history_status = inspection.history;
        self.inspected_events = inspection.events;
        if let Some(events) = initial_live {
            self.inspected_events.extend(events);
        }
        self.bound_events();
    }
}

fn safe_history_text(text: &str) -> String {
    text.chars().map(|c| match c { '\n' | '\t' => c, _ if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') => '�', _ => c }).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SupervisorMode {
    Tree,
    Inspect(AgentId),
    Message { target: AgentId, follow_up: bool },
}

pub(super) async fn handle_supervisor_key(
    key: KeyEvent,
    panel: &mut SupervisorPanel,
    status: &mut String,
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
) {
    let Some(mode) = panel.supervisor_mode else {
        return;
    };
    if let SupervisorMode::Message { target, follow_up } = mode {
        match key.code {
            KeyCode::Esc => {
                panel.supervisor_input = Composer::default();
                panel.supervisor_mode = Some(SupervisorMode::Inspect(target));
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                panel.supervisor_input.insert('\n')
            }
            KeyCode::Enter => {
                let message = panel.supervisor_input.take();
                if !message.trim().is_empty() {
                    request_supervisor_message(tx, supervisor, target, message, follow_up);
                    panel.supervisor_mode = Some(SupervisorMode::Inspect(target));
                }
            }
            KeyCode::Char(character) => panel.supervisor_input.insert(character),
            KeyCode::Backspace => panel.supervisor_input.backspace(),
            KeyCode::Delete => panel.supervisor_input.delete(),
            KeyCode::Home => panel.supervisor_input.line_start(),
            KeyCode::End => panel.supervisor_input.line_end(),
            KeyCode::Left if panel.supervisor_input.cursor > 0 => {
                panel.supervisor_input.cursor = panel.supervisor_input.text
                    [..panel.supervisor_input.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
            KeyCode::Right => {
                if let Some(character) = panel.supervisor_input.text
                    [panel.supervisor_input.cursor..]
                    .chars()
                    .next()
                {
                    panel.supervisor_input.cursor += character.len_utf8();
                }
            }
            _ => {}
        }
        return;
    }
    let selected = match mode {
        SupervisorMode::Inspect(id) => Some(id),
        SupervisorMode::Tree => panel.agents.get(panel.selected_agent).map(|agent| agent.id),
        SupervisorMode::Message { .. } => unreachable!(),
    };
    match key.code {
        KeyCode::Esc => match mode {
            SupervisorMode::Inspect(_) => {
                panel.supervisor_mode = Some(SupervisorMode::Tree);
                panel.supervisor_scroll = 0;
            }
            SupervisorMode::Tree => panel.supervisor_mode = None,
            SupervisorMode::Message { .. } => unreachable!(),
        },
        KeyCode::Up if mode == SupervisorMode::Tree => {
            panel.selected_agent = panel.selected_agent.saturating_sub(1);
        }
        KeyCode::Down if mode == SupervisorMode::Tree => {
            panel.selected_agent =
                (panel.selected_agent + 1).min(panel.agents.len().saturating_sub(1));
        }
        KeyCode::Enter if mode == SupervisorMode::Tree => {
            if let Some(id) = selected {
                panel.inspected_agent = None;
                panel.inspected_events.clear();
                panel.history_status = None;
                panel.replay_pending = true;
                panel.supervisor_mode = Some(SupervisorMode::Inspect(id));
                panel.supervisor_scroll = 0;
                request_supervisor_inspect(tx, supervisor, id, None);
            }
        }
        KeyCode::PageUp if matches!(mode, SupervisorMode::Inspect(_)) => {
            panel.supervisor_scroll = panel.supervisor_scroll.saturating_add(8);
        }
        KeyCode::PageDown if matches!(mode, SupervisorMode::Inspect(_)) => {
            panel.supervisor_scroll = panel.supervisor_scroll.saturating_sub(8);
        }
        KeyCode::Char('r') => {
            request_supervisor_tree(tx, supervisor.clone());
            if let SupervisorMode::Inspect(id) = mode {
                request_supervisor_inspect(tx, supervisor, id, None);
            }
        }
        KeyCode::Char('m') if selected.is_some() => {
            panel.supervisor_input = Composer::default();
            panel.supervisor_mode = Some(SupervisorMode::Message {
                target: selected.unwrap(),
                follow_up: false,
            });
        }
        KeyCode::Char('f') if selected.is_some() => {
            panel.supervisor_input = Composer::default();
            panel.supervisor_mode = Some(SupervisorMode::Message {
                target: selected.unwrap(),
                follow_up: true,
            });
        }
        KeyCode::Char('c') if selected.is_some() => {
            let id = selected.unwrap();
            if panel.cancel_armed == Some(id) {
                request_supervisor_cancel(tx, supervisor, id);
                panel.cancel_armed = None;
            } else if panel
                .agents
                .iter()
                .any(|agent| agent.id == id && agent.status.is_terminal())
            {
                *status = "Completed agents cannot be cancelled".into();
            } else {
                panel.cancel_armed = Some(id);
                *status = "Press c again to cancel this agent; Esc returns".into();
            }
        }
        _ => panel.cancel_armed = None,
    }
}

pub(super) fn request_supervisor_tree(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = supervisor.tree().await.map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorTree(result));
    });
}

pub(super) fn request_supervisor_inspect(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
    after: Option<crate::subagent::HistoryCursor>,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            supervisor.replay(id, after),
        )
        .await
        {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(_) => Err("Supervisor history replay timed out".into()),
        };
        let _ = tx.send(UiEvent::SupervisorInspect(id, result));
    });
}

pub(super) fn request_supervisor_message(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
    message: String,
    follow_up: bool,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = if follow_up {
            supervisor
                .follow_up(id, message)
                .await
                .map(|child| format!("Follow-up queued as {child}"))
        } else {
            supervisor
                .send_message(id, message)
                .await
                .map(|()| "Message queued".into())
        }
        .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorAction(result));
    });
}

pub(super) fn request_supervisor_cancel(
    tx: &mpsc::UnboundedSender<UiEvent>,
    supervisor: Arc<dyn AgentSupervisor>,
    id: AgentId,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = supervisor
            .cancel(id)
            .await
            .map(|()| "Cancellation requested".into())
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::SupervisorAction(result));
    });
}

pub(super) fn flatten_agent_tree(agents: Vec<AgentView>) -> Vec<AgentView> {
    pub(super) fn visit(
        id: AgentId,
        all: &[AgentView],
        visited: &mut std::collections::HashSet<AgentId>,
        output: &mut Vec<AgentView>,
    ) {
        if !visited.insert(id) {
            return;
        }
        if let Some(agent) = all.iter().find(|agent| agent.id == id) {
            output.push(agent.clone());
            for child in all.iter().filter(|child| child.parent == Some(id)) {
                visit(child.id, all, visited, output);
            }
        }
    }
    let mut output = Vec::with_capacity(agents.len());
    let mut visited = std::collections::HashSet::new();
    for root in agents.iter().filter(|agent| {
        agent.parent.is_none()
            || !agents
                .iter()
                .any(|candidate| Some(candidate.id) == agent.parent)
    }) {
        visit(root.id, &agents, &mut visited, &mut output);
    }
    for agent in &agents {
        visit(agent.id, &agents, &mut visited, &mut output);
    }
    output
}

pub(super) fn supervisor_counts(agents: &[AgentView]) -> (usize, usize) {
    let active = agents
        .iter()
        .filter(|agent| !agent.status.is_terminal())
        .count();
    (active, agents.len().saturating_sub(active))
}

pub(super) fn supervisor_summary(agents: &[AgentView]) -> String {
    let (active, retained) = supervisor_counts(agents);
    let noun = if active == 1 { "agent" } else { "agents" };
    format!("Supervising {active} active {noun} · {retained} retained")
}

fn history_messages(panel: &SupervisorPanel) -> Vec<String> {
    let mut notices = panel.history_notices.iter().cloned().collect::<Vec<_>>();
    if let Some(history) = &panel.history_status {
        notices.push(format!(
            "History {} · retained {}–{} · {}",
            &history.cursor.epoch.to_string()[..8],
            history.first_sequence.unwrap_or(0),
            history.cursor.sequence,
            if history.durable {
                "saved"
            } else {
                "not durable"
            }
        ));
        if history.evicted_through > 0 {
            notices.push(format!("Evicted through #{}", history.evicted_through));
        }
        if history.cursor_gap {
            notices.push("Cursor gap: showing retained history".into());
        }
        for notice in &history.notices {
            notices.push(match notice {
            crate::subagent::HistoryNotice::Missing => "History file was missing; earlier events unavailable",
            crate::subagent::HistoryNotice::Corrupt => "History unreadable/corrupt; new epoch",
            crate::subagent::HistoryNotice::Restarted => "Restart boundary: in-flight events may be absent; final record is authoritative",
            crate::subagent::HistoryNotice::WriteFailed => "History write failed; some events may not survive restart",
        }.into());
        }
    } else if panel.inspected_agent.is_some() {
        notices.push("Adapter does not provide durable history status".into());
    }
    if panel.live_skipped > 0 {
        notices.push(format!(
            "Live UI delivery skipped {} events; replay contains only retained history",
            panel.live_skipped
        ));
    }
    if panel.adapter_skipped > 0 {
        notices.push(format!(
            "Runtime delivery skipped {} events; replay contains only retained history",
            panel.adapter_skipped
        ));
    }
    notices
}

pub(super) fn draw_supervisor(frame: &mut ratatui::Frame<'_>, area: Rect, panel: &SupervisorPanel) {
    let (active, retained) = supervisor_counts(&panel.agents);
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(
            if panel.history_notices.is_empty()
                && panel.history_status.is_none()
                && panel.live_skipped == 0
                && panel.adapter_skipped == 0
                && panel.inspected_agent.is_none()
            {
                0
            } else {
                3
            },
        ),
        Constraint::Min(4),
        Constraint::Length(
            if matches!(panel.supervisor_mode, Some(SupervisorMode::Message { .. })) {
                5
            } else {
                0
            },
        ),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM AGENTS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {active} active · {retained} retained")),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    let warning = !panel.history_notices.is_empty()
        || panel.live_skipped > 0
        || panel.adapter_skipped > 0
        || panel.history_status.as_ref().is_none_or(|h| {
            !h.durable || h.cursor_gap || h.evicted_through > 0 || !h.notices.is_empty()
        });
    let label = if warning {
        "History warning · PgUp details"
    } else {
        "History saved · PgUp details"
    };
    frame.render_widget(
        Paragraph::new(label)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Yellow)),
        chunks[1],
    );
    match panel.supervisor_mode {
        Some(SupervisorMode::Tree) => draw_agent_tree(frame, chunks[2], panel),
        Some(SupervisorMode::Inspect(id)) | Some(SupervisorMode::Message { target: id, .. }) => {
            draw_agent_inspection(frame, chunks[2], panel, id)
        }
        None => {}
    }
    if let Some(SupervisorMode::Message { follow_up, .. }) = panel.supervisor_mode {
        frame.render_widget(
            Paragraph::new(panel.supervisor_input.text.as_str())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(if follow_up {
                            " Follow-up · Enter send · Shift+Enter newline · Esc return "
                        } else {
                            " Message · Enter send · Shift+Enter newline · Esc return "
                        })
                        .borders(Borders::ALL),
                ),
            chunks[3],
        );
        let (row, column) = cursor_position(
            &panel.supervisor_input.text[..panel.supervisor_input.cursor],
            chunks[3].width.saturating_sub(2),
        );
        frame.set_cursor_position((
            (chunks[3].x + 1 + column).min(chunks[3].right().saturating_sub(2)),
            (chunks[3].y + 1 + row).min(chunks[3].bottom().saturating_sub(2)),
        ));
    }
    let help = match (panel.supervisor_mode, area.width < 60) {
        (Some(SupervisorMode::Tree), true) => "↑↓ Enter · Esc return",
        (Some(SupervisorMode::Inspect(_)), true) => "PgUp/PgDn · m/f send · Esc tree",
        (Some(SupervisorMode::Message { .. }), true) => "Enter send · Esc return",
        (Some(SupervisorMode::Tree), false) => {
            "↑↓ select · Enter inspect · m message · f follow-up · c,c cancel · r refresh · Esc return"
        }
        (Some(SupervisorMode::Inspect(_)), false) => {
            "m message · f follow-up · c,c cancel · PgUp/PgDn progress · r refresh · Esc tree"
        }
        (Some(SupervisorMode::Message { .. }), false) => {
            "Direct supervisor message; content is sent only to the selected agent"
        }
        (None, _) => "",
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[4],
    );
}

pub(super) fn draw_agent_tree(frame: &mut ratatui::Frame<'_>, area: Rect, panel: &SupervisorPanel) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = panel
        .selected_agent
        .saturating_sub(visible.saturating_sub(1))
        .min(panel.agents.len().saturating_sub(visible));
    let items: Vec<_> = panel
        .agents
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, agent)| {
            let depth = agent_depth(agent, &panel.agents).min(8);
            let selected = if index == panel.selected_agent {
                "▶"
            } else {
                " "
            };
            let progress = agent.recent_progress.last().map_or("", String::as_str);
            ListItem::new(format!(
                "{selected} {}{} [{}] {} · {} · {}",
                "  ".repeat(depth),
                short_id(agent.id),
                status_label(&agent.status),
                elapsed_label(agent.elapsed),
                one_line(&safe_history_text(&agent.task), 60),
                one_line(&safe_history_text(progress), 50)
            ))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new("No supervised agents")]
    } else {
        items
    };
    frame.render_widget(
        List::new(items).block(Block::default().title(" Agent tree ").borders(Borders::ALL)),
        area,
    );
}

pub(super) fn draw_agent_inspection(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    panel: &SupervisorPanel,
    id: AgentId,
) {
    let Some(agent) = panel
        .inspected_agent
        .as_ref()
        .filter(|a| a.id == id)
        .or_else(|| panel.agents.iter().find(|agent| agent.id == id))
    else {
        frame.render_widget(
            Paragraph::new("Agent is no longer present; Esc returns to the tree.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::raw(format!("Task: {}", agent.task)),
        Line::raw(format!(
            "State: {} · elapsed {}",
            status_label(&agent.status),
            elapsed_label(agent.elapsed)
        )),
        Line::raw(format!(
            "Parent: {}",
            agent
                .parent
                .map_or_else(|| "root".into(), |id| id.to_string())
        )),
        Line::raw(format!(
            "Worktree: {}",
            agent
                .worktree
                .as_ref()
                .map_or_else(|| "—".into(), |path| path.display().to_string())
        )),
        Line::raw(""),
        Line::styled(
            "Recent progress",
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    lines.extend(
        agent
            .recent_progress
            .iter()
            .map(|progress| Line::raw(format!("• {progress}"))),
    );
    lines.push(Line::raw("History details"));
    for notice in history_messages(panel) {
        lines.push(Line::raw(notice));
    }
    lines.extend(
        panel
            .inspected_events
            .iter()
            .filter(|event| event.agent_id == id)
            .map(|event| {
                Line::raw(format!(
                    "{} #{} {}",
                    event.timestamp.format("%H:%M:%S"),
                    event.sequence,
                    supervision_event_label(&event.kind)
                ))
            }),
    );
    if let Some(result) = &agent.result {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Result",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(result.clone()));
    }
    if let Some(error) = &agent.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        ));
    }
    // Wrap by grapheme/display width before slicing: wrapped long Unicode lines
    // must not hide the top of the inspector or overflow a u16 scroll offset.
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let width = usize::from(area.width.saturating_sub(2).max(1));
    let mut wrapped = Vec::new();
    for line in lines {
        let safe = safe_history_text(&line.to_string()).replace('\t', "    ");
        for logical in safe.split('\n') {
            let mut current = String::new();
            let mut used = 0;
            for cluster in logical.graphemes(true) {
                let cells = cluster.width();
                if used + cells > width && !current.is_empty() {
                    wrapped.push(Line::raw(std::mem::take(&mut current)));
                    used = 0;
                }
                current.push_str(cluster);
                used += cells;
            }
            wrapped.push(Line::raw(current));
        }
    }
    let height = area.height.saturating_sub(2) as usize;
    let bottom = wrapped.len().saturating_sub(height);
    let visible = wrapped
        .into_iter()
        .skip(bottom.saturating_sub(panel.supervisor_scroll))
        .take(height)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible).block(
            Block::default()
                .title(format!(" Agent {} ", short_id(id)))
                .borders(Borders::ALL),
        ),
        area,
    );
}

pub(super) fn agent_depth(agent: &AgentView, agents: &[AgentView]) -> usize {
    let mut parent = agent.parent;
    let mut depth = 0;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = parent {
        if !seen.insert(id) {
            break;
        }
        depth += 1;
        parent = agents
            .iter()
            .find(|agent| agent.id == id)
            .and_then(|agent| agent.parent);
    }
    depth
}

pub(super) fn short_id(id: AgentId) -> String {
    id.to_string().chars().take(8).collect()
}
pub(super) fn status_label(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Queued => "queued",
        AgentStatus::Running => "running",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Completed => "completed",
        AgentStatus::Failed => "failed",
        AgentStatus::TimedOut => "timed out",
        AgentStatus::Interrupted => "interrupted",
        AgentStatus::Cancelled => "cancelled",
    }
}
pub(super) fn elapsed_label(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
pub(super) fn supervision_event_label(kind: &SupervisionEventKind) -> String {
    match kind {
        SupervisionEventKind::HistoryGap { skipped } => {
            format!("live delivery skipped {skipped} events")
        }
        SupervisionEventKind::Queued => "queued".into(),
        SupervisionEventKind::Started => "started".into(),
        SupervisionEventKind::Progress { text } => text.clone(),
        SupervisionEventKind::MessageQueued => "message queued".into(),
        SupervisionEventKind::FollowUpQueued { child } => child.map_or_else(
            || "follow-up queued".into(),
            |id| format!("follow-up queued as {}", short_id(id)),
        ),
        SupervisionEventKind::Completed { result } => {
            format!("completed: {}", one_line(result, 120))
        }
        SupervisionEventKind::Failed { error } => format!("failed: {}", one_line(error, 120)),
        SupervisionEventKind::TimedOut { error } => {
            format!("timed out: {}", one_line(error, 120))
        }
        SupervisionEventKind::Interrupted { reason } => {
            format!("interrupted: {}", one_line(reason, 120))
        }
        SupervisionEventKind::Cancelled => "cancelled".into(),
    }
}
