//! Direct PTY input, screen rendering and terminal inventory.

use super::text::centered;
use crate::terminal::{
    InteractiveTerminals, TerminalColor, TerminalEvent, TerminalId, TerminalSnapshot,
    TerminalSummary,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

#[derive(Default)]
pub(super) struct TerminalPanel {
    pub(super) terminals: Vec<TerminalSummary>,
    pub(super) terminal_picker: bool,
    pub(super) selected_terminal: usize,
    pub(super) attached_terminal: Option<TerminalId>,
    pub(super) terminal_snapshot: Option<TerminalSnapshot>,
}

pub(super) async fn handle_attached_key(
    id: TerminalId,
    key: KeyEvent,
    panel: &mut TerminalPanel,
    status: &mut String,
    terminals: &dyn InteractiveTerminals,
) {
    if is_terminal_detach_key(key) {
        panel.attached_terminal = None;
        panel.terminal_snapshot = None;
        *status = "Detached; terminal is still running".into();
    } else if let Some(bytes) = encode_terminal_key(key)
        && let Err(error) = terminals.write(id, bytes).await
    {
        *status = format!("Terminal input failed: {error}");
    }
}

pub(super) fn is_terminal_detach_key(key: KeyEvent) -> bool {
    (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('t' | ']')))
        || key.code == KeyCode::Char('\u{1d}')
}

pub(super) fn draw_attached_terminal(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    panel: &TerminalPanel,
) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);
    let (title, state) = panel
        .terminal_snapshot
        .as_ref()
        .map(|snapshot| (snapshot.title.as_str(), format!("{:?}", snapshot.state)))
        .unwrap_or(("loading", "unknown".into()));
    frame.render_widget(
        Paragraph::new(format!(
            " HELM TERMINAL · {title} · {state}{} · Ctrl+T detach (process keeps running)",
            panel
                .terminal_snapshot
                .as_ref()
                .filter(|snapshot| snapshot.dropped_unread_bytes > 0)
                .map(|snapshot| format!(
                    " · {} agent-unread bytes evicted",
                    snapshot.dropped_unread_bytes
                ))
                .unwrap_or_default()
        ))
        .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        chunks[0],
    );
    if let Some(snapshot) = &panel.terminal_snapshot {
        let screen = Text::from(
            snapshot
                .cells
                .iter()
                .map(|row| {
                    Line::from(
                        row.iter()
                            .map(|cell| {
                                let mut style = Style::default()
                                    .fg(terminal_color(cell.foreground))
                                    .bg(terminal_color(cell.background));
                                if cell.bold {
                                    style = style.add_modifier(Modifier::BOLD);
                                }
                                if cell.dim {
                                    style = style.add_modifier(Modifier::DIM);
                                }
                                if cell.italic {
                                    style = style.add_modifier(Modifier::ITALIC);
                                }
                                if cell.underlined {
                                    style = style.add_modifier(Modifier::UNDERLINED);
                                }
                                if cell.reversed {
                                    style = style.add_modifier(Modifier::REVERSED);
                                }
                                Span::styled(cell.text.clone(), style)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        frame.render_widget(Paragraph::new(screen), chunks[1]);
        if let Some((column, row)) = snapshot.cursor {
            frame.set_cursor_position((
                chunks[1]
                    .x
                    .saturating_add(column)
                    .min(chunks[1].right().saturating_sub(1)),
                chunks[1]
                    .y
                    .saturating_add(row)
                    .min(chunks[1].bottom().saturating_sub(1)),
            ));
        }
    } else {
        frame.render_widget(Paragraph::new("Waiting for terminal screen…"), chunks[1]);
    }
}

pub(super) fn terminal_color(color: TerminalColor) -> Color {
    match color {
        TerminalColor::Default => Color::Reset,
        TerminalColor::Indexed(index) => Color::Indexed(index),
        TerminalColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

pub(super) fn draw_terminal_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    panel: &TerminalPanel,
) {
    let popup = centered(area, 80, 70);
    let items: Vec<_> = panel
        .terminals
        .iter()
        .enumerate()
        .map(|(index, terminal)| {
            ListItem::new(format!(
                "{} {}  {}  {:?}",
                if index == panel.selected_terminal {
                    "▶"
                } else {
                    " "
                },
                terminal.id,
                terminal.title,
                terminal.state
            ))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(
            "No interactive terminals · r refresh · Esc close",
        )]
    } else {
        items
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Terminals · ↑↓ select · Enter attach · r refresh · Esc close ")
                .borders(Borders::ALL),
        ),
        popup,
    );
}

pub(super) async fn refresh_terminals(
    panel: &mut TerminalPanel,
    status: &mut String,
    terminals: &dyn InteractiveTerminals,
) {
    match terminals.list().await {
        Ok(list) => {
            panel.terminals = list;
            panel.selected_terminal = panel
                .selected_terminal
                .min(panel.terminals.len().saturating_sub(1));
        }
        Err(error) => *status = format!("Cannot list terminals: {error}"),
    }
}

pub(super) async fn handle_terminal_event(
    event: TerminalEvent,
    panel: &mut TerminalPanel,
    status: &mut String,
    terminals: &dyn InteractiveTerminals,
) {
    let id = match event {
        TerminalEvent::Changed(id) | TerminalEvent::Added(id) | TerminalEvent::Removed(id) => id,
    };
    refresh_terminals(panel, status, terminals).await;
    if panel.attached_terminal == Some(id) {
        match terminals.snapshot(id).await {
            Ok(snapshot) => panel.terminal_snapshot = Some(snapshot),
            Err(_) => {
                panel.attached_terminal = None;
                panel.terminal_snapshot = None;
                *status = "Attached terminal disappeared".into();
            }
        }
    }
}

pub(super) fn encode_terminal_key(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Char(character) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let upper = character.to_ascii_uppercase() as u32;
            if (64..=95).contains(&upper) {
                bytes.push((upper - 64) as u8);
            } else {
                return None;
            }
        }
        KeyCode::Char(character) => {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::F(number) if (1..=4).contains(&number) => {
            bytes.extend_from_slice(&[0x1b, b'O', b'P' + number - 1]);
        }
        KeyCode::F(number) if (5..=12).contains(&number) => {
            const CODES: [&[u8]; 8] = [b"15", b"17", b"18", b"19", b"20", b"21", b"23", b"24"];
            bytes.extend_from_slice(b"\x1b[");
            bytes.extend_from_slice(CODES[(number - 5) as usize]);
            bytes.push(b'~');
        }
        _ => return None,
    }
    Some(bytes)
}
