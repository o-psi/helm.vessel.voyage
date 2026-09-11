//! Operator controls reuse the live authorized registry and current run accounting.
use super::*;
impl Agent {
    pub(crate) async fn operator_lease(
        &self,
        session: uuid::Uuid,
        run: uuid::Uuid,
    ) -> anyhow::Result<crate::completion::runtime::ReadinessLease> {
        let coordinator = self
            .completion_coordinator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing coordinator"))?;
        let scope =
            crate::completion::runtime::RunHandle::resume(coordinator.clone(), session, run)
                .await?;
        let gate = self
            .completion_gate
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing completion gate"))?;
        Ok(gate.lease(&scope).await?)
    }
    pub(crate) fn operator_terminals(
        &self,
    ) -> Option<(crate::tools::ProcessTool, Arc<crate::policy::Policy>)> {
        self.tools
            .terminals()
            .map(|manager| (manager, self.context.policy.clone()))
    }
    pub(crate) async fn operator_todos(&self) -> anyhow::Result<serde_json::Value> {
        self.check_current_policy()?;
        let gate = self
            .completion_gate
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("task store unavailable"))?;
        Ok(serde_json::to_value(gate.todos.snapshot().await?)?)
    }
    pub(crate) async fn operator_tool(
        &self,
        session: uuid::Uuid,
        run: uuid::Uuid,
        cancel: CancellationToken,
        name: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<String> {
        self.check_current_policy()?;
        anyhow::ensure!(!cancel.is_cancelled(), "run controls closed");
        anyhow::ensure!(
            !(name == "process" && arguments["action"] == "write"),
            "human terminal input requires the private terminal channel"
        );
        let mut context = self.context.clone();
        context.cancellation = cancel;
        context.execution_id = run;
        if let Some(coordinator) = &self.completion_coordinator {
            context.completion = Some(
                crate::completion::runtime::RunHandle::resume(coordinator.clone(), session, run)
                    .await?,
            );
        }
        self.tools
            .run_extension_lifecycle("run_start", &context)
            .await;
        let result = self.tools.execute(name, arguments, &context).await;
        if result.is_ok() && !context.cancellation.is_cancelled() {
            self.tools
                .run_extension_lifecycle("run_finish", &context)
                .await;
        }
        result.map_err(Into::into)
    }
    pub(crate) fn operator_arguments(&self, value: &serde_json::Value) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self
                .context
                .redactor
                .contains_secret(&serde_json::to_string(value)?),
            "operator arguments contain a configured secret; use private input instead"
        );
        Ok(())
    }
    pub(crate) fn operator_policy(&self) -> anyhow::Result<serde_json::Value> {
        self.check_current_policy()?;
        Ok(
            serde_json::json!({"access":self.context.policy.access_mode(),"workspace":self.workspace(),"tools":self.tool_inventory()}),
        )
    }
}
