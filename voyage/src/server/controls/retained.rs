//! A root PTY manager belongs to the session across turns; subordinate PTYs do not.
use super::*;
#[derive(Clone)]
pub(super) struct Retained {
    pub id: Uuid,
    pub run: Uuid,
    pub manager: crate::tools::ProcessTool,
    pub policy: Arc<crate::policy::Policy>,
    origins: Vec<Arc<crate::policy::Policy>>,
}
impl LiveControls {
    pub(crate) async fn close_for_command(
        &self,
        owner: &ManagedSessionOwner,
        command: &RuntimeCommand,
    ) -> Result<()> {
        let id = match command {
            RuntimeCommand::Clear { command_id, .. }
            | RuntimeCommand::Compact { command_id, .. }
            | RuntimeCommand::SetModel { command_id, .. }
            | RuntimeCommand::SetInference { command_id, .. }
            | RuntimeCommand::SetAccountInference { command_id, .. }
            | RuntimeCommand::SetAccess { command_id, .. }
            | RuntimeCommand::Configure { command_id, .. }
            | RuntimeCommand::Archive { command_id, .. }
            | RuntimeCommand::Delete { command_id, .. }
            | RuntimeCommand::Branch { command_id, .. }
            | RuntimeCommand::Relinquish { command_id, .. } => *command_id,
            _ => anyhow::bail!("unsupported terminal teardown command"),
        };
        if owner.process_receipt(id).await?.is_some() {
            return Ok(());
        }
        match command {
            RuntimeCommand::Clear {
                confirm_session_id, ..
            }
            | RuntimeCommand::Delete {
                confirm_session_id, ..
            } => ensure!(
                *confirm_session_id == owner.session_id(),
                "session confirmation mismatch"
            ),
            RuntimeCommand::Compact { retain, .. } => ensure!(
                (1..=100000).contains(retain),
                "retain must be 1..100000 messages"
            ),
            _ => {}
        }
        let (expected, expiry) = match command {
            RuntimeCommand::Clear {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Compact {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::SetInference {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::SetAccountInference {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::SetModel {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::SetAccess {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Configure {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Archive {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Delete {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Branch {
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Relinquish {
                expected_revision,
                expires_at_ms,
                ..
            } => (*expected_revision, *expires_at_ms),
            _ => anyhow::bail!("unsupported terminal teardown command"),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis();
        ensure!(
            u128::from(expiry) > now && u128::from(expiry) - now <= 300000,
            "invalid command deadline"
        );
        if matches!(command, RuntimeCommand::SetAccess { .. }) {
            owner.check_access_revision(expected).await?;
        } else {
            ensure!(
                owner.snapshot().await?.revision == expected,
                "session revision conflict"
            );
        }
        self.shutdown_retained(owner).await
    }
    pub(crate) async fn retained_tool(&self) -> Option<crate::tools::ProcessTool> {
        self.retained
            .read()
            .await
            .as_ref()
            .map(|entry| entry.manager.clone())
    }
    pub(crate) async fn retain_root(
        &self,
        owner: &ManagedSessionOwner,
        run: Uuid,
        agent: &Agent,
    ) -> Result<()> {
        let Some((manager, policy)) = agent.operator_terminals() else {
            return Ok(());
        };
        let mut retained = self.retained.write().await;
        if let Some(existing) = retained.as_mut() {
            ensure!(
                existing.manager.same_manager(&manager),
                "root terminal manager changed without observed cleanup"
            );
            manager.retain_policy(policy.clone())?;
            existing.run = run;
            ensure!(
                existing.origins.len() < 256,
                "retained terminal authority capacity reached; close terminals before another run"
            );
            existing.origins.push(policy.clone());
            existing.policy = policy;
            return Ok(());
        }
        manager.retain_policy(policy.clone())?;
        let id = Uuid::new_v4();
        owner
            .session_resource_adopt(id, run, "root_terminals".into())
            .await?;
        *retained = Some(Retained {
            id,
            run,
            manager,
            origins: vec![policy.clone()],
            policy,
        });
        let retained = self.retained.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let current = retained.read().await.clone();
                let Some(entry) = current.filter(|entry| entry.id == id) else {
                    return;
                };
                if entry
                    .origins
                    .iter()
                    .all(|policy| policy.check_current().is_ok())
                {
                    continue;
                }
                let mut current = retained.write().await;
                if current.as_ref().is_none_or(|entry| entry.id != id) {
                    return;
                }
                let observed = entry
                    .manager
                    .shutdown(std::time::Duration::from_secs(10))
                    .await
                    .observation_complete;
                if observed && owner.session_resource_closed(id).await.is_ok() {
                    *current = None;
                }
                return;
            }
        });
        Ok(())
    }
    pub(crate) async fn shutdown_retained(&self, owner: &ManagedSessionOwner) -> Result<()> {
        let mut retained = self.retained.write().await;
        if let Some(entry) = retained.as_ref() {
            let report = entry
                .manager
                .shutdown(std::time::Duration::from_secs(10))
                .await;
            ensure!(
                report.observation_complete,
                "session terminal cleanup unconfirmed"
            );
            owner.session_resource_closed(entry.id).await?;
        }
        *retained = None;
        Ok(())
    }
    pub(super) async fn retained(&self, run: Uuid) -> Result<Retained> {
        let entry = self
            .retained
            .read()
            .await
            .clone()
            .context("session has no retained terminal manager")?;
        ensure!(entry.run == run, "stale terminal run binding");
        entry.policy.check_current()?;
        for origin in &entry.origins {
            origin.check_current()?;
        }
        Ok(entry)
    }
}
