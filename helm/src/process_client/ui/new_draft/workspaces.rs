//! Remote paths are selected from executing-host authority, never local defaults.
use super::*;
use crate::process_client::connections::{Scope, Workspace};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::cell::RefCell;

pub(super) fn authorized(client: &Client) -> Result<Vec<Workspace>> {
    let connection = client.managed().context("Import this access in Vessels before creating a remote voyage. Shared-conversation access cannot create new voyages; request an owner-issued workspace pairing invitation.")?;
    anyhow::ensure!(
        matches!(connection.scope, Scope::Workspaces { .. }),
        "Shared conversation only. Ask the Vessel owner for a workspace pairing invitation, then choose Vessels → Add. Existing access cannot be broadened here."
    );
    let metadata = &connection.metadata;
    anyhow::ensure!(
        metadata.features.iter().any(|s| s == "workspace_pairing"),
        "This Vessel does not advertise workspace creation; upgrade the executing host"
    );
    anyhow::ensure!(
        metadata.rights.iter().any(|s| s == "create"),
        "This access does not permit creating voyages; request broader owner-approved access"
    );
    anyhow::ensure!(
        metadata
            .expires_at_ms
            .is_some_and(|expires| expires > chrono::Utc::now().timestamp_millis().max(0) as u64),
        "Vessel access expired; renew access in Vessels before creating a voyage"
    );
    let Scope::Workspaces { workspace_ids } = &connection.scope else {
        unreachable!()
    };
    let choices: Vec<_> = metadata
        .workspaces
        .iter()
        .filter(|w| workspace_ids.contains(&w.id))
        .cloned()
        .collect();
    anyhow::ensure!(
        !choices.is_empty() && choices.len() <= 128,
        "Vessel has no bounded authorized workspace list; refresh access in Vessels"
    );
    anyhow::ensure!(
        choices.iter().all(|w| w.path.is_absolute()),
        "Vessel returned a non-absolute workspace; refresh access"
    );
    Ok(choices)
}

pub(super) fn select<'a>(choices: &'a [Workspace], value: &str) -> Result<&'a Workspace> {
    let matches: Vec<_> = choices
        .iter()
        .filter(|w| w.path == PathBuf::from(value) || w.id.to_string() == value)
        .collect();
    let [workspace] = matches.as_slice() else {
        anyhow::bail!(
            "Choose exactly one server-authorized workspace using /new or its workspace UUID; arbitrary remote paths are not permitted"
        );
    };
    Ok(workspace)
}

/// Revalidate on the server immediately before the first mutation. This request
/// cannot grant creation, change a configuration profile, or import local tokens.
pub(super) async fn validate_live(client: &Client, saved: &Saved) -> Result<()> {
    if client.is_local() {
        return Ok(());
    }
    let allowed = authorized(client)?;
    let chosen = select(&allowed, &saved.workspace.to_string_lossy())?;
    let metadata = client.request(VesselCommand::Capabilities).await?;
    let connection = client.managed().context("Managed connection required")?;
    anyhow::ensure!(
        metadata["vessel_id"] == connection.vessel_id.to_string(),
        "Executing Vessel identity changed; reconnect only after reviewing new access"
    );
    if let Some(principal) = connection.principal_id {
        anyhow::ensure!(
            metadata["principal_id"] == principal.to_string(),
            "Executing-host principal changed; original draft retained"
        );
    }
    anyhow::ensure!(
        metadata["expires_at_ms"]
            .as_u64()
            .is_some_and(|expires| expires > chrono::Utc::now().timestamp_millis().max(0) as u64),
        "Executing-host access expired or omitted expiry"
    );
    anyhow::ensure!(
        metadata["scope"] == "workspaces",
        "Shared-conversation access cannot create voyages"
    );
    anyhow::ensure!(
        metadata["features"]
            .as_array()
            .is_some_and(|v| v.iter().any(|s| s == "workspace_pairing")),
        "Executing host does not advertise remote workspace creation"
    );
    anyhow::ensure!(
        metadata["rights"]
            .as_array()
            .is_some_and(|v| v.iter().any(|s| s == "create")),
        "Executing host refused creation authority"
    );
    anyhow::ensure!(
        metadata["workspaces"]
            .as_array()
            .is_some_and(|v| v.iter().any(|w| w["id"] == chosen.id.to_string()
                && w["path"] == saved.workspace.to_string_lossy().as_ref())),
        "Workspace is no longer authorized; refresh Vessel access"
    );
    anyhow::ensure!(
        matches!(saved.start, Some(VesselCommand::Start { .. })),
        "Remote creation must use executing-host configuration, not StartConfigured"
    );
    Ok(())
}

pub(in crate::process_client::ui) struct WorkspacePicker {
    route: Route,
    choices: Vec<Workspace>,
    selected: usize,
    hits: RefCell<Vec<(Rect, usize)>>,
}

impl WorkspacePicker {
    pub(super) fn new(route: Route, choices: Vec<Workspace>, preferred: Option<Uuid>) -> Self {
        let selected = preferred
            .and_then(|id| choices.iter().position(|w| w.id == id))
            .unwrap_or(0);
        Self {
            route,
            choices,
            selected,
            hits: RefCell::new(Vec::new()),
        }
    }
}

impl App {
    pub(in crate::process_client::ui) fn workspace_picker_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        let Some(picker) = self.workspace_picker.as_mut() else {
            return Ok(false);
        };
        let mut choose = false;
        match event {
            Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release => {
                match key.code {
                    KeyCode::Esc => {
                        self.workspace_picker = None;
                        return Ok(true);
                    }
                    KeyCode::Up | KeyCode::BackTab => {
                        picker.selected = picker.selected.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Tab => {
                        picker.selected =
                            (picker.selected + 1).min(picker.choices.len().saturating_sub(1))
                    }
                    KeyCode::Home => picker.selected = 0,
                    KeyCode::End => picker.selected = picker.choices.len().saturating_sub(1),
                    KeyCode::Enter => choose = true,
                    _ => {}
                }
            }
            Event::Mouse(mouse)
                if mouse.kind
                    == crossterm::event::MouseEventKind::Down(
                        crossterm::event::MouseButton::Left,
                    ) =>
            {
                if let Some((_, index)) = picker
                    .hits
                    .borrow()
                    .iter()
                    .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                {
                    picker.selected = *index;
                    choose = true;
                }
            }
            _ => {}
        }
        if choose {
            let route = picker.route;
            let workspace = picker.choices[picker.selected].clone();
            // Leave the existing draft and composer untouched on cancellation or
            // failure. Never choose a workspace solely because it is first.
            self.create_on_route(route, Some(&workspace.id.to_string()))?;
            self.workspace_picker = None;
            let preference = self
                .vessels
                .as_ref()
                .map(|manager| {
                    manager
                        .borrow_mut()
                        .remember_workspace(route.id, workspace.id)
                })
                .transpose();
            self.status = if let Err(error) = preference {
                format!(
                    "Draft saved, but workspace preference was not saved: {}",
                    safe(&error.to_string())
                )
            } else if workspace.provider_ready == Some(true) {
                "Remote draft saved in Helm. Provider settings and credentials remain on the executing host.".into()
            } else if workspace.provider_ready.is_none() {
                "Remote draft saved. Provider readiness has not yet been checked on the executing host; credentials remain there.".into()
            } else {
                "Remote draft saved. Provider sign-in is missing on the executing host; ask its owner to authorize that host. No local tokens are copied.".into()
            };
        }
        Ok(true)
    }

    pub(in crate::process_client::ui) fn draw_workspace_picker(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let Some(picker) = &self.workspace_picker else {
            return;
        };
        picker.hits.borrow_mut().clear();
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" New voyage · authorized workspaces ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        frame.render_widget(
            Paragraph::new(format!(
                "{} · ↑/↓ Enter choose · Esc cancel",
                safe(&self.route_label(picker.route))
            )),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let height = inner.height.saturating_sub(1) as usize;
        let offset = picker.selected.saturating_sub(height.saturating_sub(1));
        for (row, (index, workspace)) in picker
            .choices
            .iter()
            .enumerate()
            .skip(offset)
            .take(height)
            .enumerate()
        {
            let rect = Rect::new(inner.x, inner.y + 1 + row as u16, inner.width, 1);
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {} · {}{}",
                    if index == picker.selected { ">" } else { " " },
                    safe(&workspace.name),
                    safe(&workspace.path.display().to_string()),
                    if workspace.provider_ready == Some(true) {
                        ""
                    } else if workspace.provider_ready.is_none() {
                        " · provider readiness not yet checked"
                    } else {
                        " · host sign-in needed"
                    }
                )),
                rect,
            );
            picker.hits.borrow_mut().push((rect, index));
        }
    }
}
