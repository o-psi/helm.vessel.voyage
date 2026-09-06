//! Complete, scrollable attended publication decisions. Paste never approves.
use super::ApprovalRequest;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthChar;

pub(super) fn exact(request: &ApprovalRequest) -> bool {
    matches!(
        request.action.as_str(),
        "github.publish" | "github.reconcile" | "github.dispose"
    )
}

pub(super) fn visible(text: &str) -> String {
    text.chars().flat_map(|ch| {
        if ch != '\n' && (ch.is_control() || matches!(ch, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')) {
            format!("\\u{{{:x}}}", ch as u32).chars().collect::<Vec<_>>()
        } else { vec![ch] }
    }).collect()
}

pub(super) fn lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut result = Vec::new();
    for source in visible(text).split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for ch in source.chars() {
            let size = ch.width().unwrap_or(0);
            if used + size > width && !line.is_empty() {
                result.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(ch);
            used += size;
        }
        result.push(line);
    }
    result
}

fn text(request: &ApprovalRequest) -> String {
    format!(
        "Action: {}\nTarget: {}\nRequest: {}\n\n{}",
        request.action, request.target, request.id, request.reason
    )
}

pub(super) fn limit(request: &ApprovalRequest, area: Rect) -> usize {
    lines(&text(request), area.width.saturating_sub(2) as usize)
        .len()
        .saturating_sub(area.height.saturating_sub(4) as usize)
}

pub(super) fn approve_enabled(request: &ApprovalRequest, scroll: usize, area: Rect) -> bool {
    area.width >= 20 && area.height >= 6 && scroll >= limit(request, area)
}

pub(super) fn navigate(key: KeyEvent, scroll: &mut usize, maximum: usize, height: usize) {
    match key.code {
        KeyCode::Down => *scroll = scroll.saturating_add(1).min(maximum),
        KeyCode::Up => *scroll = scroll.saturating_sub(1),
        KeyCode::PageDown | KeyCode::Char(' ') => {
            *scroll = scroll.saturating_add(height.max(1)).min(maximum)
        }
        KeyCode::PageUp => *scroll = scroll.saturating_sub(height.max(1)),
        KeyCode::Home => *scroll = 0,
        KeyCode::End => *scroll = maximum,
        _ => {}
    }
}

pub(super) fn draw(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    request: &ApprovalRequest,
    scroll: usize,
) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Exact GitHub decision ")
        .borders(Borders::ALL);
    let inside = block.inner(area);
    frame.render_widget(block, area);
    if area.width < 20 || area.height < 6 {
        frame.render_widget(Paragraph::new("Resize to review. Esc denies."), inside);
        return;
    }
    let height = inside.height.saturating_sub(2);
    let lines = lines(&text(request), inside.width as usize);
    let maximum = lines.len().saturating_sub(height as usize);
    let start = scroll.min(maximum);
    let view = lines
        .iter()
        .skip(start)
        .take(height as usize)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(view), Rect { height, ..inside });
    let footer = if start == maximum {
        "End · y approve · n/Esc deny"
    } else {
        "↑↓ PgUp/PgDn End · n/Esc deny"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Lines {}–{} / {}\n{footer}",
            start + 1,
            (start + height as usize).min(lines.len()),
            lines.len()
        )),
        Rect {
            y: inside.y + height,
            height: 2,
            ..inside
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_unicode_and_controls_survive_scrolling() {
        let text = "界λ\u{1b}[31m\u{202e}\tEND\n".repeat(100);
        let wrapped = lines(&text, 18);
        assert_eq!(wrapped.concat(), visible(&text).replace('\n', ""));
        assert!(!wrapped.concat().contains('\u{1b}'));
        assert!(wrapped.concat().contains("\\u{202e}"));
    }
    #[test]
    fn exact_approval_requires_end_and_readable_viewport() {
        let (response, _rx) = tokio::sync::oneshot::channel();
        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            action: "github.publish".into(),
            target: "https://github.com/o/r/issues/1".into(),
            reason: "body\n".repeat(100),
            response,
        };
        let area = Rect::new(0, 0, 40, 12);
        assert!(!approve_enabled(&request, 0, area));
        assert!(approve_enabled(&request, limit(&request, area), area));
        assert!(!approve_enabled(
            &request,
            usize::MAX,
            Rect::new(0, 0, 10, 4)
        ));
    }
}
