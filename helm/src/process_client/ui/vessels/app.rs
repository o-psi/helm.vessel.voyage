use super::*;
use crate::process_client::ui::{App, Route};
use crossterm::event::{Event as Input, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::widgets::Paragraph;

impl App {
    pub(in crate::process_client::ui) fn vessels_open(&self) -> bool {
        self.vessels.as_ref().is_some_and(|m| m.borrow().is_open())
    }
    pub(in crate::process_client::ui) fn open_vessels(&mut self) {
        // A clipboard acquisition begun in the composer must not complete after
        // the operator has switched to credential setup and copied an invitation.
        self.cancel_paste_for_private_panel();
        if self.vessels.is_none() {
            let root =
                crate::process_client::cli::default_directory().with_file_name("helm-connections");
            match Manager::open(root) {
                Ok(manager) => self.vessels = Some(std::cell::RefCell::new(manager)),
                Err(_) => {
                    self.status = "Cannot open private Vessels storage. Check owner permissions; local work and drafts are untouched.".into();
                    return;
                }
            }
        }
        if let Some(manager) = &self.vessels {
            manager.borrow_mut().open_panel();
        }
        self.sidebar.resize.clear();
        self.sidebar.pointer = None;
    }
    pub(in crate::process_client::ui) fn vessel_input(
        &mut self,
        event: &Input,
    ) -> anyhow::Result<bool> {
        if self.vessels_open() {
            if matches!(event, Input::Key(k) if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c'|'q')))
            {
                self.quit = true;
                return Ok(true);
            }
            let actions = self
                .vessels
                .as_ref()
                .unwrap()
                .borrow_mut()
                .handle_event(event, &self.sender);
            self.vessel_actions(actions);
            return Ok(true);
        }
        let open = matches!(event, Input::Key(k) if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('g'))
            || matches!(event, Input::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) && (self.vessel_button.get().contains((m.column,m.row).into()) || self.vessel_sidebar_button.get().contains((m.column,m.row).into())));
        if open {
            self.open_vessels();
            return Ok(true);
        }
        Ok(false)
    }
    pub(in crate::process_client::ui) fn vessel_update(&mut self, event: super::Event) {
        let actions = self
            .vessels
            .as_ref()
            .map(|m| m.borrow_mut().apply(event))
            .unwrap_or_default();
        self.vessel_actions(actions);
    }
    fn vessel_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::ConnectLocal => {
                    if let Some(client) = self.clients.local_client() {
                        self.activate_client(client);
                    } else {
                        self.status = "This launch has no local route. Start ordinary Helm to include this computer.".into();
                    }
                }
                Action::DisconnectLocal => {
                    if let Some(client) = self.clients.local_client() {
                        self.disconnect_connection(client.id());
                    }
                }
                Action::Activate(connection) => {
                    if let Some(manager) = &self.vessels {
                        let client = connection.client(manager.borrow().registry());
                        self.activate_client(client);
                    }
                }
                Action::Disconnect(id) | Action::Forget(id) => self.disconnect_connection(id),
                Action::New(id) => {
                    let route = self
                        .clients
                        .routes()
                        .find(|r| id.map_or(self.clients[*r].is_local(), |id| r.id == id));
                    if let Some(route) = route {
                        if let Err(error) = self.create_on_route(route, None) {
                            self.status = crate::process_client::safe(&error.to_string());
                        }
                    } else {
                        self.status = "Connect this Vessel before creating a conversation.".into();
                    }
                }
                Action::FilterLocal => {
                    self.vessel_filter = self
                        .clients
                        .routes()
                        .find(|r| self.clients[*r].is_local())
                        .map(|r| r.id);
                    self.status =
                        "Showing this computer. Vessels → All restores every machine.".into();
                }
                Action::Filter(id) => {
                    self.vessel_filter = id;
                    self.status = if id.is_some() {
                        "Vessel filter applied. Open Vessels → All to see every machine."
                    } else {
                        "Showing conversations from all Vessels."
                    }
                    .into();
                }
            }
        }
    }
    pub(in crate::process_client::ui) fn vessel_state(&self, route: Route, state: ConnectionState) {
        if let Some(manager) = &self.vessels {
            manager
                .borrow_mut()
                .set_state(route.id, route.generation, state);
        }
    }
    pub(in crate::process_client::ui) fn vessel_hover_style(
        &self,
        rect: Rect,
    ) -> ratatui::style::Style {
        if !self.vessels_open()
            && self
                .sidebar
                .pointer
                .is_some_and(|point| rect.contains(point))
        {
            crate::theme::Role::Hover.style()
        } else {
            ratatui::style::Style::default()
        }
    }
    pub(in crate::process_client::ui) fn draw_vessel_control(&self, frame: &mut Frame<'_>) {
        // The sidebar is primary; this button is only a fallback when it is hidden.
        // Clear the previous target so resizing cannot leave an invisible button.
        self.vessel_button.set(Rect::default());
        if !self.vessel_sidebar_button.get().is_empty() {
            return;
        }
        let area = frame.area();
        let width = area.width.min(20);
        let rect = Rect::new(
            area.right().saturating_sub(width),
            area.y,
            width,
            u16::from(area.height > 0),
        );
        self.vessel_button.set(rect);
        let label = if width >= 18 {
            " Vessels [Ctrl+G] "
        } else {
            " Vessels "
        };
        frame.render_widget(
            Paragraph::new(label).style(
                crate::theme::Role::Selection
                    .style()
                    .patch(self.vessel_hover_style(rect)),
            ),
            rect,
        );
    }
}

pub(in crate::process_client::ui) fn classify_error(error: &str) -> ConnectionState {
    let error = error.to_ascii_lowercase();
    if error.contains("access unavailable") {
        ConnectionState::AccessUnavailable
    } else if error.contains("expired") || error.contains("expiry") {
        ConnectionState::AccessExpired
    } else if error.contains("revoked")
        || error.contains("unauthorized")
        || error.contains("401")
        || error.contains("403")
    {
        ConnectionState::AccessRevoked
    } else if error.contains("identity") || error.contains("vessel id") || error.contains("pinned")
    {
        ConnectionState::IdentityChanged
    } else if error.contains("protocol") || error.contains("unsupported version") {
        ConnectionState::UnsupportedVersion
    } else {
        ConnectionState::Offline
    }
}
