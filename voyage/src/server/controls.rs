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
type CachedModels = (
    [u8; 32],
    std::time::Instant,
    Vec<crate::provider::ModelInfo>,
);
#[derive(Default)]
pub struct LiveControls {
    retained: Arc<RwLock<Option<retained::Retained>>>,
    active: RwLock<Option<Active>>,
    inventory: RwLock<Option<Value>>,
    model_catalog: RwLock<Option<CachedModels>>,
    catalog_refresh: tokio::sync::Mutex<()>,
    catalog_scheduled: std::sync::atomic::AtomicBool,
    catalog_attempt: RwLock<Option<([u8; 32], std::time::Instant)>>,
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
            "subagents_archive" => {
                serde_json::to_value(active.subagents.list_archived(None, 100).await?)?
            }
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
        // Native discovery must use next-turn config, not an active agent whose
        // provider/account was captured at admission.
        let context = crate::provider::inference_context(config).await;
        let result = if section == "models" {
            idle::inspect(self, section, config, workspace).await?
        } else if self.active.read().await.is_some() {
            self.inspect(run, section).await?
        } else {
            idle::inspect(self, section, config, workspace).await?
        };
        if section == "models" {
            let models = serde_json::from_value(result["value"].clone())?;
            // OAuth refresh may rotate the credential during discovery. Never
            // bind a response to a different login/config than the one requested.
            if let Some(context) = context
                && crate::provider::inference_context(config).await == Some(context)
            {
                *self.model_catalog.write().await =
                    Some((context, std::time::Instant::now(), models));
            } else {
                *self.model_catalog.write().await = None;
                anyhow::bail!("model discovery account context changed; retry discovery");
            }
        }
        Ok(result)
    }
    /// Discovery is optional; an absent catalog is unknown, not unsupported.
    pub(crate) async fn known_model(
        &self,
        config: &crate::Config,
    ) -> Option<crate::provider::ModelInfo> {
        let context = crate::provider::inference_context(config).await?;
        self.model_catalog
            .read()
            .await
            .as_ref()
            .filter(|(key, at, _)| {
                *key == context && at.elapsed() < std::time::Duration::from_secs(300)
            })
            .and_then(|(_, _, models)| models.iter().find(|m| m.id == config.model).cloned())
    }
    /// Best-effort bounded metadata discovery, independent of opening /models.
    /// Failures leave support unknown; explicit requests still receive transport
    /// validation and the provider remains the authority. No inference probes.
    pub(crate) async fn resolve_model(
        &self,
        config: &crate::Config,
        workspace: &std::path::Path,
    ) -> Option<crate::provider::ModelInfo> {
        let _refresh = self.catalog_refresh.lock().await;
        if let Some(model) = self.known_model(config).await {
            return Some(model);
        }
        let context = crate::provider::inference_context(config).await?;
        if self
            .catalog_attempt
            .read()
            .await
            .as_ref()
            .is_some_and(|(key, at)| {
                *key == context && at.elapsed() < std::time::Duration::from_secs(30)
            })
        {
            return None;
        }
        *self.catalog_attempt.write().await = Some((context, std::time::Instant::now()));
        let _ = self
            .inspect_or_idle(None, "models", config, workspace)
            .await;
        self.known_model(config).await
    }
    /// Snapshot polling must never wait on provider network I/O or enqueue an
    /// unbounded discovery backlog. The resolver supplies TTL/backoff checks.
    pub(crate) fn schedule_resolution(
        self: &Arc<Self>,
        config: crate::Config,
        workspace: std::path::PathBuf,
    ) {
        use std::sync::atomic::Ordering;
        if self.catalog_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let controls = self.clone();
        tokio::spawn(async move {
            controls.resolve_model(&config, &workspace).await;
            controls.catalog_scheduled.store(false, Ordering::Release);
        });
    }
    pub(crate) async fn inference_resolution(
        &self,
        run: Uuid,
    ) -> Option<voyage_protocol::inference::InferenceResolution> {
        let active = self.active.read().await.clone()?;
        if active.run != run {
            return None;
        }
        active.agent.inference_resolution().await
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
