//! Review the exact displayed approval without turning composer input into consent.
use super::{
    App, drafts,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use uuid::Uuid;
use voyage_protocol::process::RuntimeCommand;

#[derive(Default)]
pub(super) struct Review {
    identity: Option<(Target, Uuid)>,
    focused: bool,
    scroll: u16,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |time| {
            time.as_millis().min(u64::MAX as u128) as u64
        })
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let selected = app
        .selected
        .and_then(|target| app.views.get(&target).map(|view| (target, view)));
    let approval = selected.and_then(|(target, view)| {
        view.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .decisions
                .iter()
                .find(|decision| decision.request["kind"] == "approval")
                .map(|decision| (target, view, decision))
        })
    });
    let mut review = app.approval.borrow_mut();
    let identity = approval.map(|(target, _, decision)| (target, decision.decision_id));
    if review.identity != identity {
        *review = Review {
            identity,
            ..Review::default()
        };
    }
    let Some((target, view, decision)) = approval else {
        return;
    };
    if area.width < 16 || area.height < 7 {
        review.identity = None;
        frame.render_widget(
            Paragraph::new("Approval pending · enlarge terminal to review"),
            area,
        );
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if review.focused {
            "Approval · scrolling"
        } else {
            "Approval required"
        })
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let footer_height = inner.height.min(5);
    let body = Rect {
        height: inner.height.saturating_sub(footer_height),
        ..inner
    };
    let footer = Rect::new(inner.x, body.bottom(), inner.width, footer_height);
    let request = &decision.request["approval"];
    let remaining = decision
        .expires_at_ms
        .saturating_sub(now_ms())
        .div_ceil(1000);
    let mut text = Text::from(vec![
        Line::from(format!("Host: {}", app.route_label(target.route))),
        Line::from(format!("Voyage: {}", super::safe(&view.title()))),
        Line::from(format!("Expires in {remaining}s")),
        Line::default(),
    ]);
    for (label, field) in [
        ("Action", "action"),
        ("Target", "target"),
        ("Reason", "reason"),
    ] {
        let value = request[field]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| request[field].to_string());
        text.extend(Text::raw(format!("{label}: {}\n\n", super::safe(&value))));
    }
    text.extend(Text::raw(format!(
        "Session: {}\nRun: {}\nRequest: {}",
        target.session, decision.run_id, decision.decision_id
    )));
    // Wrap literal text ourselves so scrolling is bounded by the exact rendered lines.
    let lines = wrap_lines(text, body.width);
    let maximum = lines
        .len()
        .saturating_sub(body.height as usize)
        .min(u16::MAX as usize) as u16;
    review.scroll = review.scroll.min(maximum);
    frame.render_widget(Paragraph::new(lines).scroll((review.scroll, 0)), body);
    let controls = if view.pending.is_some() {
        "Delivery pending or unknown\nUse /receipt to resolve"
    } else if remaining == 0 {
        "Expired · approval disabled"
    } else {
        "[Ctrl+A Approve once]\n[Ctrl+D Deny]"
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{controls}\nF2 focus · PgUp/PgDn scroll\nEsc returns to composer"
        ))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Yellow)),
        footer,
    );
}

fn wrap_lines(text: Text<'_>, width: u16) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut lines = Vec::new();
    for line in text.lines {
        let content: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let mut output = String::new();
        let mut used = 0;
        for grapheme in content.graphemes(true) {
            let size = grapheme.width();
            if used + size > width.max(1) as usize && !output.is_empty() {
                lines.push(Line::from(std::mem::take(&mut output)));
                used = 0;
            }
            output.push_str(grapheme);
            used += size;
        }
        lines.push(Line::from(output));
    }
    lines
}

impl App {
    pub(super) fn approval_input(&mut self, event: &Event) -> Result<bool> {
        let mut review = self.approval.borrow_mut();
        let Some((target, decision_id)) = review.identity else {
            return Ok(false);
        };
        if self.selected != Some(target) {
            return Ok(false);
        }
        let Event::Key(key) = event else {
            return Ok(false);
        };
        let response =
            key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('a' | 'd'));
        if key.kind != KeyEventKind::Press {
            return Ok(response);
        }
        match key.code {
            KeyCode::F(2) => review.focused = !review.focused,
            KeyCode::Esc if review.focused => review.focused = false,
            KeyCode::PageUp if review.focused => review.scroll = review.scroll.saturating_sub(10),
            KeyCode::PageDown if review.focused => review.scroll = review.scroll.saturating_add(10),
            _ if response => {
                let approved = key.code == KeyCode::Char('a');
                drop(review);
                self.respond_to_review(target, decision_id, approved)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn respond_to_review(
        &mut self,
        target: Target,
        decision_id: Uuid,
        approved: bool,
    ) -> Result<()> {
        let view = self
            .views
            .get_mut(&target)
            .context("reviewed voyage unavailable")?;
        ensure!(
            view.pending.is_none(),
            "resolve pending delivery with /receipt before responding"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("waiting for current snapshot")?;
        let decision = snapshot
            .decisions
            .iter()
            .find(|decision| decision.decision_id == decision_id)
            .context("reviewed approval is no longer pending")?;
        ensure!(
            decision.request["kind"] == "approval",
            "reviewed request is not an approval"
        );
        ensure!(
            decision.incarnation == view.process.incarnation,
            "approval belongs to an earlier runtime"
        );
        ensure!(
            decision.expires_at_ms > now_ms(),
            "approval expired; nothing sent"
        );
        let command_id = Uuid::new_v4();
        let command = RuntimeCommand::Respond {
            command_id,
            expected_revision: snapshot.revision,
            expires_at_ms: decision.expires_at_ms.min(now_ms().saturating_add(60_000)),
            run_id: decision.run_id,
            decision_id,
            response: serde_json::json!(if approved { "approved" } else { "denied" }),
        };
        view.pending = Some(Pending {
            command_id,
            incarnation: view.process.incarnation,
            draft: String::new(),
            preserve_draft: true,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist response identity; nothing sent"));
        }
        self.dispatch(target, command_id, command);
        Ok(())
    }
}
