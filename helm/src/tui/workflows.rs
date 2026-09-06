//! Saved workflow discovery and isolated transient input. Execution stays in `start_run`.

use super::{UiEvent, text::display_safe};
use crate::workflow::{self, Definition, Prepared, Scope};
use anyhow::{Result, ensure};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Clone, Default)]
pub(super) struct FormEditor {
    pub(super) text: Zeroizing<String>,
    pub(super) cursor: usize,
}

impl FormEditor {
    pub(super) fn insert_str(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
    }

    pub(super) fn delete(&mut self) {
        if let Some(character) = self.text[self.cursor..].chars().next() {
            self.text
                .drain(self.cursor..self.cursor + character.len_utf8());
        }
    }

    pub(super) fn line_start(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map_or(0, |index| index + 1);
    }

    pub(super) fn line_end(&mut self) {
        self.cursor += self.text[self.cursor..]
            .find('\n')
            .unwrap_or(self.text.len() - self.cursor);
    }
}

pub(super) struct Form {
    definition: Definition,
    fields: BTreeMap<String, Option<FormEditor>>,
    selected: usize,
    preview: Option<String>,
    trusted: bool,
    scroll: u16,
}

impl Form {
    pub(super) fn new(definition: Definition) -> Result<Self> {
        let fields = definition
            .document
            .parameters
            .keys()
            .map(|k| (k.clone(), None))
            .collect();
        Ok(Self {
            definition,
            fields,
            selected: 0,
            preview: None,
            trusted: false,
            scroll: 0,
        })
    }

    fn supplied(&self) -> Vec<(String, String)> {
        self.fields
            .iter()
            .filter(|(k, _)| !self.definition.document.parameters[*k].secret)
            .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.text.to_string())))
            .collect()
    }

    fn secrets(&self) -> Result<workflow::secrets::SecretInputs> {
        workflow::secrets::SecretInputs::collect(
            &self.definition.document,
            self.fields
                .iter()
                .filter(|(k, _)| self.definition.document.parameters[*k].secret)
                .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.text.to_string())))
                .collect(),
        )
    }

    pub(super) fn set_input(&mut self, name: &str, text: &str) -> Result<()> {
        ensure!(text.len() <= 8192, "Workflow input exceeds 8192 bytes");
        let field = self
            .fields
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown workflow input"))?;
        *field = Some(FormEditor {
            text: Zeroizing::new(text.into()),
            cursor: text.len(),
        });
        self.preview = None;
        Ok(())
    }

    pub(super) fn unset_input(&mut self, name: &str) -> Result<()> {
        *self
            .fields
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown workflow input"))? = None;
        self.preview = None;
        Ok(())
    }

    pub(super) fn render_preview(&mut self) -> Result<()> {
        self.preview = None;
        self.preview = Some(
            workflow::secrets::render_public(
                &self.definition.document,
                &self.supplied(),
                &self.secrets()?.names(),
            )?
            .prompt,
        );
        self.scroll = 0;
        Ok(())
    }

    pub(super) fn trust_current_digest(&mut self) {
        self.trusted = true;
    }

    pub(super) fn verify_definition(&self, current: &Definition) -> Result<()> {
        ensure!(
            self.definition.scope == current.scope
                && self.definition.document.id == current.document.id
                && self.definition.digest == current.digest,
            "Workflow changed; reopen it and review the new definition and digest"
        );
        Ok(())
    }

    pub(super) fn prepare(&self) -> Result<Prepared> {
        self.definition
            .authorize(self.trusted.then_some(self.definition.digest.as_str()))?;
        let secrets = self.secrets()?;
        let rendered = workflow::secrets::render_public(
            &self.definition.document,
            &self.supplied(),
            &secrets.names(),
        )?;
        Ok(Prepared {
            input_monitor: None,
            prompt: rendered.prompt,
            invocation: self.definition.invocation(rendered.inputs),
            no_save: false,
            secrets,
        })
    }

    fn selected_name(&self) -> Option<String> {
        self.fields.keys().nth(self.selected).cloned()
    }

    fn insert(&mut self, text: &str) -> Result<()> {
        if self.preview.is_some() {
            return Ok(());
        }
        if let Some(name) = self.selected_name() {
            let mut edit = self.fields[&name].clone().unwrap_or_default();
            ensure!(
                edit.text.len().saturating_add(text.len()) <= 8192,
                "Workflow input exceeds 8192 bytes"
            );
            edit.insert_str(text);
            self.set_input(&name, &edit.text)?;
            self.fields.insert(name, Some(edit));
        }
        Ok(())
    }

    fn key(&mut self, key: KeyEvent) -> Result<Action> {
        if self.preview.is_some() {
            match key.code {
                KeyCode::Esc => self.preview = None,
                KeyCode::Char('t') if key.modifiers.is_empty() => self.trust_current_digest(),
                KeyCode::Char('r') if key.modifiers.is_empty() => {
                    self.prepare()?;
                    return Ok(Action::Verify);
                }
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
                _ => {}
            }
            return Ok(Action::None);
        }
        match key.code {
            KeyCode::Esc => return Ok(Action::Close),
            KeyCode::Tab => self.selected = (self.selected + 1) % self.fields.len().max(1),
            KeyCode::BackTab => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.fields.len().saturating_sub(1))
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => self.insert("\n")?,
            KeyCode::Enter => self.render_preview()?,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(name) = self.selected_name() {
                    self.unset_input(&name)?;
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(c.encode_utf8(&mut [0; 4]))?
            }
            KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End => {
                if let Some(name) = self.selected_name() {
                    if self.fields[&name].is_none()
                        && matches!(
                            key.code,
                            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
                        )
                    {
                        return Ok(Action::None);
                    }
                    let edit = self
                        .fields
                        .get_mut(&name)
                        .expect("selected field")
                        .get_or_insert_with(FormEditor::default);
                    match key.code {
                        KeyCode::Backspace => edit.backspace(),
                        KeyCode::Delete => edit.delete(),
                        KeyCode::Left => {
                            edit.cursor = edit.text[..edit.cursor]
                                .char_indices()
                                .next_back()
                                .map_or(0, |(i, _)| i)
                        }
                        KeyCode::Right => {
                            edit.cursor += edit.text[edit.cursor..]
                                .chars()
                                .next()
                                .map_or(0, char::len_utf8)
                        }
                        KeyCode::Home => edit.line_start(),
                        KeyCode::End => edit.line_end(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        Ok(Action::None)
    }
}

enum Mode {
    Loading(Option<(String, Option<Scope>)>),
    Picker,
    Form(Box<Form>),
}
enum Action {
    None,
    Close,
    Verify,
}

#[derive(Default)]
pub(super) struct Panel {
    mode: Option<Mode>,
    definitions: Vec<Definition>,
    selected: usize,
    pending: Option<Uuid>,
    pub(super) ready: Option<Prepared>,
    notice: String,
}

impl Panel {
    pub(super) fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    pub(super) fn close(&mut self) {
        *self = Self::default();
    }

    pub(super) fn open(
        &mut self,
        argument: &str,
        workspace: PathBuf,
        tx: &mpsc::UnboundedSender<UiEvent>,
    ) -> Result<()> {
        let words = shell_words::split(argument)?;
        let selection = match words.as_slice() {
            [] => None,
            [id] => Some((id.clone(), None)),
            [id, flag, scope] if flag == "--scope" => Some((
                id.clone(),
                Some(match scope.as_str() {
                    "user" => Scope::User,
                    "repository" => Scope::Repository,
                    _ => anyhow::bail!("Workflow scope must be user or repository"),
                }),
            )),
            _ => anyhow::bail!("Usage: /workflow [ID [--scope user|repository]]"),
        };
        self.close();
        self.mode = Some(Mode::Loading(selection));
        self.discover(workspace, tx);
        Ok(())
    }

    fn discover(&mut self, workspace: PathBuf, tx: &mpsc::UnboundedSender<UiEvent>) {
        let request = Uuid::new_v4();
        self.pending = Some(request);
        self.notice = "Reading workflow definitions… Esc cancels".into();
        let tx = tx.clone();
        tokio::spawn(async move {
            let read = tokio::task::spawn_blocking(move || workflow::discover(&workspace, None));
            // Read-only work may outlive a timeout; the request ID rejects every late result.
            let definitions = match tokio::time::timeout(Duration::from_secs(5), read).await {
                Ok(Ok(result)) => result.map_err(|e| e.to_string()),
                Ok(Err(_)) => Err("Workflow discovery worker failed".into()),
                Err(_) => Err("Workflow discovery timed out".into()),
            };
            let _ = tx.send(UiEvent::Workflows {
                request,
                definitions,
            });
        });
    }

    pub(super) fn discovered(
        &mut self,
        request: Uuid,
        definitions: Result<Vec<Definition>, String>,
    ) {
        if self.pending != Some(request) {
            return;
        }
        self.pending = None;
        let result = self.apply_discovery(definitions);
        if let Err(error) = result {
            self.notice = display_safe(&error.to_string());
        }
    }

    fn apply_discovery(&mut self, definitions: Result<Vec<Definition>, String>) -> Result<()> {
        let definitions = definitions.map_err(anyhow::Error::msg)?;
        if let Some(Mode::Form(form)) = &mut self.mode {
            let current = workflow::select(
                definitions,
                &form.definition.document.id,
                Some(form.definition.scope),
            );
            let verified = current.and_then(|d| form.verify_definition(&d));
            if let Err(error) = verified {
                form.trusted = false;
                return Err(error);
            }
            self.ready = Some(form.prepare()?);
        } else {
            let selection = match self.mode.take() {
                Some(Mode::Loading(selection)) => selection,
                other => {
                    self.mode = other;
                    return Ok(());
                }
            };
            self.definitions = definitions;
            self.mode = Some(Mode::Picker);
            if let Some((id, scope)) = selection {
                let d = workflow::select(self.definitions.clone(), &id, scope)?;
                self.mode = Some(Mode::Form(Box::new(Form::new(d)?)));
            }
        }
        self.notice.clear();
        Ok(())
    }

    pub(super) fn paste(&mut self, text: &str) {
        if self.pending.is_some() {
            return;
        }
        if let Some(Mode::Form(form)) = &mut self.mode
            && let Err(error) = form.insert(text)
        {
            self.notice = error.to_string();
        }
    }

    pub(super) fn key(
        &mut self,
        key: KeyEvent,
        workspace: PathBuf,
        tx: &mpsc::UnboundedSender<UiEvent>,
    ) {
        if key.code == KeyCode::Esc && self.pending.is_some() {
            self.close();
            return;
        }
        if self.pending.is_some() {
            return;
        }
        let result = match &mut self.mode {
            Some(Mode::Form(form)) => form.key(key),
            Some(Mode::Picker) => match key.code {
                KeyCode::Esc => Ok(Action::Close),
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    Ok(Action::None)
                }
                KeyCode::Down => {
                    self.selected =
                        (self.selected + 1).min(self.definitions.len().saturating_sub(1));
                    Ok(Action::None)
                }
                KeyCode::Enter => self
                    .definitions
                    .get(self.selected)
                    .cloned()
                    .map(Form::new)
                    .transpose()
                    .map(|form| {
                        if let Some(form) = form {
                            self.mode = Some(Mode::Form(Box::new(form)));
                        }
                        Action::None
                    }),
                _ => Ok(Action::None),
            },
            _ if key.code == KeyCode::Esc => Ok(Action::Close),
            _ => Ok(Action::None),
        };
        match result {
            Ok(Action::Close) => self.close(),
            Ok(Action::Verify) => self.discover(workspace, tx),
            Ok(Action::None) => self.notice.clear(),
            Err(error) => self.notice = display_safe(&error.to_string()),
        }
    }

    pub(super) fn rejected(&mut self, notice: &str) {
        self.notice = notice.into();
    }

    pub(super) fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect) {
        if let Some(Mode::Form(form)) = &self.mode
            && form.preview.is_none()
        {
            self.draw_input(frame, area, form);
            return;
        }
        let mut lines = vec![
            "Saved workflows · nonsecret inputs are visible to the model and saved session"
                .to_owned(),
        ];
        let mut scroll = 0;
        match &self.mode {
            Some(Mode::Picker) => {
                lines.push("↑/↓ select · Enter open · Esc cancel".into());
                if self.definitions.is_empty() {
                    lines.push("No saved workflows in user or .helm/workflows directories".into());
                }
                for (i, d) in self.definitions.iter().enumerate() {
                    lines.push(format!(
                        "{} {} [{:?}] v{} · {}",
                        if i == self.selected { ">" } else { " " },
                        d.document.id,
                        d.scope,
                        d.document.version,
                        d.document.description
                    ));
                }
                scroll = self
                    .selected
                    .saturating_sub(area.height.saturating_sub(7) as usize)
                    as u16;
            }
            Some(Mode::Form(form)) => {
                let d = &form.definition;
                lines.push(format!(
                    "{} [{:?}] v{} · {}",
                    d.document.id, d.scope, d.document.version, d.document.description
                ));
                lines.push(format!("SHA-256: {}", d.digest));
                lines.push("Recommendations are advisory; active provider, model and policy stay in force.".into());
                if let Some(preview) = &form.preview {
                    lines.push(if d.scope == Scope::Repository && !form.trusted {
                        "Repository definition: press t to trust this exact digest; r runs after rechecking it."
                    } else { "Ready for review: r runs · Esc edits · ↑/↓ scroll" }.into());
                    lines.push(preview.clone());
                    scroll = form.scroll;
                }
            }
            _ => {}
        }
        let chunks = ratatui::layout::Layout::vertical([
            ratatui::layout::Constraint::Min(1),
            ratatui::layout::Constraint::Length(3),
        ])
        .split(area);
        frame.render_widget(
            Paragraph::new(display_safe(&lines.join("\n")))
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(Block::default().title(" Workflows ").borders(Borders::ALL)),
            chunks[0],
        );
        frame.render_widget(
            Paragraph::new(display_safe(&self.notice)).wrap(Wrap { trim: false }),
            chunks[1],
        );
    }

    fn draw_input(&self, frame: &mut ratatui::Frame<'_>, area: Rect, form: &Form) {
        use ratatui::layout::{Constraint, Layout};
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(4),
        ])
        .split(area);
        let selected = form.fields.iter().nth(form.selected);
        let Some((name, field)) = selected else {
            frame.render_widget(
                Paragraph::new(format!(
                    "No inputs required. Enter previews the workflow; Esc cancels.\n{}",
                    display_safe(&self.notice)
                ))
                .wrap(Wrap { trim: false })
                .block(Block::default().title(" Workflows ").borders(Borders::ALL)),
                area,
            );
            return;
        };
        let p = &form.definition.document.parameters[name];
        frame.render_widget(
            Paragraph::new(display_safe(&format!(
                "Input {}/{}: {} ({:?}{})\n{} [{:?}] · {}",
                form.selected + 1,
                form.fields.len(),
                name,
                p.kind,
                if p.required { ", required" } else { "" },
                form.definition.document.id,
                form.definition.scope,
                if p.secret {
                    "secret, hidden; one-shot shell only"
                } else {
                    "nonsecret, model-visible"
                }
            )))
            .wrap(Wrap { trim: false }),
            chunks[0],
        );
        let (value, cursor) = if let Some(edit) = field {
            let value = if p.secret {
                "[hidden]".into()
            } else {
                display_safe(&edit.text).replace('\t', " ")
            };
            let prefix = if p.secret {
                String::new()
            } else {
                display_safe(&edit.text[..edit.cursor]).replace('\t', " ")
            };
            let cursor =
                super::composer::cursor_position(&prefix, chunks[1].width.saturating_sub(2));
            (value, cursor)
        } else {
            (
                format!(
                    "<unset; default {}>",
                    p.default
                        .as_ref()
                        .map_or("null".into(), ToString::to_string)
                ),
                (0, 0),
            )
        };
        let height = chunks[1].height.saturating_sub(2);
        let scroll = cursor.0.saturating_sub(height.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(value)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .block(Block::default().title(" Value ").borders(Borders::ALL)),
            chunks[1],
        );
        if chunks[1].width > 2 && height > 0 && self.pending.is_none() {
            frame.set_cursor_position((
                chunks[1].x + 1 + cursor.1,
                chunks[1].y + 1 + cursor.0.saturating_sub(scroll),
            ));
        }
        let notice = if self.notice.is_empty() {
            format!(
                "{} · choices={:?} min={:?} max={:?} length={:?}",
                p.description, p.choices, p.minimum, p.maximum, p.max_length
            )
        } else {
            self.notice.clone()
        };
        frame.render_widget(Paragraph::new(display_safe(&format!("Tab/Shift-Tab: field · Enter: preview\nCtrl-U: unset · Shift-Enter: newline · Esc: cancel\n{notice}"))).wrap(Wrap {trim:false}),chunks[2]);
    }
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;
