use super::{App, observe::Update, state::Target};
use anyhow::{Result, ensure};
impl App {
    pub(super) fn export(&mut self, target: Target, path: &str) -> Result<()> {
        ensure!(
            !path.trim().is_empty(),
            "export requires a local destination path"
        );
        let path = std::path::PathBuf::from(path);
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let incarnation = self.views[&target].process.incarnation;
        self.status = format!("Exporting voyage {}", target.session);
        tokio::spawn(async move {
            let result = crate::process_client::export::markdown(&client, target.session, &path)
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
