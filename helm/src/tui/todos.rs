//! Todo panel input, store actions and rendering.

use super::{
    bridge::UiEvent,
    composer::{Composer, cursor_position},
    text::one_line,
};
use crate::todo::{EntryKind, NewTodo, Priority, TodoId, TodoItem, TodoStatus, TodoStore};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct TodoPanel {
    pub(super) todo_mode: Option<TodoMode>,
    pub(super) todos: Vec<TodoItem>,
    pub(super) todo_revision: u64,
    pub(super) selected_todo: usize,
    pub(super) todo_scroll: u16,
    pub(super) todo_input: Composer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TodoMode {
    List,
    Inspect(TodoId),
    Input {
        target: Option<TodoId>,
        action: TodoInput,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TodoInput {
    Add,
    Edit,
    Block,
    Assign,
    Dependencies,
    Progress,
    Note,
    Evidence,
}

pub(super) async fn handle_todo_key(
    key: KeyEvent,
    panel: &mut TodoPanel,
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
) {
    let Some(mode) = panel.todo_mode else { return };
    if let TodoMode::Input { target, action } = mode {
        match key.code {
            KeyCode::Esc => {
                panel.todo_input = Composer::default();
                panel.todo_mode =
                    target.map_or(Some(TodoMode::List), |id| Some(TodoMode::Inspect(id)));
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                panel.todo_input.insert('\n')
            }
            KeyCode::Enter => {
                let value = panel.todo_input.take();
                if !value.trim().is_empty()
                    || matches!(
                        action,
                        TodoInput::Block | TodoInput::Assign | TodoInput::Dependencies
                    )
                {
                    request_todo_input(tx, store, target, action, value);
                    panel.todo_mode =
                        target.map_or(Some(TodoMode::List), |id| Some(TodoMode::Inspect(id)));
                }
            }
            KeyCode::Char(character) => panel.todo_input.insert(character),
            KeyCode::Backspace => panel.todo_input.backspace(),
            KeyCode::Delete => panel.todo_input.delete(),
            KeyCode::Home => panel.todo_input.line_start(),
            KeyCode::End => panel.todo_input.line_end(),
            KeyCode::Left if panel.todo_input.cursor > 0 => {
                panel.todo_input.cursor = panel.todo_input.text[..panel.todo_input.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index);
            }
            KeyCode::Right => {
                if let Some(character) = panel.todo_input.text[panel.todo_input.cursor..]
                    .chars()
                    .next()
                {
                    panel.todo_input.cursor += character.len_utf8();
                }
            }
            _ => {}
        }
        return;
    }
    let selected = match mode {
        TodoMode::List => panel.todos.get(panel.selected_todo).map(|item| item.id),
        TodoMode::Inspect(id) => Some(id),
        TodoMode::Input { .. } => unreachable!(),
    };
    match key.code {
        KeyCode::Esc => match mode {
            TodoMode::Inspect(_) => panel.todo_mode = Some(TodoMode::List),
            TodoMode::List => panel.todo_mode = None,
            TodoMode::Input { .. } => unreachable!(),
        },
        KeyCode::Up if mode == TodoMode::List => {
            panel.selected_todo = panel.selected_todo.saturating_sub(1)
        }
        KeyCode::Down if mode == TodoMode::List => {
            panel.selected_todo =
                (panel.selected_todo + 1).min(panel.todos.len().saturating_sub(1));
        }
        KeyCode::Enter if mode == TodoMode::List => {
            if let Some(id) = selected {
                panel.todo_mode = Some(TodoMode::Inspect(id));
            }
        }
        KeyCode::PageUp if matches!(mode, TodoMode::Inspect(_)) => {
            panel.todo_scroll = panel.todo_scroll.saturating_add(8)
        }
        KeyCode::PageDown if matches!(mode, TodoMode::Inspect(_)) => {
            panel.todo_scroll = panel.todo_scroll.saturating_sub(8)
        }
        KeyCode::Char('r') => request_todo_snapshot(tx, store),
        KeyCode::Char('n') => open_todo_input(panel, None, TodoInput::Add, ""),
        KeyCode::Char('e') if selected.is_some() => {
            let item = panel.todos.iter().find(|item| Some(item.id) == selected);
            let initial = item.map_or(String::new(), |item| {
                format!("{}\n{}", item.title, item.description)
            });
            open_todo_input(panel, selected, TodoInput::Edit, &initial);
        }
        KeyCode::Char('b') if selected.is_some() => {
            open_todo_input(panel, selected, TodoInput::Block, "")
        }
        KeyCode::Char('a') if selected.is_some() => {
            let initial = panel
                .todos
                .iter()
                .find(|item| Some(item.id) == selected)
                .map(|item| {
                    item.assignees
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            open_todo_input(panel, selected, TodoInput::Assign, &initial);
        }
        KeyCode::Char('d') if selected.is_some() => {
            let initial = panel
                .todos
                .iter()
                .find(|item| Some(item.id) == selected)
                .map(|item| {
                    item.dependencies
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            open_todo_input(panel, selected, TodoInput::Dependencies, &initial);
        }
        KeyCode::Char('p') if selected.is_some() => {
            open_todo_input(panel, selected, TodoInput::Progress, "")
        }
        KeyCode::Char('o') if selected.is_some() => {
            open_todo_input(panel, selected, TodoInput::Note, "")
        }
        KeyCode::Char('v') if selected.is_some() => {
            open_todo_input(panel, selected, TodoInput::Evidence, "")
        }
        KeyCode::Char('u') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store
                    .set_blockers(id, Vec::new())
                    .await
                    .map(|_| "Blockers cleared".into())
            })
        }
        KeyCode::Char('t') if let Some(id) = selected => {
            if let Some(item) = panel.todos.iter().find(|item| item.id == id) {
                let status = match item.status {
                    TodoStatus::Pending | TodoStatus::Blocked => TodoStatus::InProgress,
                    TodoStatus::InProgress => TodoStatus::Completed,
                    TodoStatus::Completed | TodoStatus::Cancelled => TodoStatus::Pending,
                };
                request_todo_action(tx, store.clone(), async move {
                    store
                        .set_status(id, status)
                        .await
                        .map(|_| format!("Todo is {}", todo_status_label(status)))
                });
            }
        }
        KeyCode::Char('c') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store
                    .set_status(id, TodoStatus::Cancelled)
                    .await
                    .map(|_| "Todo cancelled".into())
            })
        }
        KeyCode::Char('K') if let Some(id) = selected => reorder_todo(panel, tx, store, id, -1),
        KeyCode::Char('J') if let Some(id) = selected => reorder_todo(panel, tx, store, id, 1),
        KeyCode::Char('x') if let Some(id) = selected => {
            request_todo_action(tx, store.clone(), async move {
                store.archive(id).await.map(|_| "Todo archived".into())
            })
        }
        _ => {}
    }
}

pub(super) fn open_todo_input(
    panel: &mut TodoPanel,
    target: Option<TodoId>,
    action: TodoInput,
    initial: &str,
) {
    panel.todo_input = Composer::default();
    panel.todo_input.insert_str(initial);
    panel.todo_mode = Some(TodoMode::Input { target, action });
}

pub(super) fn request_todo_snapshot(tx: &mpsc::UnboundedSender<UiEvent>, store: Arc<TodoStore>) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(UiEvent::TodoSnapshot(
            store.snapshot().await.map_err(|error| error.to_string()),
        ));
    });
}

pub(super) fn request_todo_action<F>(
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
    future: F,
) where
    F: std::future::Future<Output = anyhow::Result<String>> + Send + 'static,
{
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = future.await.map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::TodoAction(result));
        let _ = tx.send(UiEvent::TodoSnapshot(
            store.snapshot().await.map_err(|error| error.to_string()),
        ));
    });
}

pub(super) fn request_todo_input(
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
    target: Option<TodoId>,
    action: TodoInput,
    value: String,
) {
    let operation_store = store.clone();
    request_todo_action(tx, store, async move {
        match (action, target) {
            (TodoInput::Add, None) => {
                let (title, description) = value.split_once('\n').unwrap_or((&value, ""));
                operation_store
                    .create(NewTodo {
                        title: title.to_owned(),
                        description: description.to_owned(),
                        priority: Priority::Normal,
                        order: None,
                        assignees: BTreeSet::new(),
                    })
                    .await
                    .map(|item| format!("Added {}", item.title))
            }
            (TodoInput::Edit, Some(id)) => {
                let (title, description) = value.split_once('\n').unwrap_or((&value, ""));
                operation_store
                    .edit(
                        id,
                        Some(title.to_owned()),
                        Some(description.to_owned()),
                        None,
                    )
                    .await
                    .map(|_| "Todo updated".into())
            }
            (TodoInput::Block, Some(id)) => operation_store
                .set_blockers(id, split_values(&value))
                .await
                .map(|_| "Blockers updated".into()),
            (TodoInput::Assign, Some(id)) => operation_store
                .assign(id, split_values(&value).into_iter().collect())
                .await
                .map(|_| "Assignees updated".into()),
            (TodoInput::Dependencies, Some(id)) => {
                let desired = parse_todo_ids(&value)?;
                let snapshot = operation_store.snapshot().await?;
                let current = snapshot
                    .items
                    .get(&id)
                    .ok_or_else(|| anyhow::anyhow!("unknown todo"))?
                    .dependencies
                    .clone();
                let add = desired.difference(&current).copied().collect();
                let remove = current.difference(&desired).copied().collect();
                operation_store.update_dependencies(id, add, remove).await?;
                Ok("Dependencies updated".into())
            }
            (TodoInput::Progress, Some(id)) => operation_store
                .append_note(id, EntryKind::Progress, value, Some("user".into()))
                .await
                .map(|_| "Progress added".into()),
            (TodoInput::Note, Some(id)) => operation_store
                .append_note(id, EntryKind::Note, value, Some("user".into()))
                .await
                .map(|_| "Note added".into()),
            (TodoInput::Evidence, Some(id)) => operation_store
                .append_note(id, EntryKind::Evidence, value, Some("user".into()))
                .await
                .map(|_| "Evidence added".into()),
            _ => anyhow::bail!("invalid todo action"),
        }
    });
}

pub(super) fn split_values(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn parse_todo_ids(value: &str) -> anyhow::Result<BTreeSet<TodoId>> {
    split_values(value)
        .into_iter()
        .map(|value| {
            uuid::Uuid::parse_str(&value)
                .map(TodoId)
                .map_err(|_| anyhow::anyhow!("invalid todo UUID: {value}"))
        })
        .collect()
}

pub(super) fn reorder_todo(
    panel: &TodoPanel,
    tx: &mpsc::UnboundedSender<UiEvent>,
    store: Arc<TodoStore>,
    id: TodoId,
    direction: isize,
) {
    let Some(index) = panel.todos.iter().position(|item| item.id == id) else {
        return;
    };
    let other = index
        .saturating_add_signed(direction)
        .min(panel.todos.len().saturating_sub(1));
    if other == index {
        return;
    }
    let original_order = panel.todos[index].order;
    let other_id = panel.todos[other].id;
    let other_order = panel.todos[other].order;
    request_todo_action(tx, store.clone(), async move {
        store.reorder(id, other_order).await?;
        store.reorder(other_id, original_order).await?;
        Ok("Todo reordered".into())
    });
}

pub(super) fn draw_todos(frame: &mut ratatui::Frame<'_>, area: Rect, panel: &TodoPanel) {
    let input = matches!(panel.todo_mode, Some(TodoMode::Input { .. }));
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(if input { 5 } else { 0 }),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM TODOS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {} active · revision {}",
                panel.todos.len(),
                panel.todo_revision
            )),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    match panel.todo_mode {
        Some(TodoMode::List) => draw_todo_list(frame, chunks[1], panel),
        Some(TodoMode::Inspect(id))
        | Some(TodoMode::Input {
            target: Some(id), ..
        }) => draw_todo_inspect(frame, chunks[1], panel, id),
        Some(TodoMode::Input { target: None, .. }) => draw_todo_list(frame, chunks[1], panel),
        None => {}
    }
    if let Some(TodoMode::Input { action, .. }) = panel.todo_mode {
        let title = match action {
            TodoInput::Add => " Add · title ",
            TodoInput::Edit => " Edit · title then description on next line ",
            TodoInput::Block => " Blockers · comma/newline separated ",
            TodoInput::Assign => " Assignees · comma/newline separated ",
            TodoInput::Dependencies => " Prerequisite UUIDs · comma/newline separated ",
            TodoInput::Progress => " Progress update ",
            TodoInput::Note => " Note ",
            TodoInput::Evidence => " Evidence ",
        };
        frame.render_widget(
            Paragraph::new(panel.todo_input.text.as_str())
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(format!(
                            "{title}· Enter save · Shift+Enter newline · Esc return "
                        ))
                        .borders(Borders::ALL),
                ),
            chunks[2],
        );
        let (row, column) = cursor_position(
            &panel.todo_input.text[..panel.todo_input.cursor],
            chunks[2].width.saturating_sub(2).max(1),
        );
        frame.set_cursor_position((
            (chunks[2].x + 1 + column).min(chunks[2].right().saturating_sub(2)),
            (chunks[2].y + 1 + row).min(chunks[2].bottom().saturating_sub(2)),
        ));
    }
    let help = if area.width < 60 {
        "↑↓ Enter · n new · t next · Esc return"
    } else {
        "↑↓ · Enter inspect · n new · e edit · t next · b/u block · a assign · d deps · p progress · o note · v evidence · J/K reorder · c cancel · x archive · r reload · Esc"
    };
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
}

pub(super) fn draw_todo_list(frame: &mut ratatui::Frame<'_>, area: Rect, panel: &TodoPanel) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let start = panel
        .selected_todo
        .saturating_sub(visible.saturating_sub(1))
        .min(panel.todos.len().saturating_sub(visible));
    let items: Vec<_> = panel
        .todos
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, item)| {
            let marker = if index == panel.selected_todo {
                "▶"
            } else {
                " "
            };
            let blockers = if item.blockers.is_empty() {
                String::new()
            } else {
                format!(" · {} blocker(s)", item.blockers.len())
            };
            ListItem::new(format!(
                "{marker} [{}|{}] {}{}",
                todo_status_label(item.status),
                priority_label(item.priority),
                one_line(&item.title, 80),
                blockers
            ))
        })
        .collect();
    frame.render_widget(
        List::new(if items.is_empty() {
            vec![ListItem::new("No active todos · press n to add one")]
        } else {
            items
        })
        .block(
            Block::default()
                .title(" Workspace plan ")
                .borders(Borders::ALL),
        ),
        area,
    );
}

pub(super) fn draw_todo_inspect(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    panel: &TodoPanel,
    id: TodoId,
) {
    let Some(item) = panel.todos.iter().find(|item| item.id == id) else {
        frame.render_widget(
            Paragraph::new("Todo was archived or removed; Esc returns to the list.")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::raw(format!(
            "{} [{} · {}]",
            item.title,
            todo_status_label(item.status),
            priority_label(item.priority)
        )),
        Line::raw(format!("ID: {} · order {}", item.id.0, item.order)),
        Line::raw(format!(
            "Assignees: {}",
            if item.assignees.is_empty() {
                "—".into()
            } else {
                item.assignees
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        Line::raw(format!(
            "Dependencies: {}",
            if item.dependencies.is_empty() {
                "—".into()
            } else {
                item.dependencies
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )),
        Line::raw(""),
        Line::raw(item.description.clone()),
    ];
    for blocker in &item.blockers {
        lines.push(Line::styled(
            format!("BLOCKED: {blocker}"),
            Style::default().fg(Color::Red),
        ));
    }
    for (label, entries) in [
        ("Progress", &item.progress),
        ("Evidence", &item.evidence),
        ("Notes", &item.notes),
    ] {
        if !entries.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                label,
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        lines.extend(entries.iter().map(|entry| {
            Line::raw(format!(
                "{} {}{}",
                entry.at.format("%Y-%m-%d %H:%M"),
                entry
                    .author
                    .as_deref()
                    .map_or(String::new(), |author| format!("{author}: ")),
                entry.text
            ))
        }));
    }
    let bottom = lines
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((bottom.saturating_sub(panel.todo_scroll), 0))
            .block(
                Block::default()
                    .title(" Todo detail ")
                    .borders(Borders::ALL),
            ),
        area,
    );
}

pub(super) fn todo_status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "pending",
        TodoStatus::InProgress => "in progress",
        TodoStatus::Blocked => "blocked",
        TodoStatus::Completed => "completed",
        TodoStatus::Cancelled => "cancelled",
    }
}
pub(super) fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::Low => "low",
        Priority::Normal => "normal",
        Priority::High => "high",
        Priority::Critical => "critical",
    }
}
