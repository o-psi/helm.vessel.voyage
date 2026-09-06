//! Execution-host workflow revalidation and transient private input handoff.
use super::*;
use anyhow::Context;

pub async fn run(
    config: crate::Config,
    workspace: PathBuf,
    prepared: crate::workflow::Prepared,
    user_directory: Option<PathBuf>,
    model_overridden: bool,
) -> Result<()> {
    let user_directory = user_directory
        .map(|path| path.canonicalize())
        .transpose()
        .context("workflow user directory unavailable")?;
    let (client, process) = open(&config, Some(workspace), None, model_overridden, true).await?;
    let crate::workflow::Prepared {
        input_monitor,
        secrets,
        invocation,
        no_save,
        ..
    } = prepared;
    let execution = async {
        let cancelled = input_monitor.as_ref().map(|monitor| monitor.cancellation());
        ensure!(
            !cancelled.as_ref().is_some_and(|token| token.is_cancelled()),
            "workflow input cancelled"
        );
        let values = secrets.into_private_transport();
        let private_inputs_id = if values.is_empty() {
            None
        } else {
            let input_id = Uuid::new_v4();
            client
                .forward(
                    process.session_id,
                    process.incarnation,
                    RuntimeCommand::WorkflowInputs { input_id, values },
                )
                .await?;
            Some(input_id)
        };
        let snapshot = client
            .forward(
                process.session_id,
                process.incarnation,
                RuntimeCommand::Snapshot,
            )
            .await?;
        let expected_revision = snapshot["revision"]
            .as_u64()
            .context("snapshot revision missing")?;
        let scope = match invocation.scope {
            crate::workflow::Scope::User => "user",
            crate::workflow::Scope::Repository => "repository",
        };
        let inputs = invocation
            .inputs
            .into_iter()
            .map(|(key, value)| {
                let value = match value {
                    serde_json::Value::String(value) => value,
                    value => value.to_string(),
                };
                (key, value)
            })
            .collect();
        let command_id = Uuid::new_v4();
        eprintln!(
            "Workflow {} · voyage {} · command {command_id}",
            invocation.id, process.session_id
        );
        let receipt = client
            .forward(
                process.session_id,
                process.incarnation,
                RuntimeCommand::WorkflowSubmit {
                    command_id,
                    expected_revision,
                    expires_at_ms: deadline()?,
                    id: invocation.id,
                    scope: Some(scope.into()),
                    inputs,
                    trust_digest: Some(invocation.digest),
                    private_inputs_id,
                    user_directory,
                },
            )
            .await?;
        ensure!(
            receipt["status"] != "rejected",
            "workflow was rejected: {}",
            super::super::safe(&receipt.to_string())
        );
        let run_id = serde_json::from_value(receipt["run_id"].clone())
            .context("workflow receipt omitted run")?;
        drop(input_monitor);
        super::super::plain::follow(&client, process.session_id, run_id).await
    }
    .await;
    if no_save {
        super::discard(&client, &process).await?;
    }
    execution
}
