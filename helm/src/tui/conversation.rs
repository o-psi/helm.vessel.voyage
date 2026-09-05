//! Transcript presentation, activity and viewport anchoring.

use super::{App, text::display_safe, tool_output};
use crate::{
    markdown::{MarkdownTheme, RenderOptions, render_markdown},
    model::Role,
};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use std::{collections::BTreeSet, time::Duration};

pub(super) fn working_message(elapsed: Duration) -> String {
    const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    const MESSAGES: &[&str] = &[
        "Consulting the rubber duck…",
        "Untangling the plot…",
        "Putting the bits in shipshape…",
        "Negotiating with the semicolons…",
        "Following a suspiciously useful hunch…",
        "Turning coffee into progress…",
        "Checking under the hood…",
        "Convincing the pieces to cooperate…",
    ];
    format!(
        "{} {}",
        FRAMES[(elapsed.as_millis() / 100 % FRAMES.len() as u128) as usize],
        MESSAGES[(elapsed.as_secs() / 4 % MESSAGES.len() as u64) as usize]
    )
}

pub(super) fn transcript(app: &App, width: usize) -> Text<'static> {
    let mut lines = Vec::new();
    let messages: Vec<_> = app
        .session
        .messages
        .iter()
        .filter(|message| {
            !message
                .steering
                .as_ref()
                .is_some_and(|receipt| receipt.status == crate::model::SteeringStatus::Queued)
        })
        .chain(&app.live_messages)
        .collect();
    // Walk backwards so each stretch keeps its own newest three calls. An
    // assistant's text precedes its calls, so reset after visiting those calls.
    let mut visible_calls = BTreeSet::new();
    let mut calls_in_stretch = 0;
    for message in messages.iter().rev() {
        for call in message.tool_calls.iter().rev() {
            if app.show_activity || calls_in_stretch < 3 {
                visible_calls.insert(call.id.as_str());
            }
            calls_in_stretch += 1;
        }
        if message.role == Role::User
            || (message.role == Role::Assistant && !message.content.is_empty())
        {
            calls_in_stretch = 0;
        }
    }
    for message in &messages {
        if message.role == Role::System || message.role == Role::Tool {
            continue;
        }
        let (label, color) = match message.role {
            Role::User => ("you", Color::Cyan),
            Role::Assistant => ("helm", Color::Green),
            _ => continue,
        };
        if !message.content.is_empty() {
            let label = match message.steering.as_ref().map(|receipt| &receipt.status) {
                Some(crate::model::SteeringStatus::Queued) => "you · steering queued",
                Some(crate::model::SteeringStatus::Applied) => "you · steering applied",
                Some(crate::model::SteeringStatus::NotApplied) => "you · steering not applied",
                Some(crate::model::SteeringStatus::UnknownAfterRestart) => {
                    "you · steering delivery unknown after restart"
                }
                None => label,
            };
            lines.push(Line::from(Span::styled(
                label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));
            if message.role == Role::Assistant {
                lines.extend(
                    render_markdown(
                        &message.content,
                        RenderOptions {
                            width,
                            theme: app.markdown_theme,
                            syntax_highlighting: app.markdown_syntax_highlighting,
                            ..Default::default()
                        },
                    )
                    .lines,
                );
            } else {
                lines.extend(
                    message
                        .content
                        .lines()
                        .map(|line| Line::raw(display_safe(line))),
                );
            }
            lines.push(Line::raw(""));
        }
        for call in &message.tool_calls {
            if !visible_calls.contains(call.id.as_str()) {
                continue;
            }
            let result = messages.iter().find(|message| {
                message.role == Role::Tool
                    && message.tool_call_id.as_deref() == Some(call.id.as_str())
            });
            let live = app
                .live_messages
                .iter()
                .any(|message| message.tool_calls.iter().any(|live| live.id == call.id));
            lines.extend(tool_output::render(
                call,
                result.copied(),
                app.is_running() && live,
                app.tool_details,
                width,
            ));
            lines.push(Line::raw(""));
        }
    }
    if !app.streaming_response.is_empty() {
        lines.push(Line::from(Span::styled(
            "helm · streaming",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.extend(
            render_markdown(
                &app.streaming_response,
                RenderOptions {
                    width,
                    theme: app.markdown_theme,
                    syntax_highlighting: app.markdown_syntax_highlighting,
                    ..Default::default()
                },
            )
            .lines,
        );
        lines.push(Line::raw(""));
    }
    // Pending inputs have not acquired a canonical position in the model history.
    // Present them after live output until the boundary snapshot reconciles ordering.
    for message in &app.session.messages {
        if message
            .steering
            .as_ref()
            .is_some_and(|receipt| receipt.status == crate::model::SteeringStatus::Queued)
        {
            lines.push(Line::styled(
                "you · steering queued",
                Style::default().fg(Color::Cyan),
            ));
            lines.extend(
                message
                    .content
                    .lines()
                    .map(|line| Line::raw(display_safe(line))),
            );
            lines.push(Line::raw(""));
        }
    }
    if app.show_activity && !app.activity.is_empty() {
        lines.push(Line::styled(
            "activity",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(
            app.activity
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|line| Line::styled(display_safe(line), Style::default().fg(Color::DarkGray))),
        );
    }
    if app.is_running() {
        let elapsed = app.working_since.elapsed();
        let message = if app.approval.is_some() {
            "Waiting for your approval".to_owned()
        } else if app
            .running
            .as_ref()
            .is_some_and(|running| running.cancel.is_cancelled())
        {
            "Stopping…".to_owned()
        } else {
            working_message(elapsed)
        };
        lines.push(Line::styled(message, Style::default().fg(Color::Cyan)));
    }
    Text::from(lines)
}

pub(super) fn viewport_width_for(area: Rect) -> usize {
    area.width.max(1) as usize
}

pub(super) fn markdown_theme() -> MarkdownTheme {
    markdown_theme_for(std::env::var_os("NO_COLOR").is_some())
}

pub(super) fn markdown_theme_for(no_color: bool) -> MarkdownTheme {
    if !no_color {
        return MarkdownTheme::default();
    }
    MarkdownTheme {
        text: Color::Reset,
        heading: Color::Reset,
        link: Color::Reset,
        code: Color::Reset,
        code_background: Color::Reset,
        quote: Color::Reset,
        rule: Color::Reset,
        table_header: Color::Reset,
        warning: Color::Reset,
    }
}

pub(super) fn transcript_height(app: &App, width: usize) -> usize {
    transcript(app, width)
        .lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width.max(1)))
        .sum()
}

pub(super) fn max_conversation_scroll(app: &App) -> u16 {
    transcript_height(app, app.conversation_width)
        .saturating_sub(app.conversation_height)
        .min(u16::MAX as usize) as u16
}

pub(super) fn scroll_conversation(app: &mut App, lines: i16) {
    let next = if lines >= 0 {
        app.scroll.saturating_add(lines as u16)
    } else {
        app.scroll.saturating_sub(lines.unsigned_abs())
    };
    app.scroll = next.min(max_conversation_scroll(app));
}

pub(super) fn preserve_manual_anchor(app: &mut App, previous_height: usize) {
    if app.scroll == 0 {
        return;
    }
    let next = transcript_height(app, app.conversation_width);
    if next >= previous_height {
        app.scroll = app
            .scroll
            .saturating_add((next - previous_height).min(u16::MAX as usize) as u16);
    } else {
        app.scroll = app
            .scroll
            .saturating_sub((previous_height - next).min(u16::MAX as usize) as u16);
    }
}

pub(super) fn resize_conversation(app: &mut App, width: usize, height: usize) {
    let previous = transcript_height(app, app.conversation_width);
    let previous_bottom = previous.saturating_sub(app.conversation_height);
    let anchored_top = previous_bottom.saturating_sub(app.scroll as usize);
    app.conversation_width = width.max(1);
    app.conversation_height = height.max(1);
    if app.scroll > 0 {
        let next_bottom =
            transcript_height(app, app.conversation_width).saturating_sub(app.conversation_height);
        app.scroll = next_bottom
            .saturating_sub(anchored_top)
            .min(u16::MAX as usize) as u16;
    }
}
