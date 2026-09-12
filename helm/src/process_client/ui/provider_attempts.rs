//! Explicit bounded history reads; no execution or continuation side effects.
use super::{App, observe::Update, state::Target};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::{provider_attempt::ProviderAttempt, vessel::VoyageCommand};

fn parse(text: &str) -> Result<(Option<Uuid>, u64)> {
    ensure!(text.len() <= 128, "attempt command is too long");
    let words: Vec<_> = text.split_whitespace().collect();
    Ok(match words.as_slice() {
        ["/attempts"] => (None, 0),
        ["/attempts", run] => (Some(run.parse().context("expected run UUID")?), 0),
        ["/attempts", "all", offset] => (None, offset.parse()?),
        ["/attempts", run, offset] => (Some(run.parse()?), offset.parse()?),
        _ => anyhow::bail!("Use /attempts [RUN_UUID [OFFSET]] or /attempts all OFFSET"),
    })
}
fn render(value: serde_json::Value, run: Option<Uuid>) -> Result<String> {
    let rows = value["attempts"]
        .as_array()
        .context("attempt history unavailable")?;
    ensure!(rows.len() <= 16, "oversized diagnostic page");
    let mut out = String::from("# Provider attempts\n\n");
    for row in rows {
        let id: Uuid = serde_json::from_value(row["run_id"].clone())?;
        let attempt: ProviderAttempt = serde_json::from_value(row["attempt"].clone())?;
        if let Some(id) = attempt.retry.upstream_request_id.as_deref().filter(|id| {
            id.len() <= 128
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
        }) {
            out.push_str(&format!("Upstream request {id}\n"));
        }
        out.push_str(&format!(
            "Run {id} · Request {}\n{}\n\n",
            attempt.request_id,
            attempt.summary()
        ));
    }
    if rows.is_empty() {
        out.push_str("No recorded attempts on this page. Older runs may have no diagnostics.\n");
    }
    if value["legacy_runs_without_diagnostics"]
        .as_u64()
        .unwrap_or(0)
        > 0
    {
        out.push_str("Some runs have no recorded diagnostic detail.\n");
    }
    if value["has_more"] == true {
        let next = value["next_offset"]
            .as_u64()
            .context("invalid history cursor")?;
        out.push_str(&format!(
            "Next page: /attempts {} {next}\n",
            run.map(|id| id.to_string()).unwrap_or("all".into())
        ));
    }
    out.push_str("\nPages are fresh observations; running attempts may change between reads.\nPartial output: review saved text and tool outcomes before explicitly sending a continuation. Unknown tool effects require reconciliation. This view never retries a run. Esc returns to the conversation.");
    Ok(out)
}
impl App {
    pub(super) fn provider_attempts_command(&mut self, target: Target, text: &str) -> Result<()> {
        let (run_id, offset) = parse(text)?;
        ensure!(self.clients.available(target.route), "Vessel disconnected");
        let view = &self.views[&target];
        let incarnation = view.process.incarnation;
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let job = tokio::spawn(async move {
            let result = client.voyage(target.session, incarnation,
                VoyageCommand::ProviderAttempts { run_id, offset, limit: 16, expected_revision: None })
                .await.and_then(|value| render(value, run_id))
                .map_err(|_| "Provider history unavailable; reconnect or reload from /attempts. Older Vessels may not support this view.".into());
            let _ = sender
                .send(Update::Control {
                    target,
                    incarnation,
                    result,
                })
                .await;
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(job);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_read_commands_and_legacy_history() {
        assert_eq!(parse("/attempts").unwrap(), (None, 0));
        assert_eq!(parse("/attempts all 16").unwrap(), (None, 16));
        assert!(parse("/attempts retry").is_err());
        assert!(parse("/attempts all -1").is_err());
        let text = render(
            serde_json::json!({"attempts":[],"legacy_runs_without_diagnostics":1}),
            None,
        )
        .unwrap();
        assert!(text.contains("no recorded diagnostic detail"));
    }
}
