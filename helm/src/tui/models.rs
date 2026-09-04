//! Model discovery, selection and picker rendering.

use super::{bridge::UiEvent, composer::Composer};
use crate::{
    Agent,
    provider::ModelInfo,
    session::{Session, SessionStore},
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct ModelPanel {
    pub(super) model_picker: bool,
    pub(super) models: Vec<ModelInfo>,
    pub(super) selected_model: usize,
    pub(super) model_filter: Composer,
    pub(super) model_manual: bool,
}

pub(super) fn filtered_models(panel: &ModelPanel) -> Vec<&ModelInfo> {
    let query = panel.model_filter.text.trim().to_ascii_lowercase();
    panel
        .models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.id.to_ascii_lowercase().contains(&query)
                || model.display_name.to_ascii_lowercase().contains(&query)
                || model.description.to_ascii_lowercase().contains(&query)
        })
        .collect()
}

pub(super) async fn handle_model_key(
    key: KeyEvent,
    panel: &mut ModelPanel,
    session: &mut Session,
    status: &mut String,
    agent: &Arc<Agent>,
    store: &SessionStore,
    tx: &mpsc::UnboundedSender<UiEvent>,
) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            panel.model_picker = false;
            panel.model_filter = Composer::default();
        }
        KeyCode::Tab => {
            panel.model_manual = !panel.model_manual;
            panel.selected_model = 0;
            *status = if panel.model_manual {
                "Manual model ID · type an ID and press Enter".into()
            } else {
                "Model search".into()
            };
        }
        KeyCode::Up if !panel.model_manual => {
            panel.selected_model = panel.selected_model.saturating_sub(1)
        }
        KeyCode::Down if !panel.model_manual => {
            let count = filtered_models(panel).len();
            panel.selected_model = (panel.selected_model + 1).min(count.saturating_sub(1));
        }
        KeyCode::Enter => {
            let selected = if panel.model_manual {
                (!panel.model_filter.text.trim().is_empty())
                    .then(|| panel.model_filter.text.trim().to_owned())
            } else {
                filtered_models(panel)
                    .get(panel.selected_model)
                    .map(|model| model.id.clone())
            };
            if let Some(model) = selected {
                session.switch_model(&model)?;
                agent.set_model(&model)?;
                store.save(session).await?;
                *status = format!("Model switched to {model}");
                panel.model_picker = false;
                panel.model_filter = Composer::default();
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            request_models(tx, agent.clone(), true)
        }
        KeyCode::Char(character) => {
            panel.model_filter.insert(character);
            panel.selected_model = 0;
        }
        KeyCode::Backspace => {
            panel.model_filter.backspace();
            panel.selected_model = 0;
        }
        KeyCode::Delete => panel.model_filter.delete(),
        KeyCode::Left if panel.model_filter.cursor > 0 => {
            panel.model_filter.cursor = panel.model_filter.text[..panel.model_filter.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index);
        }
        KeyCode::Right => {
            if let Some(character) = panel.model_filter.text[panel.model_filter.cursor..]
                .chars()
                .next()
            {
                panel.model_filter.cursor += character.len_utf8();
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn request_models(
    tx: &mpsc::UnboundedSender<UiEvent>,
    agent: Arc<Agent>,
    refresh: bool,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        let result = agent
            .models(refresh)
            .await
            .map_err(|error| error.to_string());
        let _ = tx.send(UiEvent::Models(result));
    });
}

pub(super) fn draw_model_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    panel: &ModelPanel,
    current_model: &str,
) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " HELM MODELS ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  current: {}", current_model)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(panel.model_filter.text.as_str()).block(
            Block::default()
                .title(if panel.model_manual {
                    " Manual model ID "
                } else {
                    " Search models "
                })
                .borders(Borders::ALL),
        ),
        chunks[1],
    );
    let models = filtered_models(panel);
    let visible = chunks[2].height.saturating_sub(2).max(1) as usize;
    let start = panel
        .selected_model
        .saturating_sub(visible.saturating_sub(1))
        .min(models.len().saturating_sub(visible));
    let items = if panel.model_manual {
        vec![ListItem::new("Press Enter to use this exact model ID")]
    } else if models.is_empty() {
        vec![ListItem::new(
            "No matching discovered models · Tab enters an ID manually",
        )]
    } else {
        models
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, model)| {
                ListItem::new(format!(
                    "{}{} {} · {}",
                    if index == panel.selected_model {
                        "▶ "
                    } else {
                        "  "
                    },
                    if model.is_default { "★" } else { " " },
                    model.display_name,
                    model.id
                ))
            })
            .collect()
    };
    frame.render_widget(
        List::new(items).block(Block::default().title(" Available ").borders(Borders::ALL)),
        chunks[2],
    );
    frame.render_widget(
        Paragraph::new(if area.width < 60 {
            "type filter · ↑↓ Enter · Tab manual · Esc"
        } else {
            "type to search · ↑↓ select · Enter switch · Tab manual ID · Ctrl+R refresh · Esc return"
        })
        .style(Style::default().fg(Color::Gray)),
        chunks[3],
    );
    frame.set_cursor_position((
        (chunks[1].x + 1 + panel.model_filter.cursor as u16)
            .min(chunks[1].right().saturating_sub(2)),
        chunks[1].y + 1,
    ));
}
