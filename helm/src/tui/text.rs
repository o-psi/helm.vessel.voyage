//! Small terminal-safe text and layout helpers.

use ratatui::layout::{Constraint, Layout, Rect};

pub(super) fn display_safe(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\t' => character,
            _ if character.is_control() => '�',
            _ => character,
        })
        .collect()
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height) / 2),
        Constraint::Percentage(height),
        Constraint::Percentage((100 - height) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - width) / 2),
        Constraint::Percentage(width),
        Constraint::Percentage((100 - width) / 2),
    ])
    .split(vertical[1])[1]
}

pub(super) fn one_line(text: &str, max: usize) -> String {
    let text = text.lines().next().unwrap_or_default();
    let mut output: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        output.push('…');
    }
    output
}

pub(super) fn compact_line(text: &str, max: usize) -> String {
    let safe = display_safe(text);
    let compact = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = compact.chars().take(max).collect::<String>();
    if compact.chars().count() > max {
        output.push('…');
    }
    output
}
