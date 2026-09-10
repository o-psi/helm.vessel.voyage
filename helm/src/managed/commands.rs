use super::*;
use voyage_protocol::vessel::{VesselCommand, VoyageCommand};

pub(super) async fn run(
    args: Args,
    config: Option<Config>,
    workspace: Option<PathBuf>,
    model_overridden: bool,
) -> Result<()> {
    let client = connection::connect(&args.directory).await?;
    match args.command {
        ManagedCommand::Create { id, name } => {
            let config = config.context("managed create requires configuration")?;
            let workspace = config.resolve_workspace(workspace)?;
            let config_path = helm::process_client::frontend::persist_launch(
                &config,
                &workspace,
                &client.directory,
            )?;
            let session_id = id.unwrap_or_else(Uuid::new_v4);
            let command_id = Uuid::new_v4();
            eprintln!("Starting managed voyage {session_id} · command {command_id}");
            let process: voyage_protocol::vessel::ProcessInfo = serde_json::from_value(
                client
                    .request(VesselCommand::StartConfigured {
                        command_id,
                        session_id,
                        workspace,
                        config_path,
                    })
                    .await?,
            )?;
            let mut snapshot = client
                .voyage(session_id, process.incarnation, VoyageCommand::Snapshot)
                .await?;
            if let Some(name) = name {
                client
                    .voyage(
                        session_id,
                        process.incarnation,
                        VoyageCommand::Rename {
                            command_id: Uuid::new_v4(),
                            expected_revision: revision(&snapshot)?,
                            expires_at_ms: connection::deadline()?,
                            name,
                        },
                    )
                    .await?;
                snapshot = client
                    .voyage(session_id, process.incarnation, VoyageCommand::Snapshot)
                    .await?;
            }
            snapshot["id"] = json!(session_id);
            emit(
                json!({"event":"session_created","session":snapshot}),
                args.json,
            )
        }
        ManagedCommand::List { after, limit } => emit(
            catalogue::list(&client, &args.directory, after, limit).await?,
            args.json,
        ),
        ManagedCommand::Submit {
            session,
            expected_revision,
            command_id,
            expires_at_ms,
            prompt,
        } => {
            let process = connection::session(
                &client,
                &args.directory,
                session,
                config.as_ref(),
                workspace,
            )
            .await?;
            let process = connection::live(&client, process).await?;
            let snapshot = client
                .voyage(session, process.incarnation, VoyageCommand::Snapshot)
                .await?;
            ensure!(
                !model_overridden
                    || config
                        .as_ref()
                        .is_some_and(|c| snapshot["model"] == c.model),
                "managed model does not match saved session"
            );
            let command_id = command_id.unwrap_or_else(Uuid::new_v4);
            let expires_at_ms = match expires_at_ms {
                Some(value) => u64::try_from(value)?,
                None => connection::deadline()?,
            };
            let receipt = client
                .voyage(
                    session,
                    process.incarnation,
                    VoyageCommand::Submit {
                        coordination: None,
                        command_id,
                        expected_revision,
                        expires_at_ms,
                        prompt: prompt.join(" "),
                    },
                )
                .await?;
            ensure!(
                receipt["status"] != "rejected",
                "managed command rejected: {}",
                helm::process_client::safe(&receipt.to_string())
            );
            let duplicate = receipt["duplicate"] == true || receipt["status"] == "transferred";
            emit(
                json!({"event":if duplicate{"existing_run"}else{"run_accepted"},"receipt":receipt}),
                args.json,
            )?;
            if duplicate {
                return Ok(());
            }
            let run = serde_json::from_value(receipt["run_id"].clone())?;
            if args.json {
                helm::process_client::plain::follow_json(&client, session, run).await
            } else {
                helm::process_client::plain::follow(&client, session, run).await
            }
        }
        ManagedCommand::Cancel { session, run } => {
            let process =
                connection::session(&client, &args.directory, session, None, None).await?;
            let snapshot = client
                .voyage(session, process.incarnation, VoyageCommand::Snapshot)
                .await?;
            let receipt = client
                .voyage(
                    session,
                    process.incarnation,
                    VoyageCommand::Cancel {
                        command_id: Uuid::new_v4(),
                        expected_revision: revision(&snapshot)?,
                        expires_at_ms: connection::deadline()?,
                        run_id: run,
                    },
                )
                .await?;
            emit(
                json!({"event":"cancel_requested","receipt":receipt}),
                args.json,
            )
        }
        ManagedCommand::Recover {
            session,
            acknowledge_cleanup,
            acknowledge_resources,
            reconcile_tools,
            expected_revision,
        } => {
            let catalogue: Vec<voyage_protocol::vessel::ProcessInfo> =
                serde_json::from_value(client.request(VesselCommand::Catalogue).await?)?;
            if !catalogue
                .iter()
                .any(|process| process.session_id == session)
            {
                ensure!(
                    acknowledge_resources.is_empty(),
                    "legacy installations have no supervised session resource identities"
                );
                let result = maintenance::recover(
                    &args.directory,
                    session,
                    acknowledge_cleanup,
                    reconcile_tools,
                    expected_revision,
                )
                .await?;
                return emit(json!({"event":"recovered","result":result}), args.json);
            }
            let process =
                connection::session(&client, &args.directory, session, None, None).await?;
            let result = client
                .request(VesselCommand::Recover {
                    command_id: Uuid::new_v4(),
                    session_id: session,
                    incarnation: process.incarnation,
                    acknowledge_cleanup,
                    acknowledge_resources,
                    reconcile_tools,
                    expected_revision,
                })
                .await?;
            emit(json!({"event":"recovered","result":result}), args.json)
        }
        ManagedCommand::Upgrade => emit(maintenance::upgrade(&args.directory).await?, args.json),
    }
}
fn revision(value: &Value) -> Result<u64> {
    value["revision"]
        .as_u64()
        .context("snapshot revision missing")
}
fn emit(value: Value, json: bool) -> Result<()> {
    let text = if json {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    println!("{}", helm::process_client::safe(&text));
    Ok(())
}
