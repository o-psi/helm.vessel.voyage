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
        self.status = "Saving your conversation...".into();
        tokio::spawn(async move {
            let result = crate::process_client::export::markdown(&client, target.session, &path)
                .await
                .map(|value| {
                    format!(
                        "# Conversation saved\n\n{}\n\n{} messages exported.",
                        super::safe(value["path"].as_str().unwrap_or_default()),
                        value["messages"].as_u64().unwrap_or_default()
                    )
                })
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
