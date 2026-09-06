//! Operator panels read the active owner's real controls and keep their target.
use super::{App, observe::Update, state::Target};
use anyhow::{Context, Result, ensure};
use voyage_protocol::process::RuntimeCommand;

impl App {
    pub(super) fn inspect_control(&mut self, target: Target, section: &str) -> Result<()> {
        let view = &self.views[&target];
        let run_id = view
            .snapshot
            .as_ref()
            .and_then(|s| s.run.as_ref())
            .map(|run| run.run_id);
        let incarnation = view.process.incarnation;
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let section = section.to_owned();
        self.status = format!("Loading {section} for {}", target.session);
        tokio::spawn(async move {
            let result = client
                .forward(
                    target.session,
                    incarnation,
                    RuntimeCommand::Controls { run_id, section },
                )
                .await
                .and_then(|value| Ok(serde_json::to_string_pretty(&value)?))
                .map_err(|error| error.to_string());
            let _ = sender
                .send(Update::Control {
                    target,
                    incarnation,
                    result,
                })
                .await;
        });
        Ok(())
    }
}

pub(super) fn mutation(
    text: &str,
    view: &super::state::View,
    command_id: uuid::Uuid,
    expected_revision: u64,
    expires_at_ms: u64,
) -> Result<Option<RuntimeCommand>> {
    Ok(if let Some(id) = text.strip_prefix("/clear ") {
        let confirm_session_id = id.parse()?;
        ensure!(
            confirm_session_id == view.process.session_id,
            "type /clear followed by this voyage's full UUID"
        );
        Some(RuntimeCommand::Clear {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        })
    } else if let Some(retain) = text.strip_prefix("/compact ") {
        let retain = retain
            .parse::<u32>()
            .context("use /compact NUMBER_OF_MESSAGES")?;
        Some(RuntimeCommand::Compact {
            command_id,
            expected_revision,
            expires_at_ms,
            retain,
        })
    } else if let Some(path) = text.strip_prefix("/configure ") {
        let config_path = std::path::PathBuf::from(path);
        ensure!(
            config_path.is_absolute(),
            "configuration path must be absolute on the executing host"
        );
        Some(RuntimeCommand::Configure {
            command_id,
            expected_revision,
            expires_at_ms,
            config_path,
        })
    } else if text == "/archive" || text == "/restore" {
        Some(RuntimeCommand::Archive {
            command_id,
            expected_revision,
            expires_at_ms,
            archived: text == "/archive",
        })
    } else if let Some(id) = text.strip_prefix("/delete ") {
        let confirm_session_id = id.parse()?;
        ensure!(
            confirm_session_id == view.process.session_id,
            "type /delete followed by this voyage's full UUID to confirm history deletion"
        );
        Some(RuntimeCommand::Delete {
            command_id,
            expected_revision,
            expires_at_ms,
            confirm_session_id,
        })
    } else if let Some(arguments) = text.strip_prefix("/tool ") {
        let (name, arguments) = arguments
            .split_once(' ')
            .context("use /tool NAME JSON_ARGUMENTS")?;
        let arguments = serde_json::from_str(arguments).context("invalid tool arguments")?;
        let run_id = view
            .snapshot
            .as_ref()
            .and_then(|s| s.run.as_ref())
            .filter(|r| r.active())
            .map(|r| r.run_id);
        Some(match run_id {
            Some(run_id) => RuntimeCommand::ExecuteTool {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id,
                name: name.into(),
                arguments,
            },
            None => RuntimeCommand::OperatorTool {
                command_id,
                expected_revision,
                expires_at_ms,
                name: name.into(),
                arguments,
            },
        })
    } else {
        None
    })
}
