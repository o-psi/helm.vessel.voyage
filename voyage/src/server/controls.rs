//! Operator access to the active executor, never a second agent or resource owner.
mod idle;
mod retained;
mod terminal;
mod tool;
use crate::{Agent, attachment::runtime::ManagedSessionOwner, subagent::SubagentRuntime};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::process::{RuntimeCommand, TerminalOperation};

#[derive(Clone)]
struct Active {
    run: Uuid,
    agent: Arc<Agent>,
    subagents: Arc<SubagentRuntime>,
    cancel: CancellationToken,
    slots: Arc<Semaphore>,
}
#[derive(Default)]
pub struct LiveControls {
    retained: Arc<RwLock<Option<retained::Retained>>>,
    active: RwLock<Option<Active>>,
    inventory: RwLock<Option<Value>>,
    model_catalog: RwLock<Option<(crate::ProviderKind, Vec<crate::provider::ModelInfo>)>>,
}
impl LiveControls {
    pub(crate) async fn open(
        &self,
        run: Uuid,
        agent: Arc<Agent>,
        subagents: Arc<SubagentRuntime>,
        cancel: CancellationToken,
    ) {
        *self.inventory.write().await = serde_json::to_value(agent.tool_inventory()).ok();
        *self.active.write().await = Some(Active {
            run,
            agent,
            subagents,
            cancel,
            slots: Arc::new(Semaphore::new(8)),
        });
    }
    pub(crate) async fn close(&self) -> bool {
        let mut owned = self.active.write().await;
        let Some(active) = owned.as_ref() else {
            return true;
        };
        active.cancel.cancel();
        let observed = matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(15),
                active.slots.clone().acquire_many_owned(8),
            )
            .await,
            Ok(Ok(_))
        );
        if observed {
            *owned = None;
        }
        observed
    }

    async fn active(&self, run: Option<Uuid>) -> Result<Active> {
        let active = self
            .active
            .read()
            .await
            .clone()
            .context("no live executor; submit a run before controlling its resources")?;
        ensure!(run.is_none_or(|run| run == active.run), "stale control run");
        ensure!(!active.cancel.is_cancelled(), "run controls closed");
        active.agent.check_current_policy()?;
        Ok(active)
    }
    pub(crate) async fn inspect(&self, run: Option<Uuid>, section: &str) -> Result<Value> {
        if section == "host_resources" {
            let mut summary = crate::host_resources::inspect()?;
            // Per-session observation grants must not enumerate other owners.
            if let Some(object) = summary.as_object_mut() {
                object.remove("reservations");
                object.remove("database");
            }
            return Ok(summary);
        }
        let active = self.active(run).await?;
        let _permit = active
            .slots
            .clone()
            .try_acquire_owned()
            .context("runtime controls busy")?;
        let value = match section {
            "tools" => serde_json::to_value(active.agent.tool_inventory())?,
            "models" => serde_json::to_value(
                tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    active.agent.models(false),
                )
                .await??,
            )?,
            "policy" => active.agent.operator_policy()?,
            "todos" => active.agent.operator_todos().await?,
            "subagents" => serde_json::to_value(active.subagents.list().await)?,
            "terminals" => {
                let (manager, _) = active.agent.plain_terminals()?;
                serde_json::to_value(manager.list().await?)?
            }
            "workflows" => {
                serde_json::to_value(crate::workflow::discover(active.agent.workspace(), None)?)?
            }
            _ => anyhow::bail!(
                "unknown control section; use tools, policy, todos, subagents, terminals or workflows"
            ),
        };
        ensure!(
            serde_json::to_vec(&value)?.len() <= 1024 * 1024,
            "control result exceeds bounded frame; use individual tool inspection"
        );
        Ok(json!({"run_id":active.run,"section":section,"value":value}))
    }
    pub(crate) async fn inspect_or_idle(
        &self,
        run: Option<Uuid>,
        section: &str,
        config: &crate::Config,
        workspace: &std::path::Path,
    ) -> Result<Value> {
        let result = if self.active.read().await.is_some() {
            self.inspect(run, section).await?
        } else {
            idle::inspect(self, section, config, workspace).await?
        };
        if section == "models" {
            let models = serde_json::from_value(result["value"].clone())?;
            *self.model_catalog.write().await = Some((config.provider.clone(), models));
        }
        Ok(result)
    }
    /// Discovery is optional; an absent catalog is unknown, not unsupported.
    pub(crate) async fn known_model(
        &self,
        config: &crate::Config,
    ) -> Option<crate::provider::ModelInfo> {
        self.model_catalog
            .read()
            .await
            .as_ref()
            .filter(|(provider, _)| provider == &config.provider)
            .and_then(|(_, models)| {
                models
                    .iter()
                    .find(|model| model.id == config.model)
                    .cloned()
            })
    }
    pub(crate) async fn execute(
        &self,
        owner: ManagedSessionOwner,
        session: Uuid,
        command: RuntimeCommand,
    ) -> Result<Value> {
        tool::execute(self, owner, session, command).await
    }
    pub(crate) async fn terminal(
        &self,
        run: Uuid,
        id: Uuid,
        operation: TerminalOperation,
    ) -> Result<Value> {
        terminal::execute(self, run, id, operation).await
    }
}
