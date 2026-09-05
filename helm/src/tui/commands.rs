//! Session commands and explicit CLI handoff requests.

use super::{
    App, CliRequest, TuiExit,
    bridge::UiEvent,
    composer::{Composer, PromptHistory},
    models::request_models,
    palette::{SLASH_COMMANDS, expand_user_path},
};
use crate::{
    Agent,
    session::{Session, SessionStore, compact_messages},
};
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::mpsc;

pub(super) fn export_path(session: &Session) -> PathBuf {
    session
        .workspace
        .join(format!("helm-session-{}.md", session.id))
}

pub(super) async fn request_cli(
    app: &mut App,
    store: &SessionStore,
    arguments: Vec<String>,
    use_active_config: bool,
    resume_after: bool,
) -> Result<()> {
    store.save(&mut app.session).await?;
    app.exit = Some(TuiExit::Launch(CliRequest {
        arguments,
        use_active_config,
        resume_after,
        verbose: None,
        log_format: None,
    }));
    app.quit = true;
    Ok(())
}

pub(super) fn resume_chat_arguments(app: &App) -> Vec<String> {
    vec!["chat".into(), "--resume".into(), app.session.id.to_string()]
}

pub(super) fn parse_words(argument: &str, usage: &str) -> Result<Vec<String>> {
    shell_words::split(argument).with_context(|| format!("invalid arguments; usage: {usage}"))
}

pub(super) fn start_new_session(app: &mut App, name: Option<&str>) {
    let mut session = Session::new(app.session.workspace.clone(), app.session.model.clone());
    if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
        session.set_name(name.trim().to_owned());
    }
    let name = session.display_name();
    app.session = session;
    app.prompt_history = PromptHistory::default();
    app.activity.clear();
    app.live_messages.clear();
    app.streaming_response.clear();
    app.scroll = 0;
    app.status = format!("New session: {name}");
}

pub(super) async fn handle_command(
    command: &str,
    app: &mut App,
    store: &SessionStore,
    agent: Option<&Arc<Agent>>,
    tx: Option<&mpsc::UnboundedSender<UiEvent>>,
) -> Result<bool> {
    let Some(command) = command.strip_prefix('/') else {
        return Ok(false);
    };
    let (name, argument) = command.split_once(' ').unwrap_or((command, ""));
    match name {
        "help" => {
            app.show_activity = true;
            app.activity.push("Slash commands:".into());
            app.activity.extend(
                SLASH_COMMANDS
                    .iter()
                    .map(|command| format!("  {:<24} {}", command.usage, command.description)),
            );
            app.status = "Slash command help added to the conversation".into();
        }
        "access" if argument.trim().is_empty() => {
            app.status = format!("Current access mode: {}", app.access_mode)
        }
        "access" => {
            let mode = argument.trim();
            if !matches!(mode, "read-only" | "approval" | "unrestricted") {
                app.status = "Usage: /access read-only|approval|unrestricted".into();
            } else {
                let mut arguments = vec!["--access".into(), mode.into()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, true, false).await?;
            }
        }
        "provider" if argument.trim().is_empty() => {
            app.status = format!("Current provider: {}", app.provider_label)
        }
        "provider" => {
            let provider = argument.trim();
            if !matches!(
                provider,
                "openai-responses"
                    | "openai-chat"
                    | "openai"
                    | "chatgpt-oauth"
                    | "anthropic"
                    | "codex-compatibility"
                    | "codex-subscription"
            ) {
                app.status = "Usage: /provider openai-responses|openai-chat|chatgpt-oauth|anthropic|codex-compatibility".into();
            } else {
                let mut arguments = vec!["--provider".into(), provider.into()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, true, false).await?;
            }
        }
        "workspace" if argument.trim().is_empty() => {
            app.status = format!("Current workspace: {}", app.session.workspace.display())
        }
        "workspace" => {
            let path = expand_user_path(argument.trim());
            match path.canonicalize() {
                Ok(path) if path.is_dir() => {
                    request_cli(
                        app,
                        store,
                        vec![
                            "--workspace".into(),
                            path.to_string_lossy().into_owned(),
                            "chat".into(),
                        ],
                        true,
                        false,
                    )
                    .await?;
                }
                _ => app.status = format!("Workspace does not exist: {}", path.display()),
            }
        }
        "config" if argument.trim().is_empty() => {
            request_cli(app, store, vec!["config".into()], true, true).await?;
        }
        "config" => {
            let path = expand_user_path(argument.trim());
            if !path.is_file() {
                app.status = format!("Config file does not exist: {}", path.display());
            } else {
                let mut arguments = vec!["--config".into(), path.to_string_lossy().into_owned()];
                arguments.extend(resume_chat_arguments(app));
                request_cli(app, store, arguments, false, false).await?;
            }
        }
        "set" => {
            let Some((key, value)) = argument.trim().split_once(char::is_whitespace) else {
                app.status = "Usage: /set KEY VALUE".into();
                return Ok(true);
            };
            let mut arguments = vec!["--set".into(), format!("{}={}", key.trim(), value.trim())];
            arguments.extend(resume_chat_arguments(app));
            request_cli(app, store, arguments, true, false).await?;
        }
        "tools" => {
            if let Some(agent) = agent {
                app.show_activity = true;
                let tools = agent.tool_inventory();
                app.activity.push("Available Helm tool calls:".into());
                app.activity.extend(
                    tools
                        .iter()
                        .map(|tool| format!("  {} — {}", tool.name, tool.description)),
                );
                app.status = format!("{} tool call(s) available", tools.len());
            } else {
                app.status = "Tool inventory unavailable while runtime is starting".into();
            }
        }
        "activity" => {
            app.show_activity = match argument.trim() {
                "" => !app.show_activity,
                "on" => true,
                "off" => false,
                _ => {
                    app.status = "Usage: /activity [on|off]".into();
                    return Ok(true);
                }
            };
            app.status = if app.show_activity {
                "Full activity visible · Ctrl+L returns to recent calls"
            } else {
                "Last 3 calls per reply · Ctrl+L shows full activity"
            }
            .into();
        }
        "model" if argument.trim().is_empty() => {
            app.status = format!(
                "Current model: {} · Ctrl+M opens the model picker",
                app.session.model
            )
        }
        "model" => {
            let model = argument.trim();
            app.session.switch_model(model)?;
            if let Some(agent) = agent {
                agent.set_model(model)?;
            }
            store.save(&mut app.session).await?;
            app.status = format!("Model switched to {model}");
        }
        "models" if argument.trim().is_empty() => {
            if let (Some(agent), Some(tx)) = (agent, tx) {
                app.model_panel.model_picker = true;
                app.model_panel.model_manual = false;
                app.model_panel.model_filter = Composer::default();
                app.model_panel.selected_model = 0;
                request_models(tx, (*agent).clone(), false);
            } else {
                app.status = "Model picker unavailable while runtime is starting".into();
            }
        }
        "models" if argument.trim() == "json" => {
            request_cli(
                app,
                store,
                vec!["models".into(), "--json".into()],
                true,
                true,
            )
            .await?;
        }
        "models" => app.status = "Usage: /models [json]".into(),
        "sessions" => {
            app.sessions = store.list().await?;
            app.selected_session = 0;
            app.show_sessions = true;
        }
        "resume" if !argument.trim().is_empty() => {
            match store.load_reference(argument.trim()).await {
                Ok(session) => {
                    request_cli(
                        app,
                        store,
                        vec!["chat".into(), "--resume".into(), session.id.to_string()],
                        true,
                        false,
                    )
                    .await?;
                }
                Err(error) => app.status = format!("Cannot resume session: {error}"),
            }
        }
        "resume" => app.status = "Usage: /resume SESSION".into(),
        "new" => start_new_session(
            app,
            (!argument.trim().is_empty()).then_some(argument.trim()),
        ),
        "name" if !argument.trim().is_empty() => {
            app.session.set_name(argument.trim().into());
            store.save(&mut app.session).await?;
            app.sessions = store.list().await?;
            app.status = "Session renamed".into();
        }
        "branch" => {
            let branch_name = (!argument.trim().is_empty()).then(|| argument.trim().to_owned());
            app.session = store.branch(&app.session, branch_name).await?;
            app.sessions = store.list().await?;
            app.streaming_response.clear();
            app.scroll = 0;
            app.status = "Branched session".into();
        }
        "compact" => {
            let retain = if argument.trim().is_empty() {
                24
            } else {
                argument
                    .trim()
                    .parse()
                    .context("/compact expects a message count")?
            };
            let removed = compact_messages(&mut app.session.messages, retain);
            store.save(&mut app.session).await?;
            app.scroll = 0;
            app.status = format!("Compacted {removed} messages");
        }
        "export" => {
            let path = if argument.trim().is_empty() {
                export_path(&app.session)
            } else {
                expand_user_path(argument.trim())
            };
            store.export_markdown(&app.session, &path).await?;
            app.status = format!("Exported to {}", path.display());
        }
        "auth" => {
            let words = parse_words(
                argument,
                "/auth status|login [--device]|logout|import-codex",
            )?;
            let valid = matches!(words.as_slice(), [action] if matches!(action.as_str(), "status" | "logout"))
                || matches!(words.as_slice(), [action] if action == "login")
                || matches!(words.as_slice(), [action, flag] if action == "login" && flag == "--device")
                || words.first().is_some_and(|action| action == "import-codex");
            if !valid {
                app.status =
                    "Usage: /auth status|login [--device]|logout|import-codex [--path PATH] [--force]"
                        .into();
            } else {
                let mut arguments = vec!["auth".into()];
                arguments.extend(words);
                request_cli(app, store, arguments, true, true).await?;
            }
        }
        "doctor" => {
            request_cli(app, store, vec!["doctor".into()], true, true).await?;
        }
        "verbose" => {
            let value = argument.trim();
            if !matches!(value, "on" | "off") {
                app.status = "Usage: /verbose on|off".into();
            } else {
                let arguments = resume_chat_arguments(app);
                request_cli(app, store, arguments, true, false).await?;
                if let Some(TuiExit::Launch(request)) = &mut app.exit {
                    request.verbose = Some(value == "on");
                }
            }
        }
        "log-format" => {
            let format = argument.trim();
            if !matches!(format, "text" | "json") {
                app.status = "Usage: /log-format text|json".into();
            } else {
                let arguments = resume_chat_arguments(app);
                request_cli(app, store, arguments, true, false).await?;
                if let Some(TuiExit::Launch(request)) = &mut app.exit {
                    request.log_format = Some(format.into());
                }
            }
        }
        "plain" => {
            let mut arguments = resume_chat_arguments(app);
            arguments.push("--plain".into());
            request_cli(app, store, arguments, true, false).await?;
        }
        "run" if !argument.trim().is_empty() => {
            let mut words = parse_words(argument, "/run [--no-save] PROMPT")?;
            let no_save = words.first().is_some_and(|word| word == "--no-save");
            if no_save {
                words.remove(0);
            }
            if words.is_empty() {
                app.status = "Usage: /run [--no-save] PROMPT".into();
            } else {
                let mut arguments =
                    vec!["run".into(), "--resume".into(), app.session.id.to_string()];
                if no_save {
                    arguments.push("--no-save".into());
                }
                arguments.extend(words);
                request_cli(app, store, arguments, true, true).await?;
            }
        }
        "run" => app.status = "Usage: /run [--no-save] PROMPT".into(),
        "completions" => {
            let shell = argument.trim();
            if !matches!(shell, "bash" | "elvish" | "fish" | "powershell" | "zsh") {
                app.status = "Usage: /completions bash|elvish|fish|powershell|zsh".into();
            } else {
                request_cli(
                    app,
                    store,
                    vec!["completions".into(), shell.into()],
                    true,
                    true,
                )
                .await?;
            }
        }
        "manpage" => {
            request_cli(app, store, vec!["manpage".into()], true, true).await?;
        }
        "clear" if argument.trim() == "confirm" => {
            app.session.clear_conversation();
            store.save(&mut app.session).await?;
            app.activity.clear();
            app.live_messages.clear();
            app.streaming_response.clear();
            app.scroll = 0;
            app.status = "Conversation cleared".into();
        }
        "clear" => app.status = "Clearing is permanent; use /clear confirm".into(),
        _ => app.status = format!("Unknown or incomplete command: /{name}"),
    }
    Ok(true)
}
