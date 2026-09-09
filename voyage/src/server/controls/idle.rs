//! Idle views read session inventories and policy; they do not instantiate an agent.
use super::*;
use sha2::{Digest, Sha256};
pub(super) async fn inspect(
    controls: &LiveControls,
    section: &str,
    config: &crate::Config,
    workspace: &std::path::Path,
) -> Result<Value> {
    let resolved = crate::runtime_policy::RuntimePolicy::resolve(config, workspace)?;
    let root = crate::build::resource_root();
    let workspace = workspace.canonicalize()?;
    let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    let value = match section {
        "tools" => match controls.inventory.read().await.clone() {
            Some(inventory) => {
                json!({"inventory":inventory,"source":"last_runtime_registry","refresh":"next_run"})
            }
            None => {
                json!({"inventory":[],"source":"runtime_not_constructed","refresh":"first_run"})
            }
        },
        "policy" => serde_json::to_value(resolved.policy().effective())?,
        "models" => {
            let reservation =
                crate::host_resources::Reservation::acquire("executors", uuid::Uuid::new_v4(), 1)?;
            let discovery = async {
                let provider = crate::provider::from_config(resolved.config())?;
                Ok::<_, anyhow::Error>(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(20).min(config.timeout()),
                        provider.models(),
                    )
                    .await??,
                )
            }
            .await;
            reservation.release_observed()?;
            let mut models = discovery?;
            if !models.iter().any(|model| model.id == config.model) {
                models.push(crate::provider::ModelInfo::minimal(config.model.clone()));
            }
            let secrets = crate::build::redactor(resolved.config());
            crate::provider::validate_models_for_display(&models, |value| {
                secrets.contains_secret(value)
            })?;
            crate::provider::normalize_models(&mut models);
            serde_json::to_value(models)?
        }
        "todos" => serde_json::to_value(
            crate::todo::TodoStore::new(
                root.join("todos").join(format!("{key}.json")),
                crate::todo::TodoScope::workspace(workspace),
            )
            .snapshot()
            .await?,
        )?,
        "subagents" => serde_json::to_value(
            crate::subagent::AgentTreeStore::new(
                root.join("subagents").join(format!("{key}.json")),
            )
            .list()
            .await?,
        )?,
        "terminals" => {
            use crate::terminal::InteractiveTerminals;
            let retained = controls.retained.read().await.clone();
            match retained {
                Some(entry) => {
                    let entry = controls.retained(entry.run).await?;
                    return Ok(
                        json!({"run_id":entry.run,"section":section,"value":entry.manager.list().await?,"execution":"idle"}),
                    );
                }
                None => json!([]),
            }
        }
        "workflows" => serde_json::to_value(crate::workflow::discover(&workspace, None)?)?,
        "host_resources" => {
            let mut summary = crate::host_resources::inspect()?;
            if let Some(object) = summary.as_object_mut() {
                object.remove("database");
                object.remove("reservations");
            }
            summary
        }
        _ => anyhow::bail!("unknown idle control section"),
    };
    ensure!(
        serde_json::to_vec(&value)?.len() <= 1024 * 1024,
        "idle control result exceeds bounded frame"
    );
    Ok(json!({"run_id":null,"section":section,"value":value,"execution":"idle"}))
}
