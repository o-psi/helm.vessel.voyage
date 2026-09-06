//! Slash command metadata, contextual completion and palette rendering.

use super::text::one_line;
use crate::{
    config::{CONFIG_OVERRIDE_SPECS, ConfigValueKind},
    provider::ModelInfo,
    session::Session,
};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::{Path, PathBuf};

/// Read-only inputs for completion. No session mutation, tools or modal state.
pub(super) struct PaletteContext<'a> {
    pub(super) input: &'a str,
    pub(super) running: bool,
    pub(super) dismissed: bool,
    pub(super) selected: usize,
    pub(super) models: &'a [ModelInfo],
    pub(super) sessions: &'a [Session],
    pub(super) workspace: &'a Path,
}

#[derive(Default)]
pub(super) struct PaletteState {
    pub(super) selected_slash_command: usize,
    pub(super) slash_palette_dismissed: bool,
    pub(super) slash_models_requested: bool,
}

#[derive(Clone, Copy)]
pub(super) struct SlashCommand {
    pub(super) name: &'static str,
    pub(super) usage: &'static str,
    pub(super) description: &'static str,
    pub(super) completion: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SlashPaletteItem {
    pub(super) usage: String,
    pub(super) description: String,
    pub(super) completion: String,
}

pub(super) const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "inference",
        usage: "/inference",
        description: "Inspect local session/project inference allowances",
        completion: "/inference",
    },
    SlashCommand {
        name: "github",
        usage: "/github COMMAND [ARGUMENTS]",
        description: "Observe GitHub, retain references, import feedback or review exact publication",
        completion: "/github ",
    },
    SlashCommand {
        name: "policy",
        usage: "/policy [DIRECTORY]",
        description: "Review and switch workspace runtime policy",
        completion: "/policy",
    },
    SlashCommand {
        name: "voyages",
        usage: "/voyages",
        description: "Create and edit voyage drafts",
        completion: "/voyages",
    },
    SlashCommand {
        name: "workflow",
        usage: "/workflow [ID [--scope user|repository]]",
        description: "Browse and run saved workflows",
        completion: "/workflow",
    },
    SlashCommand {
        name: "help",
        usage: "/help",
        description: "Show command help",
        completion: "/help",
    },
    SlashCommand {
        name: "access",
        usage: "/access [MODE]",
        description: "Show or change access mode",
        completion: "/access ",
    },
    SlashCommand {
        name: "provider",
        usage: "/provider [ID]",
        description: "Show or change provider",
        completion: "/provider ",
    },
    SlashCommand {
        name: "workspace",
        usage: "/workspace [PATH]",
        description: "Show or change workspace",
        completion: "/workspace ",
    },
    SlashCommand {
        name: "config",
        usage: "/config [PATH]",
        description: "Show config or relaunch with a file",
        completion: "/config ",
    },
    SlashCommand {
        name: "set",
        usage: "/set KEY VALUE",
        description: "Set any validated config value",
        completion: "/set ",
    },
    SlashCommand {
        name: "tools",
        usage: "/tools",
        description: "List available tool calls",
        completion: "/tools",
    },
    SlashCommand {
        name: "activity",
        usage: "/activity [on|off]",
        description: "Toggle all tool calls or the last three between replies",
        completion: "/activity ",
    },
    SlashCommand {
        name: "model",
        usage: "/model [ID]",
        description: "Show or switch the model",
        completion: "/model ",
    },
    SlashCommand {
        name: "new",
        usage: "/new [TITLE]",
        description: "Start a new session",
        completion: "/new",
    },
    SlashCommand {
        name: "name",
        usage: "/name TITLE",
        description: "Rename the current session",
        completion: "/name ",
    },
    SlashCommand {
        name: "branch",
        usage: "/branch [TITLE]",
        description: "Branch the current session",
        completion: "/branch ",
    },
    SlashCommand {
        name: "compact",
        usage: "/compact [KEEP]",
        description: "Compact older context",
        completion: "/compact ",
    },
    SlashCommand {
        name: "export",
        usage: "/export [PATH]",
        description: "Export the session as Markdown",
        completion: "/export ",
    },
    SlashCommand {
        name: "clear",
        usage: "/clear confirm",
        description: "Permanently clear the conversation",
        completion: "/clear confirm",
    },
    SlashCommand {
        name: "auth",
        usage: "/auth ACTION",
        description: "Login, logout, status, or import",
        completion: "/auth ",
    },
    SlashCommand {
        name: "doctor",
        usage: "/doctor",
        description: "Run runtime diagnostics",
        completion: "/doctor",
    },
    SlashCommand {
        name: "sessions",
        usage: "/sessions",
        description: "Browse saved sessions",
        completion: "/sessions",
    },
    SlashCommand {
        name: "models",
        usage: "/models [json]",
        description: "Browse or print available models",
        completion: "/models",
    },
    SlashCommand {
        name: "resume",
        usage: "/resume REF",
        description: "Open a saved session",
        completion: "/resume ",
    },
    SlashCommand {
        name: "verbose",
        usage: "/verbose on|off",
        description: "Relaunch with debug logging",
        completion: "/verbose ",
    },
    SlashCommand {
        name: "log-format",
        usage: "/log-format text|json",
        description: "Relaunch with a log format",
        completion: "/log-format ",
    },
    SlashCommand {
        name: "plain",
        usage: "/plain",
        description: "Continue in line-oriented chat",
        completion: "/plain",
    },
    SlashCommand {
        name: "run",
        usage: "/run [--no-save] PROMPT",
        description: "Run a one-shot task",
        completion: "/run ",
    },
    SlashCommand {
        name: "completions",
        usage: "/completions SHELL",
        description: "Generate shell completions",
        completion: "/completions ",
    },
    SlashCommand {
        name: "manpage",
        usage: "/manpage",
        description: "Generate the Helm manpage",
        completion: "/manpage",
    },
];

pub(super) fn slash_command_query<'a>(context: &PaletteContext<'a>) -> Option<&'a str> {
    if context.running || context.dismissed {
        return None;
    }
    let query = context.input.strip_prefix('/')?;
    (!query.chars().any(char::is_whitespace)).then_some(query)
}

pub(super) fn slash_palette_matches(context: &PaletteContext<'_>) -> Vec<SlashCommand> {
    let Some(query) = slash_command_query(context) else {
        return Vec::new();
    };
    SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|command| command.name.starts_with(query))
        .collect()
}

pub(super) fn fixed_suggestions(
    prefix: &str,
    query: &str,
    values: &[(&str, &str)],
) -> Vec<SlashPaletteItem> {
    let query = query.trim().to_ascii_lowercase();
    values
        .iter()
        .filter(|(value, _)| value.to_ascii_lowercase().contains(&query))
        .map(|(value, description)| SlashPaletteItem {
            usage: (*value).into(),
            description: (*description).into(),
            completion: format!("{prefix}{value}"),
        })
        .collect()
}

pub(super) fn argument_hint(usage: &str, description: &str) -> SlashPaletteItem {
    SlashPaletteItem {
        usage: usage.into(),
        description: description.into(),
        completion: String::new(),
    }
}

pub(super) fn expand_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(suffix) = raw.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(suffix);
    }
    PathBuf::from(raw)
}

pub(super) fn filesystem_suggestions<F>(
    raw: &str,
    workspace: &Path,
    directories_only: bool,
    completion: F,
) -> Vec<SlashPaletteItem>
where
    F: Fn(&str, bool) -> String,
{
    let raw = raw.trim();
    let (scan_input, rendered_prefix) = if raw == "~" || raw.starts_with("~/") {
        let suffix = raw
            .strip_prefix('~')
            .unwrap_or_default()
            .trim_start_matches('/');
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        (home.join(suffix), "~/")
    } else {
        (PathBuf::from(raw), "")
    };
    let trailing_separator = raw == "~" || raw.ends_with(std::path::MAIN_SEPARATOR);
    let (directory, needle, display_parent) = if raw.is_empty() {
        (workspace.to_path_buf(), String::new(), String::new())
    } else if trailing_separator {
        let directory = if scan_input.is_absolute() {
            scan_input.clone()
        } else {
            workspace.join(&scan_input)
        };
        (
            directory,
            String::new(),
            if raw == "~" {
                "~/".into()
            } else {
                raw.to_owned()
            },
        )
    } else {
        let parent = scan_input
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let directory = if scan_input.is_absolute() {
            parent.to_path_buf()
        } else {
            workspace.join(parent)
        };
        let needle = scan_input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let display_parent = if rendered_prefix.is_empty() {
            parent.to_string_lossy().into_owned()
        } else {
            let suffix = parent
                .strip_prefix(dirs::home_dir().unwrap_or_default())
                .unwrap_or(parent)
                .to_string_lossy();
            if suffix.is_empty() {
                rendered_prefix.into()
            } else {
                format!("{rendered_prefix}{suffix}/")
            }
        };
        (directory, needle, display_parent)
    };
    let mut entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries.filter_map(Result::ok).collect::<Vec<_>>(),
        Err(_) => return Vec::new(),
    };
    entries.sort_by_key(|entry| {
        (
            !entry.file_type().is_ok_and(|kind| kind.is_dir()),
            entry.file_name().to_string_lossy().to_ascii_lowercase(),
        )
    });
    entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name
                .to_ascii_lowercase()
                .starts_with(&needle.to_ascii_lowercase())
                || (name.starts_with('.') && !needle.starts_with('.'))
            {
                return None;
            }
            let is_directory = entry.file_type().ok()?.is_dir();
            if directories_only && !is_directory {
                return None;
            }
            let separator = if !display_parent.is_empty()
                && !display_parent.ends_with(std::path::MAIN_SEPARATOR)
            {
                std::path::MAIN_SEPARATOR_STR
            } else {
                ""
            };
            let mut value = format!("{display_parent}{separator}{name}");
            if is_directory {
                value.push(std::path::MAIN_SEPARATOR);
            }
            Some(SlashPaletteItem {
                usage: value.clone(),
                description: if is_directory { "directory" } else { "file" }.into(),
                completion: completion(&value, is_directory),
            })
        })
        .take(100)
        .collect()
}

pub(super) fn model_suggestions(
    context: &PaletteContext<'_>,
    query: &str,
    prefix: &str,
) -> Vec<SlashPaletteItem> {
    let query = query.trim().to_ascii_lowercase();
    context
        .models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.id.to_ascii_lowercase().contains(&query)
                || model.display_name.to_ascii_lowercase().contains(&query)
        })
        .map(|model| SlashPaletteItem {
            usage: model.id.clone(),
            description: model.display_name.clone(),
            completion: format!("{prefix}{}", model.id),
        })
        .collect()
}

pub(super) fn environment_name_suggestions(query: &str, prefix: &str) -> Vec<SlashPaletteItem> {
    let query = query.to_ascii_lowercase();
    let mut names = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| name.to_ascii_lowercase().contains(&query))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .take(100)
        .map(|name| SlashPaletteItem {
            usage: name.clone(),
            description: "environment variable".into(),
            completion: format!("{prefix}{name} "),
        })
        .collect()
}

pub(super) fn set_value_suggestions(
    context: &PaletteContext<'_>,
    key: &str,
    value: &str,
) -> Vec<SlashPaletteItem> {
    let root_key = key.split('.').next().unwrap_or(key);
    let Some(spec) = CONFIG_OVERRIDE_SPECS
        .iter()
        .find(|spec| spec.key == root_key)
    else {
        return Vec::new();
    };
    let prefix = format!("/set {key} ");
    match spec.kind {
        ConfigValueKind::Bool => fixed_suggestions(
            &prefix,
            value,
            &[
                ("true", "Enable this setting"),
                ("false", "Disable this setting"),
            ],
        ),
        ConfigValueKind::Provider => fixed_suggestions(&prefix, value, PROVIDER_VALUES),
        ConfigValueKind::Model => model_suggestions(context, value, &prefix),
        ConfigValueKind::EnvironmentName => fixed_suggestions(
            &prefix,
            value,
            &[
                ("OPENAI_API_KEY", "OpenAI API credential"),
                ("ANTHROPIC_API_KEY", "Anthropic API credential"),
            ],
        ),
        ConfigValueKind::Url => fixed_suggestions(
            &prefix,
            value,
            &[
                ("https://api.openai.com/v1", "OpenAI public API"),
                ("https://api.anthropic.com", "Anthropic public API"),
            ],
        ),
        ConfigValueKind::PositiveInteger | ConfigValueKind::NonNegativeInteger => {
            let values: &[(&str, &str)] = match key {
                "max_tokens" => &[
                    ("2048", "small"),
                    ("4096", "standard"),
                    ("8192", "large"),
                    ("16384", "very large"),
                ],
                "provider_retry_attempts" => &[
                    ("0", "disabled"),
                    ("2", "light"),
                    ("4", "standard"),
                    ("8", "persistent"),
                ],
                "provider_retry_initial_ms" => {
                    &[("250", "fast"), ("500", "standard"), ("1000", "one second")]
                }
                "provider_retry_max_ms" => &[
                    ("4000", "four seconds"),
                    ("8000", "standard"),
                    ("16000", "sixteen seconds"),
                ],
                "command_timeout_secs" => &[
                    ("30", "short"),
                    ("120", "standard"),
                    ("300", "five minutes"),
                    ("900", "fifteen minutes"),
                ],
                "max_output_bytes" | "terminal_max_unread_bytes" => &[
                    ("65536", "64 KiB"),
                    ("131072", "128 KiB"),
                    ("1048576", "1 MiB"),
                    ("8388608", "8 MiB"),
                ],
                "terminal_max_count" | "subagent_max_concurrency" => &[
                    ("1", "one"),
                    ("4", "four"),
                    ("8", "eight"),
                    ("16", "sixteen"),
                ],
                "subagent_event_history" => {
                    &[("512", "small"), ("2048", "standard"), ("8192", "large")]
                }
                _ => &[("1", "minimum"), ("16", "small"), ("64", "standard")],
            };
            fixed_suggestions(&prefix, value, values)
        }
        ConfigValueKind::Temperature => fixed_suggestions(
            &prefix,
            value,
            &[
                ("0", "deterministic"),
                ("0.2", "focused"),
                ("0.7", "creative"),
                ("1.0", "high variance"),
            ],
        ),
        ConfigValueKind::Access => fixed_suggestions(&prefix, value, ACCESS_VALUES),
        ConfigValueKind::Approval => fixed_suggestions(
            &prefix,
            value,
            &[
                ("always", "approve every action"),
                ("on-risk", "approve risky actions"),
                ("never", "never prompt"),
            ],
        ),
        ConfigValueKind::UnattendedApproval => fixed_suggestions(
            &prefix,
            value,
            &[
                ("deny", "deny without a user"),
                ("allow", "allow without a user"),
            ],
        ),
        ConfigValueKind::Path => {
            filesystem_suggestions(value, context.workspace, true, |path, _| {
                format!("{prefix}{}", expand_user_path(path).display())
            })
        }
        ConfigValueKind::PathList => filesystem_suggestions(
            value.trim_matches(|ch| matches!(ch, '[' | ']' | '"')),
            context.workspace,
            true,
            |path, _| format!("{prefix}[\"{}\"]", expand_user_path(path).display()),
        ),
        ConfigValueKind::StringList => match key {
            "deny_commands" => fixed_suggestions(
                &prefix,
                value,
                &[(
                    "[\"shutdown\", \"reboot\", \"mkfs\"]",
                    "safe default deny list",
                )],
            ),
            "inherit_env" => fixed_suggestions(
                &prefix,
                value,
                &[(
                    "[\"PATH\", \"LANG\", \"LC_ALL\", \"TERM\"]",
                    "safe terminal environment",
                )],
            ),
            _ if value.trim().is_empty() => vec![argument_hint(
                "[\"VALUE\", ...]",
                "Enter a TOML string array",
            )],
            _ => Vec::new(),
        },
        ConfigValueKind::Executable => fixed_suggestions(
            &prefix,
            value,
            &[("codex", "Codex compatibility executable")],
        ),
        ConfigValueKind::Text if value.trim().is_empty() => {
            vec![argument_hint("<TEXT>", "Enter a free-form value")]
        }
        ConfigValueKind::StringMap if value.trim().is_empty() => {
            vec![argument_hint("<VALUE>", "Enter the map entry value")]
        }
        ConfigValueKind::McpServers if value.trim().is_empty() => vec![argument_hint(
            "<VALUE>",
            "Enter a string, TOML array, or map entry value",
        )],
        ConfigValueKind::Text | ConfigValueKind::StringMap | ConfigValueKind::McpServers => {
            Vec::new()
        }
    }
}

pub(super) const ACCESS_VALUES: &[(&str, &str)] = &[
    ("read-only", "Inspect without mutations"),
    ("approval", "Ask before consequential actions"),
    ("unrestricted", "Proceed without approval prompts"),
];

pub(super) const PROVIDER_VALUES: &[(&str, &str)] = &[
    ("openai-responses", "OpenAI Responses API"),
    ("openai-chat", "OpenAI-compatible Chat Completions"),
    ("chatgpt-oauth", "Native ChatGPT subscription"),
    ("anthropic", "Anthropic Messages API"),
    ("codex-compatibility", "External Codex compatibility bridge"),
    ("openai", "Legacy alias for openai-chat"),
    ("codex-subscription", "Legacy alias for codex-compatibility"),
];

pub(super) fn slash_argument_suggestions(context: &PaletteContext<'_>) -> Vec<SlashPaletteItem> {
    let Some(input) = context.input.strip_prefix('/') else {
        return Vec::new();
    };
    let Some((command, argument)) = input.split_once(char::is_whitespace) else {
        return Vec::new();
    };
    if command == "set" {
        if let Some((key, value)) = argument.split_once(char::is_whitespace) {
            return set_value_suggestions(context, key, value);
        }
        if let Some(query) = argument.strip_prefix("env.") {
            let suggestions = environment_name_suggestions(query, "/set env.");
            return if suggestions.is_empty() && query.is_empty() {
                vec![argument_hint(
                    "<NAME>",
                    "Enter an environment variable name",
                )]
            } else {
                suggestions
            };
        }
        if let Some(rest) = argument.strip_prefix("mcp_servers.") {
            if let Some((server, query)) = rest.split_once(".env.") {
                return environment_name_suggestions(
                    query,
                    &format!("/set mcp_servers.{server}.env."),
                );
            }
            let (server, field_query) = rest.split_once('.').unwrap_or((rest, ""));
            if server.is_empty() {
                return vec![argument_hint("<NAME>", "Enter an MCP server name")];
            }
            return fixed_suggestions(
                &format!("/set mcp_servers.{server}."),
                field_query,
                &[
                    ("command ", "Server executable"),
                    ("args ", "TOML argument array"),
                    ("env.", "Server environment variable"),
                ],
            );
        }
        let query = argument.trim().to_ascii_lowercase();
        return CONFIG_OVERRIDE_SPECS
            .iter()
            .filter(|spec| spec.key.contains(&query))
            .map(|spec| SlashPaletteItem {
                usage: spec.key.into(),
                description: spec.description.into(),
                completion: format!(
                    "/set {}{}",
                    spec.key,
                    if matches!(
                        spec.kind,
                        ConfigValueKind::StringMap | ConfigValueKind::McpServers
                    ) {
                        "."
                    } else {
                        " "
                    }
                ),
            })
            .collect();
    }
    if command == "workspace" {
        let suggestions = filesystem_suggestions(argument, context.workspace, true, |path, _| {
            format!("/workspace {path}")
        });
        return if suggestions.is_empty() && argument.trim().is_empty() {
            vec![argument_hint("<PATH>", "Enter a workspace directory")]
        } else {
            suggestions
        };
    }
    if matches!(command, "config" | "export") {
        let suggestions = filesystem_suggestions(argument, context.workspace, false, |path, _| {
            format!("/{command} {path}")
        });
        return if suggestions.is_empty() && argument.trim().is_empty() {
            vec![argument_hint("<PATH>", "Enter a file path")]
        } else {
            suggestions
        };
    }
    if command == "auth" && argument.starts_with("import-codex --path ") {
        let path = argument.trim_start_matches("import-codex --path ");
        return filesystem_suggestions(path, context.workspace, false, |path, _| {
            let path = expand_user_path(path).to_string_lossy().into_owned();
            format!("/auth import-codex --path {}", shell_words::quote(&path))
        });
    }
    if command == "auth" && argument.starts_with("import-codex --force --path ") {
        let path = argument.trim_start_matches("import-codex --force --path ");
        return filesystem_suggestions(path, context.workspace, false, |path, _| {
            let path = expand_user_path(path).to_string_lossy().into_owned();
            format!(
                "/auth import-codex --force --path {}",
                shell_words::quote(&path)
            )
        });
    }
    if command == "run"
        && let Some(prompt) = argument.strip_prefix("--no-save ")
    {
        return if prompt.trim().is_empty() {
            vec![argument_hint("<PROMPT>", "Enter the unsaved task to run")]
        } else {
            Vec::new()
        };
    }
    let query = argument.trim().to_ascii_lowercase();
    let fixed: &[(&str, &str)] = match command {
        "access" => ACCESS_VALUES,
        "provider" => PROVIDER_VALUES,
        "activity" | "verbose" => &[("on", "Enable"), ("off", "Disable")],
        "log-format" => &[("text", "Human-readable logs"), ("json", "JSON logs")],
        "compact" => &[
            ("24", "keep recent context"),
            ("64", "keep extended context"),
            ("96", "keep large context"),
        ],
        "run" => &[("--no-save ", "Run without saving the result")],
        "completions" => &[
            ("bash", "Bash"),
            ("elvish", "Elvish"),
            ("fish", "Fish"),
            ("powershell", "PowerShell"),
            ("zsh", "Zsh"),
        ],
        "auth" if argument.starts_with("login ") => {
            &[("login --device", "Sign in with a device code")]
        }
        "auth" if argument.starts_with("import-codex ") => &[
            ("import-codex --path ", "Choose a Codex auth file"),
            ("import-codex --force", "Replace existing Helm credentials"),
            (
                "import-codex --force --path ",
                "Replace credentials using a selected file",
            ),
        ],
        "auth" => &[
            ("status", "Show credential status"),
            ("login", "Sign in with a browser callback"),
            ("login --device", "Sign in with a device code"),
            ("logout", "Remove Helm credentials"),
            ("import-codex", "Import an existing Codex login once"),
        ],
        "models" => &[("json", "Print the provider model catalog as JSON")],
        _ => &[],
    };
    let prefix = format!("/{command} ");
    let mut suggestions = fixed_suggestions(&prefix, &query, fixed);
    if argument.trim().is_empty() {
        match command {
            "new" | "name" | "branch" => {
                suggestions.push(argument_hint("<TITLE>", "Enter a session title"))
            }
            "run" => suggestions.push(argument_hint("<PROMPT>", "Enter the task to run")),
            _ => {}
        }
    }
    if matches!(command, "model" | "models") {
        suggestions.extend(model_suggestions(context, &query, "/model "));
    }
    if command == "resume" {
        suggestions.extend(
            context
                .sessions
                .iter()
                .filter(|session| {
                    let id = session.id.to_string();
                    query.is_empty()
                        || id.contains(&query)
                        || session
                            .name
                            .as_deref()
                            .is_some_and(|name| name.to_ascii_lowercase().contains(&query))
                })
                .map(|session| SlashPaletteItem {
                    usage: session.id.to_string(),
                    description: session.display_name(),
                    completion: format!("/resume {}", session.id),
                }),
        );
    }
    suggestions
}

pub(super) fn slash_palette_items(context: &PaletteContext<'_>) -> Vec<SlashPaletteItem> {
    let commands = slash_palette_matches(context);
    if !commands.is_empty() {
        return commands
            .into_iter()
            .map(|command| SlashPaletteItem {
                usage: command.usage.into(),
                description: command.description.into(),
                completion: command.completion.into(),
            })
            .collect();
    }
    slash_argument_suggestions(context)
}

pub(super) fn slash_input_is_complete(context: &PaletteContext<'_>) -> bool {
    let Some(query) = slash_command_query(context) else {
        return slash_palette_items(context)
            .iter()
            .any(|item| item.completion == context.input);
    };
    SLASH_COMMANDS.iter().any(|command| command.name == query)
}

pub(super) fn draw_slash_palette(
    frame: &mut ratatui::Frame<'_>,
    composer_area: Rect,
    context: &PaletteContext<'_>,
) {
    let items = slash_palette_items(context);
    if items.is_empty() || composer_area.y < 3 || composer_area.width < 8 {
        return;
    }
    let selected = context.selected.min(items.len().saturating_sub(1));
    let available_rows = composer_area.y.saturating_sub(2).max(1) as usize;
    let columns = usize::from(items.len() > available_rows && composer_area.width >= 72) + 1;
    let rows_needed = items.len().div_ceil(columns);
    let visible_rows = rows_needed.min(available_rows);
    let capacity = visible_rows * columns;
    let start = (selected / capacity) * capacity;
    let cell_width = (composer_area.width.saturating_sub(2) as usize / columns).max(1);
    let mut lines = Vec::with_capacity(visible_rows);
    for row in 0..visible_rows {
        let mut spans = Vec::new();
        for column in 0..columns {
            let index = start + row + column * visible_rows;
            let Some(item) = items.get(index) else {
                continue;
            };
            let content = format!("{:<18} {}", item.usage, item.description);
            let mut content = one_line(&content, cell_width.saturating_sub(1));
            let padding = cell_width.saturating_sub(content.chars().count());
            content.extend(std::iter::repeat_n(' ', padding));
            let style = if index == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(content, style));
        }
        lines.push(Line::from(spans));
    }
    let height = visible_rows as u16 + 2;
    let popup = Rect {
        x: composer_area.x,
        y: composer_area.y.saturating_sub(height),
        width: composer_area.width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .title(" Commands and options · ↑↓ select · Enter/Tab complete · Esc close ")
                .borders(Borders::ALL),
        ),
        popup,
    );
}
