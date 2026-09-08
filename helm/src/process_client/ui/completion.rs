//! Composer-only discovery. Completion edits drafts; Enter remains the dispatch boundary.
use super::{App, drafts, observe::Update, safe, state::Target};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
};
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

const COMMANDS: &[(&str, &str, &str)] = &[
    ("help", "Show help", ""),
    (
        "new",
        "Start a voyage",
        "Optional absolute workspace on the executing host",
    ),
    ("use", "Switch voyage", "Choose a voyage"),
    ("rename", "Rename this voyage", "Type a name"),
    (
        "branch",
        "Continue in a separate voyage",
        "Type an optional name",
    ),
    (
        "model",
        "Change the next-turn model",
        "Choose or type a model ID",
    ),
    (
        "thinking",
        "Change next-turn thinking",
        "Choose or type an effort; inherit clears",
    ),
    (
        "service",
        "Change next-turn service",
        "Choose or type a tier; inherit clears",
    ),
    ("models", "Show available models", ""),
    (
        "configure",
        "Load next-turn configuration",
        "Type an absolute configuration path on the executing host",
    ),
    ("tools", "Show available tools", ""),
    (
        "tool",
        "Run an operator tool",
        "Choose a tool, then type JSON arguments",
    ),
    ("policy", "Show permissions", ""),
    (
        "access",
        "Change voyage access",
        "Choose an access mode; Enter reviews the change",
    ),
    ("todos", "Show tasks", ""),
    ("subagents", "Show delegated work", ""),
    ("workflows", "Show saved workflows", ""),
    ("host_resources", "Show machine capacity", ""),
    ("terminals", "Browse program terminals", ""),
    (
        "terminal",
        "Open a program terminal",
        "Choose a terminal or press Enter to browse",
    ),
    (
        "approve",
        "Allow a pending permission request",
        "Choose a pending approval",
    ),
    (
        "deny",
        "Deny a pending permission request",
        "Choose a pending approval",
    ),
    (
        "answer",
        "Answer a pending question",
        "Choose a question, then type your answer",
    ),
    ("cancel", "Request cancellation of the active run", ""),
    ("receipt", "Check an unconfirmed command", ""),
    (
        "compact",
        "Compact older conversation messages",
        "Choose how many recent messages to retain",
    ),
    (
        "clear",
        "Clear this conversation",
        "Type this voyage's full UUID to confirm clearing history",
    ),
    (
        "archive",
        "Archive this voyage and release its process after cleanup",
        "",
    ),
    ("archived", "Browse archived voyages", ""),
    ("voyages", "Browse current voyages", ""),
    ("restore", "Restore this voyage", ""),
    (
        "delete",
        "Delete this conversation",
        "Type this voyage's full UUID to confirm deletion",
    ),
    (
        "export",
        "Save conversation as Markdown",
        "Type a destination path on this computer",
    ),
    ("conversation", "Return to the conversation", ""),
    ("quit", "Leave Helm; voyages keep running", ""),
];

#[derive(Default)]
pub(super) struct Completion {
    key: String,
    selected: usize,
    dismissed: bool,
    metadata: Option<Metadata>,
}
struct Metadata {
    target: Target,
    incarnation: Uuid,
    section: &'static str,
    value: Option<serde_json::Value>,
    failed: bool,
}
struct Menu {
    entries: Vec<(String, String)>,
    hint: String,
}

impl App {
    fn completion_text(&self) -> Option<&str> {
        let view = self.views.get(&self.selected?)?;
        if self.sidebar.menu.is_some()
            || self.sidebar.focus != super::sidebar::Focus::Composer
            || self.help
            || self.explore.is_some()
            || view.panel.is_some()
            || view.terminals.open
            || self.interactions.borrow().focused
            || view.draft.cursor != view.draft.text.len()
            || view.draft.text.contains('\n')
        {
            return None;
        }
        view.draft.text.strip_prefix('/')
    }

    pub(super) fn sync_completion(&mut self) {
        let text = self.completion_text().map(str::to_owned);
        let key = format!("{:?}:{text:?}", self.selected);
        if key != self.completion.key {
            self.completion.key = key;
            self.completion.selected = 0;
            self.completion.dismissed = false;
        }
        let section = text
            .as_deref()
            .and_then(|s| s.split_once(' '))
            .and_then(|(name, _)| match name {
                "model" => Some("models"),
                "tool" => Some("tools"),
                "terminal" => Some("terminals"),
                _ => None,
            });
        let Some(section) = section else {
            self.completion.metadata = None;
            return;
        };
        let Some(target) = self.selected else {
            return;
        };
        let view = &self.views[&target];
        let incarnation = view.process.incarnation;
        if self.completion.metadata.as_ref().is_some_and(|m| {
            m.target == target && m.incarnation == incarnation && m.section == section
        }) {
            return;
        }
        self.completion.metadata = Some(Metadata {
            target,
            incarnation,
            section,
            value: None,
            failed: false,
        });
        let run_id = view
            .snapshot
            .as_ref()
            .and_then(|s| s.run.as_ref())
            .filter(|r| r.active())
            .map(|r| r.run_id);
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                client.voyage(
                    target.session,
                    incarnation,
                    VoyageCommand::Controls {
                        run_id,
                        section: section.into(),
                    },
                ),
            )
            .await;
            let value = result.ok().and_then(Result::ok);
            let _ = sender
                .send(Update::Completion {
                    target,
                    incarnation,
                    section,
                    value,
                })
                .await;
        });
    }

    pub(super) fn completion_update(
        &mut self,
        target: Target,
        incarnation: Uuid,
        section: &str,
        value: Option<serde_json::Value>,
    ) {
        if let Some(metadata) =
            self.completion.metadata.as_mut().filter(|m| {
                m.target == target && m.incarnation == incarnation && m.section == section
            })
        {
            metadata.failed = value.is_none();
            metadata.value = value;
        }
    }

    fn completion_menu(&self) -> Option<Menu> {
        let text = self.completion_text()?;
        if self.completion.dismissed {
            return None;
        }
        let Some((command, argument)) = text.split_once(' ') else {
            return Some(Menu {
                entries: COMMANDS
                    .iter()
                    .filter(|(name, _, _)| name.starts_with(text))
                    .map(|(name, description, hint)| {
                        (
                            format!("/{name}{}", if hint.is_empty() { "" } else { " " }),
                            description.to_string(),
                        )
                    })
                    .collect(),
                hint: "No matching commands".into(),
            });
        };
        let (_, _, hint) = COMMANDS.iter().find(|(name, _, _)| *name == command)?;
        if hint.is_empty() {
            return None;
        }
        let mut menu = Menu {
            entries: Vec::new(),
            hint: (*hint).into(),
        };
        let target = self.selected?;
        let view = &self.views[&target];
        let mut options = Vec::new();
        match command {
            "thinking" | "service" => {
                options.push((
                    "inherit".into(),
                    "Catalog/provider-managed (clear override)".into(),
                ));
                if let Some(settings) = view.snapshot.as_ref().and_then(|s| s.inference.as_ref()) {
                    let choices = if command == "thinking" {
                        &settings.reasoning_efforts
                    } else {
                        &settings.service_tiers
                    };
                    options.extend(
                        choices
                            .iter()
                            .filter(|value| {
                                value.as_str() != "inherit"
                                    && safe(value) == **value
                                    && !value.chars().any(char::is_whitespace)
                            })
                            .map(|value| {
                                (
                                    value.clone(),
                                    "Explicit request; support/entitlement may be unknown".into(),
                                )
                            }),
                    );
                }
                menu.hint =
                    "Enter a value, or send the bare command to open its picker; inherit clears; service default is explicit"
                        .into();
            }
            "access" => options.extend([
                ("read-only".into(), "Read only".into()),
                ("approval".into(), "Ask first".into()),
                ("unrestricted".into(), "Unrestricted".into()),
            ]),
            "use" => {
                for key in self.ordered_targets() {
                    if self
                        .views
                        .keys()
                        .filter(|k| k.session == key.session)
                        .count()
                        == 1
                    {
                        options.push((key.session.to_string(), self.views[&key].title()));
                    }
                }
            }
            "new" | "configure" | "export" => {
                if command == "export" || self.clients[target.route].is_local() {
                    options.extend(path_options(
                        argument,
                        command == "new",
                        command != "export",
                    ));
                }
            }
            "compact" => {
                for count in [1, 10, 20, 50, 100] {
                    options.push((count.to_string(), format!("Retain {count} recent messages")));
                }
            }
            "approve" | "deny" | "answer" => {
                if let Some(snapshot) = &view.snapshot {
                    for decision in &snapshot.decisions {
                        if decision.incarnation == view.process.incarnation
                            && decision.request["kind"].as_str()
                                == Some(if command == "answer" {
                                    "question"
                                } else {
                                    "approval"
                                })
                        {
                            options.push((
                                format!(
                                    "{}{}",
                                    decision.decision_id,
                                    if command == "answer" { " " } else { "" }
                                ),
                                if command == "answer" {
                                    "Pending question"
                                } else {
                                    "Pending approval"
                                }
                                .into(),
                            ));
                        }
                    }
                }
            }
            "model" | "tool" | "terminal" => {
                if let Some(metadata) = self
                    .completion
                    .metadata
                    .as_ref()
                    .filter(|m| m.target == target && m.incarnation == view.process.incarnation)
                {
                    if let Some(value) = &metadata.value {
                        let value = value.get("value").unwrap_or(value);
                        let value = value.get("inventory").unwrap_or(value);
                        if let Some(entries) = value.as_array() {
                            for entry in entries.iter().take(256) {
                                let key = if command == "tool" { "name" } else { "id" };
                                if let Some(id) = entry[key].as_str().filter(|id| {
                                    !id.chars().any(char::is_whitespace) && safe(id) == *id
                                }) {
                                    let description = entry[if command == "model" {
                                        "display_name"
                                    } else if command == "terminal" {
                                        "title"
                                    } else {
                                        "description"
                                    }]
                                    .as_str()
                                    .unwrap_or(id);
                                    options.push((
                                        format!("{id}{}", if command == "tool" { " " } else { "" }),
                                        safe(description),
                                    ));
                                }
                            }
                        }
                    } else {
                        menu.hint = if metadata.failed {
                            format!(
                                "Options unavailable. {hint}; erase the space and retry to reload."
                            )
                        } else {
                            format!("Loading options… {hint}")
                        };
                    }
                }
                if command == "model"
                    && let Some(snapshot) = &view.snapshot
                    && !options.iter().any(|(id, _)| *id == snapshot.model)
                {
                    options.push((snapshot.model.clone(), "Current model".into()));
                }
                if command == "tool" && argument.contains(' ') {
                    menu.hint =
                        "Type JSON arguments for the selected tool; /tools shows the inventory"
                            .into();
                }
            }
            // Destructive confirmations are deliberately typed, never inserted by completion.
            "clear" | "delete" => menu.hint = format!("{hint}: {}", target.session),
            _ => {}
        }
        menu.entries = options
            .into_iter()
            .filter(|(value, _)| value.starts_with(argument) && value != argument)
            .map(|(value, description)| (format!("/{command} {value}"), description))
            .collect();
        Some(menu)
    }

    pub(super) fn completion_input(&mut self, key: &KeyEvent) -> Result<bool> {
        if self
            .selected
            .and_then(|t| self.views.get(&t))
            .is_some_and(|v| !v.images.is_empty())
        {
            return Ok(false);
        }
        let Some(menu) = self.completion_menu() else {
            return Ok(false);
        };
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Ok(false);
        }
        let count = menu.entries.len();
        match key.code {
            KeyCode::Esc => self.completion.dismissed = true,
            KeyCode::Up | KeyCode::Down | KeyCode::BackTab => {
                if count > 0 {
                    self.completion.selected = if key.code == KeyCode::Down {
                        (self.completion.selected + 1) % count
                    } else {
                        (self.completion.selected.min(count - 1) + count - 1) % count
                    };
                }
            }
            KeyCode::Tab => {
                if let Some((text, _)) = menu
                    .entries
                    .get(self.completion.selected.min(count.saturating_sub(1)))
                {
                    let target = self.selected.expect("completion target");
                    let view = self.views.get_mut(&target).expect("completion view");
                    view.draft.set_text(text.clone());
                    drafts::save(&self.clients[target.route], view)?;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn completion_height(&self) -> u16 {
        self.completion_menu()
            .map_or(0, |m| (m.entries.len().clamp(1, 6) + 2) as u16)
    }

    pub(super) fn draw_completion(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(menu) = self.completion_menu() else {
            return;
        };
        let entries = if menu.entries.is_empty() {
            vec![ListItem::new(safe(&menu.hint))]
        } else {
            menu.entries
                .iter()
                .map(|(text, description)| {
                    ListItem::new(format!("{}  {}", safe(text), safe(description)))
                })
                .collect()
        };
        let selected = (!menu.entries.is_empty())
            .then(|| self.completion.selected.min(menu.entries.len() - 1));
        frame.render_stateful_widget(
            List::new(entries)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Commands · ↑↓ select · Tab complete · Esc close "),
                )
                .highlight_style(Style::default().fg(Color::Cyan))
                .highlight_symbol("> "),
            area,
            &mut ListState::default().with_selected(selected),
        );
    }
}

// Only inspect this computer's paths. Remote paths remain explicit host-side input.
fn path_options(argument: &str, directories_only: bool, absolute: bool) -> Vec<(String, String)> {
    let path = std::path::Path::new(argument);
    let (directory, prefix) = if argument.is_empty() {
        (
            std::env::current_dir().unwrap_or_else(|_| "/".into()),
            String::new(),
        )
    } else if argument.ends_with('/') {
        (path.to_path_buf(), String::new())
    } else {
        (
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf(),
            path.file_name()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_owned(),
        )
    };
    if absolute && !argument.is_empty() && !path.is_absolute() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut options = entries
        .take(2048)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(&prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            if directories_only && !is_dir {
                return None;
            }
            let value = if argument.is_empty() || directory != std::path::Path::new(".") {
                entry.path().to_str()?.to_owned()
            } else {
                name
            };
            if safe(&value) != value {
                return None;
            }
            Some((
                format!("{value}{}", if is_dir { "/" } else { "" }),
                if is_dir { "Directory" } else { "File" }.into(),
            ))
        })
        .collect::<Vec<_>>();
    options.sort();
    options.truncate(256);
    options
}
