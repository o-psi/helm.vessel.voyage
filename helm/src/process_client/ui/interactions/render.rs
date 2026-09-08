use super::super::{App, safe};
use super::{Control, now_ms};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

fn wrapped(text: impl Into<String>, width: u16) -> Vec<Line<'static>> {
    crate::markdown::wrap_text(Text::raw(text.into()), width.into()).lines
}

pub(in crate::process_client::ui) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut review = app.interactions.borrow_mut();
    review.displayed = None;
    review.hits.clear();
    review.area = Rect::default();
    let Some((target, id)) = review.selected else {
        return;
    };
    let Some(view) = app.views.get(&target) else {
        return;
    };
    let Some(snapshot) = &view.snapshot else {
        return;
    };
    let Some(index) = snapshot.decisions.iter().position(|d| d.decision_id == id) else {
        return;
    };
    let decision = &snapshot.decisions[index];
    if area.width < 20 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Enlarge the window to answer this request."),
            area,
        );
        return;
    }
    review.answers.retain(|(t, identity), _| {
        app.views.get(t).is_some_and(|v| {
            v.snapshot
                .as_ref()
                .is_none_or(|s| s.decisions.iter().any(|d| d.decision_id == *identity))
        })
    });
    let kind = decision.request["kind"].as_str().unwrap_or("");
    let title = if kind == "approval" {
        "Permission needed"
    } else {
        "Your answer needed"
    };
    let source = decision.request["approval"]["source"]["name"]
        .as_str()
        .map(safe);
    let title = source
        .as_ref()
        .map_or_else(|| title.to_owned(), |name| format!("{title} · {name}"));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " {title} · {}/{} ",
            index + 1,
            snapshot.decisions.len()
        ))
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let answer = review.answers.entry((target, id)).or_default();
    if kind == "question"
        && decision.request["question"]["options"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        answer.option = None;
    }
    let editing = answer.editing;
    let selected = answer.option;
    let answer_text = safe(&answer.text.text);
    let column = unicode_width::UnicodeWidthStr::width(
        safe(&answer.text.text[..answer.text.cursor]).as_str(),
    ) as u16;
    let footer_height = if editing { 8 } else { 5 };
    let body = Rect {
        height: inner.height.saturating_sub(footer_height),
        ..inner
    };
    let footer = Rect::new(
        inner.x,
        body.bottom(),
        inner.width,
        inner.height - body.height,
    );
    let remaining = decision
        .expires_at_ms
        .saturating_sub(now_ms())
        .div_ceil(1000);
    let mut lines = wrapped(format!("{remaining}s left to respond\n"), body.width);
    let question = if kind == "question" {
        safe(
            decision.request["question"]["question"]
                .as_str()
                .unwrap_or("Question unavailable"),
        )
    } else if kind == "approval" {
        [
            ("Action", "action"),
            ("Target", "target"),
            ("Reason", "reason"),
        ]
        .into_iter()
        .map(|(label, field)| {
            let v = &decision.request["approval"][field];
            format!(
                "{label}: {}",
                v.as_str()
                    .map(safe)
                    .unwrap_or_else(|| super::super::presentation::fields(v))
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
    } else {
        "This request needs a newer version of Helm. It cannot be answered here.".into()
    };
    lines.extend(wrapped(question, body.width));
    lines.push(Line::default());
    let options: Vec<String> = if kind == "question" {
        decision.request["question"]["options"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| safe(v.as_str().unwrap_or("Unavailable answer")))
            .chain(std::iter::once("Write a custom answer…".into()))
            .collect()
    } else if kind == "approval" {
        vec!["Deny".into(), "Allow once".into()]
    } else {
        Vec::new()
    };
    let chosen = selected.unwrap_or(options.len().saturating_sub(1));
    let mut option_ranges = Vec::new();
    let mut selected_start = 0;
    let mut selected_end = 0;
    for (i, option) in options.iter().enumerate() {
        let start = lines.len();
        for (part, line) in wrapped(option.clone(), body.width.saturating_sub(4))
            .into_iter()
            .enumerate()
        {
            let prefix = if part == 0 && i == chosen {
                " > "
            } else {
                "   "
            };
            let mut spans = vec![Span::raw(prefix)];
            spans.extend(line.spans);
            let line = Line::from(spans);
            lines.push(if i == chosen {
                line.style(
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                line
            });
        }
        option_ranges.push((start, lines.len(), i));
        if i == chosen {
            selected_start = start;
            selected_end = lines.len();
        }
        lines.push(Line::default());
    }
    let max = lines.len().saturating_sub(body.height as usize);
    let mut scroll = usize::from(review.scroll).min(max);
    if review.follow_selection {
        // Keep the next choice visible when it fits alongside the selected one.
        let preview_end = selected_end.saturating_add(2).min(lines.len());
        let visible_end = if preview_end - selected_start <= body.height as usize {
            preview_end
        } else {
            selected_end
        };
        if selected_start < scroll || selected_end - selected_start >= body.height as usize {
            scroll = selected_start;
        } else if visible_end > scroll + body.height as usize {
            scroll = visible_end.saturating_sub(body.height as usize);
        }
        review.follow_selection = false;
    }
    scroll = scroll.min(max);
    review.scroll = scroll.min(u16::MAX as usize) as u16;
    let enabled =
        view.pending.is_none() && remaining > 0 && matches!(kind, "question" | "approval");
    if enabled {
        for (start, end, i) in option_ranges {
            let top = start.max(scroll);
            let bottom = end.min(scroll + body.height as usize);
            if top < bottom {
                review.hits.push((
                    Rect::new(
                        body.x,
                        body.y + (top - scroll) as u16,
                        body.width,
                        (bottom - top) as u16,
                    ),
                    Control::Choice(if kind == "question" && i == options.len() - 1 {
                        None
                    } else {
                        Some(i)
                    }),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines).scroll((review.scroll, 0)), body);
    let controls = if view.pending.is_some() {
        "Sending answer… Unconfirmed responses are checked automatically".to_owned()
    } else if remaining == 0 {
        "This request has expired. Nothing will be sent.".into()
    } else if editing {
        "Type · Enter Send · Esc Back".into()
    } else if kind == "approval" {
        "Up/Down Choose · Enter Confirm · Esc Deny".into()
    } else {
        "Up/Down Choose · Enter Select · Esc Skip".into()
    };
    let extra = if snapshot.decisions.len() > 1 {
        "Left/Right Requests · PgUp/PgDn Read"
    } else if max > 0 {
        "PgUp/PgDn Details"
    } else {
        ""
    };
    let controls_area = Rect {
        height: 2.min(footer.height),
        ..footer
    };
    frame.render_widget(
        Paragraph::new(wrapped(format!("{controls}\n{extra}"), controls_area.width)),
        controls_area,
    );
    // Separate selection from confirmation, including for mouse approvals.
    let mut buttons = vec![
        (
            if editing { "[Send]" } else { "[Confirm]" },
            Control::Confirm,
        ),
        (
            if editing {
                "[Back]"
            } else if kind == "approval" {
                "[Deny]"
            } else {
                "[Skip]"
            },
            Control::Cancel,
        ),
    ];
    if snapshot.decisions.len() > 1 && !editing {
        buttons.extend([("[Prev]", Control::Previous), ("[Next]", Control::Next)]);
    }
    let mut x = footer.x;
    let mut y = footer.y + 2;
    for (index, (label, control)) in buttons.into_iter().enumerate() {
        if index == 2 {
            x = footer.x;
            y += 1;
        }
        let width = label.len() as u16;
        let rect = Rect::new(x, y, width, 1);
        if rect.right() <= footer.right() && rect.bottom() <= footer.bottom() {
            frame.render_widget(
                Paragraph::new(label).style(Style::default().fg(if enabled {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })),
                rect,
            );
            if enabled {
                review.hits.push((rect, control));
            }
        }
        x += width + 1;
    }
    if editing {
        let editor = Rect::new(footer.x, footer.bottom().saturating_sub(3), footer.width, 3);
        let offset = column.saturating_sub(editor.width.saturating_sub(3));
        frame.render_widget(
            Paragraph::new(answer_text).scroll((0, offset)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Your answer "),
            ),
            editor,
        );
        frame.set_cursor_position((editor.x + 1 + column.saturating_sub(offset), editor.y + 1));
    }
    review.area = area;
    review.displayed = Some((target, id));
}
