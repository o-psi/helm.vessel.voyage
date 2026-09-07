use super::super::{App, safe};
use super::{now_ms, wrap::wrap_lines};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(in crate::process_client::ui) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut review = app.interactions.borrow_mut();
    // Discard only drafts whose requests have positively disappeared from a snapshot.
    review.answers.retain(|(target, id), _| {
        app.views.get(target).is_some_and(|view| {
            view.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot
                    .decisions
                    .iter()
                    .any(|decision| decision.decision_id == *id)
            })
        })
    });
    let selected = app
        .selected
        .and_then(|target| app.views.get(&target).map(|view| (target, view)));
    let current = selected.and_then(|(target, view)| {
        view.snapshot.as_ref().and_then(|snapshot| {
            let index = review
                .selected
                .filter(|(old, _)| *old == target)
                .and_then(|(_, id)| {
                    snapshot
                        .decisions
                        .iter()
                        .position(|decision| decision.decision_id == id)
                })
                .unwrap_or(0);
            snapshot
                .decisions
                .get(index)
                .map(|decision| (target, view, decision, index, snapshot.decisions.len()))
        })
    });
    let identity = current.map(|(target, _, decision, _, _)| (target, decision.decision_id));
    if review.selected != identity {
        review.scroll = 0;
        review.focused = false;
    }
    review.selected = identity;
    review.displayed = identity;
    let Some((target, view, decision, index, count)) = current else {
        return;
    };
    if area.width < 16 || area.height < 10 {
        review.displayed = None;
        frame.render_widget(
            Paragraph::new("Interactions pending · enlarge terminal to review"),
            area,
        );
        return;
    }
    let kind = decision.request["kind"].as_str().unwrap_or("unknown");
    let label = match kind {
        "approval" => "Approval",
        "question" => "Question",
        _ => "Unsupported request",
    };
    let title = format!(
        "{label} {}/{} · {}",
        index + 1,
        count,
        if review.focused {
            "focused"
        } else {
            "F2 focus"
        }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer_height = inner.height.min(if kind == "question" { 8 } else { 5 });
    let body = Rect {
        height: inner.height.saturating_sub(footer_height),
        ..inner
    };
    let footer = Rect::new(inner.x, body.bottom(), inner.width, footer_height);
    let remaining = decision
        .expires_at_ms
        .saturating_sub(now_ms())
        .div_ceil(1000);
    let mut text = Text::from(vec![
        Line::from(format!("Host: {}", app.route_label(target.route))),
        Line::from(format!(
            "Voyage: {} · {remaining}s left",
            safe(&view.title())
        )),
        Line::default(),
    ]);
    let answer = review
        .answers
        .entry((target, decision.decision_id))
        .or_default();
    match kind {
        "approval" => {
            for (label, field) in [
                ("Action", "action"),
                ("Target", "target"),
                ("Reason", "reason"),
            ] {
                let value = &decision.request["approval"][field];
                let value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| super::super::presentation::fields(value));
                text.extend(Text::raw(format!("{label}: {}\n\n", safe(&value))));
            }
        }
        "question" => {
            text.extend(Text::raw(format!(
                "{}\n\n",
                safe(
                    decision.request["question"]["question"]
                        .as_str()
                        .unwrap_or("Invalid question")
                )
            )));
            for (option, value) in decision.request["question"]["options"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                text.extend(Text::raw(format!(
                    "{} {}. {}\n",
                    if answer.option == Some(option) {
                        ">"
                    } else {
                        " "
                    },
                    option + 1,
                    safe(value.as_str().unwrap_or("Invalid option"))
                )));
            }
            text.lines.push(Line::from(format!(
                "{} Custom answer",
                if answer.option.is_none() { ">" } else { " " }
            )));
        }
        _ => text.extend(Text::raw(
            "This request needs a newer version of Helm. It can't be answered here.",
        )),
    }
    let answer_text = safe(&answer.text.text);
    let answer_column = unicode_width::UnicodeWidthStr::width(
        safe(&answer.text.text[..answer.text.cursor]).as_str(),
    );
    let chosen = answer
        .option
        .and_then(|option| decision.request["question"]["options"].get(option))
        .and_then(|value| value.as_str())
        .map(safe);
    let lines = wrap_lines(text, body.width);
    let maximum = lines
        .len()
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    review.scroll = review.scroll.min(maximum);
    frame.render_widget(Paragraph::new(lines).scroll((review.scroll, 0)), body);
    let controls = if view.pending.is_some() {
        "Not confirmed yet · F4 check status"
    } else if remaining == 0 {
        "This request has expired"
    } else {
        match kind {
            "approval" => "Ctrl+A Approve once · Ctrl+D Deny",
            "question" if review.focused => "Enter sends · Ctrl+D skips question",
            "question" => "F2 to answer this question",
            _ => "No supported response",
        }
    };
    let (controls_area, editor) = if kind == "question" {
        (
            Rect {
                height: footer.height.saturating_sub(3),
                ..footer
            },
            Some(Rect::new(
                footer.x,
                footer.bottom().saturating_sub(3),
                footer.width,
                3,
            )),
        )
    } else {
        (footer, None)
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{controls}\nF2 focus · F6/F7 prev/next\nPgUp/PgDn scroll · Esc composer{}",
            if kind == "question" && review.focused {
                "\nUp/Down choose · type custom answer"
            } else {
                ""
            }
        ))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Yellow)),
        controls_area,
    );
    if let Some(editor) = editor {
        let (title, value) = chosen
            .as_ref()
            .map_or(("Custom answer", answer_text.as_str()), |value| {
                ("Selected answer", value.as_str())
            });
        let column = if chosen.is_some() {
            0
        } else {
            answer_column.min(u16::MAX as usize) as u16
        };
        let offset = column.saturating_sub(editor.width.saturating_sub(3));
        frame.render_widget(
            Paragraph::new(value)
                .scroll((0, offset))
                .block(Block::default().borders(Borders::ALL).title(title)),
            editor,
        );
        if review.focused {
            frame.set_cursor_position((
                editor.x.saturating_add(1 + column.saturating_sub(offset)),
                editor.y.saturating_add(1),
            ));
        }
    }
}
