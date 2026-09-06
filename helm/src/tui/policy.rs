//! Interactive policy review owns no execution authority or live runtime resources.
use super::{App, UiEvent, text::display_safe};
use crate::policy_profile::{
    EffectivePolicy,
    store::ProfileSnapshot,
    switching::{SwitchContext, SwitchPreview, SwitchRequest, Target},
};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
    request: Uuid,
    context: Option<SwitchContext>,
    directory: Option<PathBuf>,
    profiles: Vec<ProfileSnapshot>,
    selected: usize,
    preview: Option<(Target, SwitchPreview)>,
    pending: bool,
    current_only: bool,
    error: Option<String>,
    scroll: u16,
}
impl Panel {
    pub fn configure(&mut self, context: Option<SwitchContext>) {
        self.context = context;
    }
    pub fn open(
        &mut self,
        directory: Option<PathBuf>,
        tx: &mpsc::UnboundedSender<UiEvent>,
    ) -> anyhow::Result<()> {
        let context = self
            .context
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Policy switching unavailable in this frontend"))?;
        self.open = true;
        self.request = Uuid::new_v4();
        self.profiles.clear();
        self.preview = None;
        self.current_only = false;
        self.selected = 0;
        self.scroll = 0;
        self.error = None;
        self.pending = true;
        let request = self.request;
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                crate::policy_profile::switching::read_sources(|| {
                    let directory = directory
                        .clone()
                        .map(Ok)
                        .unwrap_or_else(|| context.default_directory())?;
                    let profiles = context.profiles(&directory)?;
                    Ok::<_, anyhow::Error>((directory, profiles))
                })
            })
            .await
            .map_err(|_| "Policy source reader failed".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = tx.send(UiEvent::PolicyProfiles { request, result });
        });
        Ok(())
    }
    pub fn profiles(
        &mut self,
        request: Uuid,
        result: Result<(PathBuf, Vec<ProfileSnapshot>), String>,
    ) {
        if !self.open || request != self.request {
            return;
        }
        self.pending = false;
        match result {
            Ok((directory, profiles)) => {
                self.directory = Some(directory);
                self.profiles = profiles;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn preview(&mut self, request: Uuid, result: Result<(Target, Box<SwitchPreview>), String>) {
        if !self.open || request != self.request {
            return;
        }
        self.pending = false;
        match result {
            Ok((target, preview)) => {
                self.preview = Some((target, *preview));
                self.scroll = 0;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn failed(&mut self, message: String) {
        self.error = Some(message);
        self.scroll = 0;
    }
    pub fn key(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<UiEvent>,
    ) -> Option<SwitchRequest> {
        if key.code == KeyCode::Esc {
            self.request = Uuid::new_v4();
            self.pending = false;
            self.error = None;
            if self.current_only {
                self.current_only = false;
            } else if self.preview.take().is_none() {
                self.open = false;
            }
            self.scroll = 0;
            return None;
        }
        if key.code == KeyCode::Char('i') && self.preview.is_none() {
            self.current_only = true;
            self.scroll = 0;
            return None;
        }
        if self.current_only {
            match key.code {
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                KeyCode::Home => self.scroll = 0,
                _ => {}
            }
            return None;
        }
        if self.pending {
            return None;
        }
        if let Some((target, preview)) = &self.preview {
            match key.code {
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                KeyCode::Home => self.scroll = 0,
                KeyCode::Char('y' | 'Y') if preview.requires_confirmation => {
                    return Some(SwitchRequest {
                        target: target.clone(),
                        preview_digest: preview.digest.clone(),
                        confirmation: Some(preview.digest.clone()),
                    });
                }
                KeyCode::Enter if !preview.requires_confirmation => {
                    return Some(SwitchRequest {
                        target: target.clone(),
                        preview_digest: preview.digest.clone(),
                        confirmation: None,
                    });
                }
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(self.profiles.len()),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.profiles.len(),
            KeyCode::Enter if self.error.is_none() => {
                let context = self.context.clone()?;
                let target = if self.selected == 0 {
                    Ok(Target::Defaults)
                } else {
                    context.target(
                        self.directory.clone()?,
                        self.profiles.get(self.selected - 1)?,
                    )
                };
                let target = match target {
                    Ok(target) => target,
                    Err(error) => {
                        self.error = Some(error.to_string());
                        return None;
                    }
                };
                self.request = Uuid::new_v4();
                self.pending = true;
                let request = self.request;
                let tx = tx.clone();
                tokio::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        crate::policy_profile::switching::read_sources(|| context.preview(&target))
                            .map(|preview| (target, Box::new(preview)))
                    })
                    .await
                    .map_err(|_| "Policy preview reader failed".to_owned())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                    let _ = tx.send(UiEvent::PolicyPreview { request, result });
                });
            }
            _ => {}
        }
        None
    }
}
fn effective(label: &str, policy: &EffectivePolicy) -> String {
    format!(
        "{label}\n{}\nProvenance\n{}\n",
        serde_json::to_string_pretty(policy.rules()).unwrap_or_default(),
        serde_json::to_string_pretty(policy.provenance()).unwrap_or_default()
    )
}
pub(super) fn draw(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let panel = &app.policy_panel;
    let mut text = String::new();
    if let Some(error) = &panel.error {
        text.push_str(&format!("{error}\n\n"));
    }
    let mut footer = "↑↓ select · Enter review · I current policy · Esc close";
    let scroll;
    if panel.current_only {
        if let Some(context) = &panel.context {
            text.push_str(&effective("CURRENT EFFECTIVE POLICY", context.current()));
        }
        footer = "↑↓/PgUp/PgDn scroll · Home top · Esc back";
        scroll = panel.scroll;
    } else if let Some((target, preview)) = &panel.preview {
        text.push_str(if preview.requires_confirmation {
            "AUTHORITY INCREASE — explicit confirmation required\n"
        } else {
            "Review proposed policy\n"
        });
        match target {
            Target::Profile(request) => text.push_str(&format!(
                "Selected: {} · revision {} · store {}\n",
                request.name,
                request.revision,
                request.directory.display()
            )),
            Target::Defaults => text.push_str("Selected: launch defaults for this workspace\n"),
        }
        text.push_str(&format!("Review receipt: {}\n", preview.digest));
        text.push_str("Effective changes (current → proposed)\n");
        if let (Ok(previous), Ok(proposed)) = (
            serde_json::to_value(preview.previous.rules()),
            serde_json::to_value(preview.proposed.rules()),
        ) && let (Some(previous), Some(proposed)) = (previous.as_object(), proposed.as_object())
        {
            for (field, value) in proposed {
                let old = &previous[field];
                if old == value {
                    text.push_str(&format!("{field}: unchanged {value}\n"));
                } else {
                    text.push_str(&format!("{field}: {old} → {value}\n"));
                }
            }
        }
        text.push_str(&effective("PROPOSED EFFECTIVE POLICY", &preview.proposed));
        text.push_str(&effective("CURRENT EFFECTIVE POLICY", &preview.previous));
        if preview.baseline.digest() != preview.previous.digest() {
            text.push_str(&effective(
                "LAUNCH CONFIGURATION BASELINE",
                &preview.baseline,
            ));
        }
        footer = if preview.requires_confirmation {
            "Y confirms this exact increase · ↑↓/PgUp/PgDn scroll · Esc cancel"
        } else {
            "Enter apply · ↑↓/PgUp/PgDn scroll · Esc cancel"
        };
        scroll = panel.scroll;
    } else {
        text.push_str("Policy selection applies only to this workspace runtime.\n");
        if let Some(directory) = &panel.directory {
            text.push_str(&format!("Store: {}\n", directory.display()));
        }
        if let Some(context) = &panel.context {
            text.push_str(&format!(
                "Current access: {:?}\n",
                context.current().rules().access
            ));
        }
        text.push_str(&format!(
            "{} Use launch defaults\n",
            if panel.selected == 0 { "›" } else { " " }
        ));
        for (index, profile) in panel.profiles.iter().enumerate() {
            text.push_str(&format!(
                "{} {} · revision {}\n",
                if index + 1 == panel.selected {
                    "›"
                } else {
                    " "
                },
                profile.name,
                profile.revision
            ));
        }
        scroll = panel
            .selected
            .saturating_sub(area.height.saturating_sub(7) as usize)
            .min(u16::MAX as usize) as u16;
    }
    if panel.pending {
        text.push_str("Reading policy sources…\n");
    }
    let safe = |text: &str| {
        display_safe(
            &app.diagnostic_agent
                .as_ref()
                .map_or_else(|| text.to_owned(), |agent| agent.redact_diagnostic(text)),
        )
    };
    let block = Block::default()
        .title(" Policy ")
        .title_bottom(footer)
        .borders(Borders::ALL);
    frame.render_widget(
        Paragraph::new(safe(&text))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0))
            .block(block),
        area,
    );
}
