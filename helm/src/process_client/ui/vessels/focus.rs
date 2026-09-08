//! Focus owns only control identities, never private input values.
use super::{Button, Manager, Page};

impl Manager {
    pub(super) fn sync_focus(&mut self) {
        let controls = if self.panel.busy.is_some() {
            vec![Button::Back]
        } else {
            match self.panel.page {
                Page::List => Vec::new(),
                Page::Form { import, .. } => {
                    let mut controls = vec![Button::Field(0)];
                    if !import {
                        controls.push(Button::Field(1));
                    }
                    controls.extend([
                        Button::Field(2),
                        Button::Auto,
                        Button::Submit,
                        Button::Import,
                        Button::Back,
                    ]);
                    controls
                }
                Page::Rename(_) => vec![Button::Field(0), Button::Submit, Button::Back],
                // Start confirmations on Back: opening a preview is not consent.
                Page::Preview { .. } => vec![Button::Back, Button::Auto, Button::Save],
                Page::Forget(_) => vec![Button::Back, Button::Save],
            }
        };
        if self.panel.focus.elements() != controls.as_slice() {
            self.panel.focus.clear();
            self.panel.focus.register_all(controls);
            self.panel.reveal_selection = true;
        }
        if let Some(Button::Field(index)) = self.panel.focus.current() {
            self.panel.field = *index;
        }
    }

    pub(super) fn focus_control(&mut self, button: Button) {
        self.panel.focus.set(button);
        if let Some(Button::Field(index)) = self.panel.focus.current() {
            self.panel.field = *index;
        }
        self.panel.reveal_selection = true;
    }
}
