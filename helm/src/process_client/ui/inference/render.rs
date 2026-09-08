use super::*;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

impl App {
    pub(in crate::process_client::ui) fn clear_inference_hits(&self) {
        self.inference.access.hit.set(None);
        self.inference.access.visible.set(false);
        self.inference.visible.set(false);
        self.inference.hits.borrow_mut().clear();
        self.inference.choices.borrow_mut().clear();
    }
    pub(in crate::process_client::ui) fn draw_inference_controls(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let Some(destination) = self.inference_destination() else {
            return;
        };
        let settings = self.inference_settings(destination).ok();
        let fields = [Field::Model, Field::Thinking, Field::Service];
        let columns = Layout::horizontal([Constraint::Percentage(25); 4]).split(area);
        self.draw_access_control(frame, columns[3], destination);
        for (field, area) in fields.into_iter().zip(columns.iter().copied()) {
            let value = settings
                .as_ref()
                .map(|s| s.label(field))
                .unwrap_or_else(|| "unavailable".into());
            let hover = self
                .sidebar
                .pointer
                .is_some_and(|point| area.contains(point));
            let style = Style::default().fg(if settings.is_some() {
                Color::Cyan
            } else {
                Color::DarkGray
            });
            frame.render_widget(
                Paragraph::new(format!("{}: {} ▾", field.name(), safe(&value))).style(if hover {
                    style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
                } else {
                    style
                }),
                area,
            );
            // Keep unavailable controls discoverable; activation explains why.
            self.inference
                .hits
                .borrow_mut()
                .push((area, destination, field));
        }
    }
    pub(in crate::process_client::ui) fn draw_inference_picker(&self, frame: &mut Frame<'_>) {
        self.draw_draft_access(frame);
        let Some(picker) = &self.inference.picker else {
            return;
        };
        self.inference.visible.set(true);
        let screen = frame.area();
        let width = screen.width.saturating_sub(4).min(94);
        let height = screen.height.saturating_sub(2).min(20);
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let block = Block::default().borders(Borders::ALL).title(format!(
            " {} · ↑↓ select · Enter apply · Esc cancel ",
            picker.field.name()
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let current = match picker.destination {
            Destination::Live(target) => self
                .views
                .get(&target)
                .and_then(|v| v.snapshot.as_ref())
                .and_then(|s| s.inference_current.as_ref()),
            _ => None,
        };
        let active = match picker.destination {
            Destination::Live(target) => self
                .views
                .get(&target)
                .and_then(|v| v.snapshot.as_ref())
                .is_some_and(|s| {
                    s.inference_next_turn || s.run.as_ref().is_some_and(|r| r.active())
                }),
            _ => false,
        };
        let summary = |s: &Settings| {
            format!(
                "{} · thinking {} · service {}",
                safe(&s.model),
                safe(&s.label(Field::Thinking)),
                safe(&format!(
                    "{}{}",
                    s.label(Field::Service),
                    s.resolution
                        .as_ref()
                        .and_then(|r| r.service.provider_reported.as_ref())
                        .map(|tier| format!(" · last reported {tier}"))
                        .unwrap_or_default()
                ))
            )
        };
        let heading = format!(
            "{}: {}\n{}",
            if active { "Next turn" } else { "Saved" },
            summary(&picker.original),
            current
                .map(|s| format!("Current turn (unchanged): {}", summary(s)))
                .unwrap_or_else(|| if active {
                    "Current turn is unchanged by this selection".into()
                } else {
                    format!("Provider: {}", safe(&picker.original.provider))
                })
        );
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .split(inner);
        frame.render_widget(
            Paragraph::new(heading).style(Style::default().fg(Color::DarkGray)),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(if let Some(settings) = &picker.confirmation {
                format!("Confirm model change → {}", safe(&settings.model))
            } else {
                format!(
                    "Search / explicit value: {}{}",
                    safe(&picker.query),
                    if picker.loading {
                        " (loading catalog…)"
                    } else {
                        ""
                    }
                )
            })
            .style(Style::default().fg(Color::Cyan)),
            rows[1],
        );
        let options = picker.options();
        let offset = picker
            .selected
            .saturating_sub(rows[2].height.saturating_sub(1) as usize);
        for (row, (index, value)) in
            (rows[2].y..rows[2].bottom()).zip(options.iter().enumerate().skip(offset))
        {
            let rect = Rect::new(rows[2].x, row, rows[2].width, 1);
            let hover = self
                .sidebar
                .pointer
                .is_some_and(|point| rect.contains(point));
            let explicit = picker.confirmation.is_none() && !picker.options.contains(value);
            let text = format!(
                "{} {}{}",
                if index == picker.selected { ">" } else { " " },
                safe(value),
                if explicit {
                    " (explicit; support unverified)"
                } else if picker.confirmation.is_none() && value == "inherit" {
                    " (clear override)"
                } else {
                    ""
                }
            );
            frame.render_widget(
                Paragraph::new(text).style(
                    Style::default()
                        .fg(if index == picker.selected {
                            Color::Cyan
                        } else {
                            Color::White
                        })
                        .bg(if hover { Color::DarkGray } else { Color::Reset }),
                ),
                rect,
            );
            self.inference.choices.borrow_mut().push((rect, index));
        }
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n{}",
                picker.notice,
                if active {
                    "Applies to the next turn only. Composer text is kept."
                } else {
                    "Composer text is kept. Inherit selects catalog defaults, otherwise provider-managed."
                }
            ))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Yellow)),
            rows[3],
        );
        if picker.confirmation.is_none() && rows[1].width > 0 {
            let col = super::super::composer::cursor_position(
                &format!("Search / explicit value: {}", safe(&picker.query)),
                u16::MAX,
            )
            .1;
            frame.set_cursor_position((rows[1].x + col.min(rows[1].width - 1), rows[1].y));
        }
    }
}
