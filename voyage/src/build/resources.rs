use super::*;
#[derive(Default)]
pub struct ManagedResourceState {
    closed: bool,
    pub terminals: Vec<crate::tools::ProcessTool>,
    pub shells: Vec<crate::tools::ManagedShell>,
    pub browser: Option<Arc<crate::browser::BrowserBroker>>,
    pub mcp: Vec<Arc<crate::tools::mcp::McpServer>>,
}
pub struct ManagedResources(std::sync::Mutex<ManagedResourceState>, bool);
impl Default for ManagedResources {
    fn default() -> Self {
        Self(std::sync::Mutex::new(ManagedResourceState::default()), true)
    }
}
impl ManagedResources {
    pub fn local() -> Self {
        Self(
            std::sync::Mutex::new(ManagedResourceState::default()),
            cfg!(target_os = "linux"),
        )
    }
    pub fn register_browser(&self, browser: Arc<crate::browser::BrowserBroker>) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("managed resources poisoned"))?;
        anyhow::ensure!(!state.closed, "resource admission closed");
        state.browser = Some(browser);
        Ok(())
    }
    pub fn register(&self, tools: &mut ToolRegistry) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("managed resources poisoned"))?;
        anyhow::ensure!(!state.closed, "managed resource admission closed");
        state.terminals.retain(|resource| !resource.can_retire());
        state.shells.retain(|resource| !resource.can_retire());
        state.mcp.retain(|server| !server.can_retire());
        anyhow::ensure!(
            state.terminals.len() < 1024,
            "managed resource limit reached"
        );
        for server in tools.mcp_servers() {
            if !state.mcp.iter().any(|other| Arc::ptr_eq(other, &server)) {
                state.mcp.push(server);
            }
        }
        if let Some(terminals) = tools.terminals() {
            state.terminals.push(terminals);
        }
        // Replace only an already-authorized shell; never reintroduce a filtered tool.
        if self.1 && tools.definitions().iter().any(|tool| tool.name == "shell") {
            let shell = crate::tools::ManagedShell::new();
            tools.register(shell.clone());
            state.shells.push(shell);
        }
        Ok(())
    }
    pub fn start_mcp(
        &self,
        start: impl FnOnce() -> Result<Arc<crate::tools::mcp::McpServer>>,
    ) -> Result<Arc<crate::tools::mcp::McpServer>> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("managed resources poisoned"))?;
        anyhow::ensure!(!state.closed, "managed resource admission closed");
        let server = start()?;
        state.mcp.push(server.clone());
        Ok(server)
    }
    pub fn has_owned_work(&self) -> Result<bool> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("managed resources poisoned"))?;
        Ok(state.terminals.iter().any(|item| item.has_owned_work())
            || state.shells.iter().any(|item| item.has_owned_work()))
    }
    pub async fn shutdown_observed(&self, require_session_observation: bool) -> Result<()> {
        let retained = self.close()?;
        let observed = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let terminals = futures_util::future::join_all(
                retained
                    .terminals
                    .iter()
                    .map(|item| item.shutdown(std::time::Duration::from_secs(5))),
            )
            .await;
            let shells = futures_util::future::join_all(
                retained
                    .shells
                    .iter()
                    .map(|item| item.shutdown(std::time::Duration::from_secs(5))),
            )
            .await;
            let mcp =
                futures_util::future::join_all(retained.mcp.iter().map(|item| item.shutdown()))
                    .await;
            let browser = retained
                .browser
                .as_ref()
                .map(|b| b.finish_run().unwrap_or(false))
                .unwrap_or(true);
            browser
                && terminals.iter().all(|item| item.observation_complete)
                && shells.iter().all(|item| item.observation_complete)
                && mcp.iter().all(Result::is_ok)
                && (!require_session_observation
                    || retained.mcp.iter().all(|server| server.observed()))
        })
        .await
        .unwrap_or(false);
        anyhow::ensure!(
            observed,
            "runtime cleanup unconfirmed; execution remains blocked"
        );
        Ok(())
    }
    pub fn close(&self) -> Result<ManagedResourceState> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("managed resources poisoned"))?;
        state.closed = true;
        Ok(ManagedResourceState {
            closed: true,
            terminals: state.terminals.clone(),
            shells: state.shells.clone(),
            mcp: state.mcp.clone(),
            browser: state.browser.clone(),
        })
    }
}
