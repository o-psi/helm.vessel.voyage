use super::*;
use ratatui::{
    Frame,
    layout::{Margin, Rect},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

impl App {
    pub(in crate::process_client::ui) fn attachment_summary(&self) -> String {
        let images = if let Some(id) = self.active_draft {
            self.new_draft_images(id).unwrap_or(&[])
        } else {
            self.selected
                .and_then(|t| self.views.get(&t))
                .map_or(&[][..], |v| v.images.as_slice())
        };
        if images.is_empty() {
            "Ctrl+I / F6 Images".into()
        } else {
            format!(
                "Ctrl+I / F6 · {} image(s) · {} bytes",
                images.len(),
                images.iter().map(|i| i.byte_size).sum::<u64>()
            )
        }
    }

    pub(in crate::process_client::ui) fn draw_attachments(&self, frame: &mut Frame<'_>) {
        let Some(modal) = &self.attachment_modal else {
            return;
        };
        let screen = frame.area();
        let width = screen.width.saturating_sub(2).min(104);
        let height = screen.height.saturating_sub(2).min(24);
        if width < 8 || height < 4 {
            return;
        }
        let area = Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(if modal.confirm_screenshot {
                " Confirm local screenshot "
            } else {
                " Images · local files "
            })
            .title_bottom(" Enter apply · Esc back (preserves draft) ");
        let inner = block.inner(area).inner(Margin::new(1, 0));
        frame.render_widget(block, area);
        let mut lines = if modal.confirm_screenshot {
            vec![
                Line::styled(
                    "Entire Helm-local display: may contain secrets!",
                    Style::default().fg(Color::Yellow),
                ),
                Line::from("NOT the remote host. Local policy applies."),
                Line::from("Added to draft only; never auto-sent."),
                Line::from("Type CAPTURE + Enter. Esc cancels."),
            ]
        } else {
            let mut lines = vec![
                Line::from("PATH (or add PATH) · remove INDEX · screenshot"),
                Line::from("Up to 4 images, 2 MiB total. Files are read on this computer."),
                Line::default(),
            ];
            match self.attachment_images(modal.destination) {
                Ok([]) => lines.push(Line::from("No images attached.")),
                Ok(images) => lines.extend(
                    images
                        .iter()
                        .enumerate()
                        .map(|(i, image)| Line::from(image.label(i))),
                ),
                Err(_) => lines.push(Line::from(
                    "Destination unavailable; Escape preserves the draft.",
                )),
            }
            lines
        };
        lines.push(Line::default());
        lines.push(Line::from(safe(&self.status)));
        let body = Rect {
            height: inner.height.saturating_sub(3),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            body,
        );
        if inner.width > 0 && inner.height >= 2 {
            // Wrap and scroll the modal editor, never index display width as UTF-8 bytes.
            let editor = Rect::new(inner.x, inner.bottom() - 2, inner.width, 2);
            let (row, col) = composer::cursor_position(
                &safe(&modal.input.text[..modal.input.cursor]),
                editor.width,
            );
            let scroll = row.saturating_sub(editor.height - 1);
            frame.render_widget(
                Paragraph::new(presentation::wrap(
                    Text::raw(safe(&modal.input.text)),
                    editor.width,
                ))
                .scroll((scroll, 0)),
                editor,
            );
            frame.set_cursor_position((
                editor.x + col.min(editor.width - 1),
                editor.y + row.saturating_sub(scroll).min(editor.height - 1),
            ));
        }
    }
}
