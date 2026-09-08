use super::*;

impl App {
    /// Runs before composer editing. A modal pins its destination and consumes
    /// navigation so a selected image cannot land in a different voyage.
    pub(in crate::process_client::ui) fn attachment_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        if self.attachment_modal.is_none() {
            let opens = matches!(event, Event::Key(key)
                if key.kind != crossterm::event::KeyEventKind::Release
                && (key.code == KeyCode::F(6) || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('i'))));
            if !opens {
                return Ok(false);
            }
            if self.help
                || self.explore.is_some()
                || self.sidebar.menu.is_some()
                || self.interactions.borrow().focused
                || self.inference_picker_open()
            {
                return Ok(false);
            }
            let destination = if let Some(id) = self.active_draft {
                Destination::Draft(id)
            } else {
                let target = self.selected.context("Create or select a voyage first")?;
                let view = self.views.get(&target).context("Waiting for voyage")?;
                ensure!(
                    view.panel.is_none() && !view.terminals.open,
                    "Return to the composer before attaching an image"
                );
                Destination::Live(target)
            };
            self.attachment_images(destination)?;
            self.sidebar.resize.clear();
            self.completion = Default::default();
            self.attachment_modal = Some(Modal {
                destination,
                input: Default::default(),
                confirm_screenshot: false,
            });
            self.status = "Enter a local path; remove INDEX; or screenshot".into();
            return Ok(true);
        }

        if let Event::Key(key) = event {
            if key.kind == crossterm::event::KeyEventKind::Release {
                return Ok(true);
            }
            if key.code == KeyCode::Esc {
                let modal = self.attachment_modal.as_mut().expect("modal");
                if modal.confirm_screenshot {
                    modal.confirm_screenshot = false;
                    modal.input.take();
                    self.status = "Screenshot cancelled · text and attachments preserved".into();
                } else {
                    self.attachment_modal = None;
                    self.status = "Back to composer · text and attachments preserved".into();
                }
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                self.quit = true;
                return Ok(true);
            }
            if key.code == KeyCode::Enter {
                // Ignore key-repeat: one held Enter must not cross confirmations.
                if key.kind != crossterm::event::KeyEventKind::Press {
                    return Ok(true);
                }
                let modal = self.attachment_modal.as_ref().expect("modal");
                let destination = modal.destination;
                let input = modal.input.text.clone();
                if modal.confirm_screenshot {
                    ensure!(
                        input.trim() == "CAPTURE",
                        "Type CAPTURE to confirm, or Escape to cancel"
                    );
                    // Consume consent before the effect; failures require a fresh
                    // screenshot command and fresh confirmation before retrying.
                    let modal = self.attachment_modal.as_mut().expect("modal");
                    modal.confirm_screenshot = false;
                    modal.input.take();
                    self.capture_attachment(destination)?;
                } else if input.trim() == "screenshot" {
                    self.ensure_images_editable(destination)?;
                    let modal = self.attachment_modal.as_mut().expect("modal");
                    modal.confirm_screenshot = true;
                    modal.input.take();
                    self.status = "Nothing captured. Review the warning before confirming.".into();
                } else {
                    self.change_attachment(destination, &input)?;
                    self.attachment_modal.as_mut().expect("modal").input.take();
                }
                return Ok(true);
            }
        }

        let modal = self.attachment_modal.as_mut().expect("modal");
        match event {
            Event::Paste(text) => {
                ensure!(
                    !modal.confirm_screenshot,
                    "Type CAPTURE yourself or Escape to cancel"
                );
                ensure!(
                    !text.chars().any(char::is_control),
                    "Paste one path without control characters"
                );
                ensure!(
                    modal.input.text.len() + text.len() <= 4096,
                    "Path input limit is 4096 bytes"
                );
                modal.input.insert_str(text);
            }
            Event::Key(key) => match key.code {
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !ch.is_control() =>
                {
                    ensure!(
                        modal.input.text.len() + ch.len_utf8() <= 4096,
                        "Path input is too long"
                    );
                    modal.input.insert(ch);
                }
                KeyCode::Backspace => modal.input.backspace(),
                KeyCode::Delete => modal.input.delete(),
                KeyCode::Home => modal.input.line_start(),
                KeyCode::End => modal.input.line_end(),
                KeyCode::Left => {
                    modal.input.cursor = modal.input.text[..modal.input.cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i);
                }
                KeyCode::Right => {
                    if let Some(ch) = modal.input.text[modal.input.cursor..].chars().next() {
                        modal.input.cursor += ch.len_utf8();
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(true)
    }
}
