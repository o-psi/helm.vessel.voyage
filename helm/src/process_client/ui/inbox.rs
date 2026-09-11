//! A user-requested, passive overview. Never changes selection, composer or decisions.
use super::{App, observe::Update, state::Target};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::notifications::{
    DestinationRecord, InboxPage, Notification, NotificationOperation, NotificationReceipt,
    ProducerCursor, ReceiptState,
};

const HELP: &str = "# Notification inbox\n\n/inbox destinations\n/inbox list DESTINATION [AFTER]\n/inbox open DESTINATION EVENT\n/inbox seen DESTINATION EVENT\n/inbox dismiss DESTINATION EVENT\n\nUse helm connect inbox for configure, accept, test, watch and revoke.\n\nOpen only reads owner metadata. It never switches voyages, opens URLs/programs, or responds to a request. Dismiss does not resolve an owner decision. Esc returns to your unchanged composer.";

pub(super) fn is_command(text: &str) -> bool {
    text == "/inbox" || text.starts_with("/inbox ")
}

fn parse(text: &str) -> Result<Option<NotificationOperation>> {
    ensure!(text.len() <= 512, "inbox command exceeds 512 bytes");
    let words = text.split_whitespace().collect::<Vec<_>>();
    let uuid = |s: &str| s.parse::<Uuid>().context("expected destination/event UUID");
    Ok(Some(match words.as_slice() {
        ["/inbox"] => return Ok(None),
        ["/inbox", "destinations"] => NotificationOperation::Destinations,
        ["/inbox", "list", destination] => NotificationOperation::Inbox {
            destination_id: uuid(destination)?,
            after: 0,
            limit: 50,
        },
        ["/inbox", "list", destination, after] => NotificationOperation::Inbox {
            destination_id: uuid(destination)?,
            after: after
                .parse::<u64>()
                .context("expected unsigned inbox cursor")?,
            limit: 50,
        },
        ["/inbox", "open" | "inspect", destination, event] => NotificationOperation::Open {
            destination_id: uuid(destination)?,
            event_id: uuid(event)?,
        },
        ["/inbox", state @ ("seen" | "dismiss"), destination, event] => {
            NotificationOperation::Receipt {
                destination_id: uuid(destination)?,
                event_id: uuid(event)?,
                state: if *state == "seen" {
                    ReceiptState::Seen
                } else {
                    ReceiptState::Dismissed
                },
            }
        }
        _ => anyhow::bail!("unknown inbox command; /inbox lists supported operations"),
    }))
}

fn metadata(notification: &Notification) -> String {
    let mut output = format!(
        "Event: {}\nKind: {:?}{}\nVoyage: {}\nRun: {}\nIncarnation: {}\nSource event: {}\nCreated (UTC epoch ms): {}\nExpires (UTC epoch ms): {}\nDecision reference: {}\n",
        notification.event_id,
        notification.kind,
        if matches!(
            notification.kind,
            voyage_protocol::notifications::NotificationKind::Test
        ) {
            " (synthetic test, not a run outcome)"
        } else {
            ""
        },
        notification.session_id,
        notification
            .run_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".into()),
        notification
            .incarnation
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown".into()),
        notification.source_event_id,
        notification.created_at_ms,
        notification.expires_at_ms,
        notification
            .decision_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".into()),
    );
    if let Some(budget) = &notification.budget {
        output.push_str(&format!(
            "Budget: {:?} {:?} · scope {} · revision {} · ledger sequence {}\n",
            budget.scope,
            budget.threshold,
            budget.scope_id,
            budget.revision,
            budget.ledger_sequence
        ));
    }
    output.push_str("Execution cleanup is not implied by this notification.\n");
    output
}

/// Parse only typed metadata. Ignore any unsolicited free-form server fields.
fn render(operation: &NotificationOperation, value: Value) -> Result<String> {
    let mut output = String::from("# Notification inbox\n\n");
    match operation {
        NotificationOperation::Destinations => {
            if !value["local_recipient_id"].is_null() {
                let id: Uuid = serde_json::from_value(value["local_recipient_id"].clone())
                    .context("invalid local recipient identity")?;
                output.push_str(&format!("Local recipient identity: {id}\n\n"));
            }
            let records: Vec<DestinationRecord> =
                serde_json::from_value(value["destinations"].clone())
                    .context("invalid destination list")?;
            ensure!(
                records.len() <= 256,
                "destination list exceeds display bound"
            );
            if records.is_empty() {
                output.push_str("No destinations visible to this connection.\n");
            }
            for record in records {
                let d = record.destination;
                output.push_str(&format!("Destination: {}\nSource: {}\nRecipient grant: {}\nExpires (UTC epoch ms): {}\nAccepted: {}\nRevoked: {}\n\n", d.id, d.source_session_id, d.recipient_grant_id, d.expires_at_ms, record.accepted_at_ms.is_some(), record.revoked_at_ms.is_some()));
            }
        }
        NotificationOperation::Inbox { destination_id, .. } => {
            let page: InboxPage =
                serde_json::from_value(value["page"].clone()).context("invalid inbox page")?;
            let producer: ProducerCursor = serde_json::from_value(value["producer"].clone())
                .context("invalid producer status")?;
            let deferred = value["attention_deferred"]
                .as_bool()
                .context("missing attention status")?;
            let expires = value["destination_expires_at_ms"]
                .as_u64()
                .context("missing destination expiry")?;
            output.push_str(&format!("Destination expires (UTC epoch ms): {expires}\nAttention deferred by quiet hours: {deferred}\n"));
            if let Some(error) = producer.error {
                output.push_str(&format!(
                    "Producer status: {error:?} — delivery may be incomplete; not task success.\n"
                ));
            }
            ensure!(
                page.entries.len() <= 100,
                "inbox page exceeds display bound"
            );
            if page.entries.is_empty() {
                output.push_str("No visible notifications in this page.\n");
            }
            for entry in page.entries {
                output.push_str(&format!(
                    "Sequence: {} · {:?}\n{}\n",
                    entry.receipt.sequence,
                    entry.receipt.state,
                    metadata(&entry.notification)
                ));
            }
            output.push_str(&format!("Next cursor: {} · More: {}\n/inbox list {} {}\n\nReading does not mark seen. Rescan from 0 to refresh receipts.\n", page.next_after, page.has_more, destination_id, page.next_after));
        }
        NotificationOperation::Receipt { .. } => {
            let receipt: NotificationReceipt =
                serde_json::from_value(value).context("invalid notification receipt")?;
            output.push_str(&format!("Event: {}\nReceipt: {:?}\n\nThis is destination receipt state, not owner resolution or proof of human reading.\n", receipt.event_id, receipt.state));
        }
        NotificationOperation::Open { event_id, .. } => {
            let status = match value["status"].as_str() {
                Some("current") => "Current owner reference",
                Some("resolved_or_expired") => "Owner request resolved or expired",
                Some("stale") => "Stale owner reference; nothing may be answered here",
                Some("expired_or_revoked") => "Notification expired or destination revoked",
                Some("decision_authority_unavailable") => "Current decision authority unavailable",
                Some("unavailable") => "Owner unavailable; no current decision established",
                _ => anyhow::bail!("unknown notification open status"),
            };
            output.push_str(status);
            output.push_str("\n\n");
            let notification = if !value["notification"].is_null() {
                let notification: Notification =
                    serde_json::from_value(value["notification"].clone())
                        .context("invalid notification reference")?;
                ensure!(
                    notification.event_id == *event_id,
                    "notification event mismatch"
                );
                output.push_str(&metadata(&notification));
                Some(notification)
            } else {
                None
            };
            if value["status"] == "current" && !value["decision"].is_null() {
                let decision: super::state::Decision =
                    serde_json::from_value(value["decision"].clone())
                        .context("invalid owner decision reference")?;
                let reference = notification
                    .as_ref()
                    .context("decision has no notification reference")?;
                ensure!(
                    reference.decision_id == Some(decision.decision_id)
                        && reference.run_id == Some(decision.run_id)
                        && reference.incarnation == Some(decision.incarnation),
                    "owner decision does not match notification reference"
                );
                output.push_str(&format!("\nAuthorized owner decision: {}\nRun: {}\nIncarnation: {}\nExpires (UTC epoch ms): {}\n", decision.decision_id, decision.run_id, decision.incarnation, decision.expires_at_ms));
            }
            output.push_str("\nNo response sent. To review or answer, explicitly select the owner voyage and use its existing Questions and permissions flow. That flow must revalidate the current request.\n");
        }
        _ => anyhow::bail!("operation is not supported by the inbox overview"),
    }
    output.push_str("\nEsc returns without changing your composer or selected voyage.");
    Ok(output)
}

impl App {
    pub(super) fn inbox_command(&mut self, target: Target, text: &str) -> Result<()> {
        let operation = parse(text)?;
        let Some(operation) = operation else {
            self.views
                .get_mut(&target)
                .context("selected voyage unavailable")?
                .panel = Some(HELP.into());
            return Ok(());
        };
        ensure!(
            self.clients.available(target.route),
            "Vessel disconnected; reconnect before reading inbox"
        );
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let incarnation = self.views[&target].process.incarnation;
        self.status = "Reading notification metadata; no owner action will be taken...".into();
        let job = tokio::spawn(async move {
            let result = crate::process_client::inbox::request(&client, operation.clone())
                .await.and_then(|value| render(&operation, value))
                .map_err(|_| "Notification operation unavailable. No automatic retry; inspect receipt state before retrying a mutation.".to_string());
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
    use serde_json::json;

    #[test]
    fn only_explicit_bounded_metadata_operations_parse() {
        let id = Uuid::nil();
        assert!(parse("/inbox").unwrap().is_none());
        for text in [
            "/inbox approve",
            "/inbox open https://example.com",
            "/inboxish",
            "/inbox list",
            "/inbox destinations extra",
        ] {
            assert!(parse(text).is_err());
        }
        assert!(parse(&format!("/inbox list {id} -1")).is_err());
        assert!(parse(&format!("/inbox open {id} {id} extra")).is_err());
        assert!(matches!(
            parse(&format!("/inbox dismiss {id} {id}")).unwrap(),
            Some(NotificationOperation::Receipt {
                state: ReceiptState::Dismissed,
                ..
            })
        ));
        assert!(!is_command("/inboxish"));
        assert!(parse(&format!("/inbox {}", "x".repeat(512))).is_err());
    }

    #[test]
    fn open_ignores_unsolicited_text_and_rejects_unknown_status() {
        let operation = NotificationOperation::Open {
            destination_id: Uuid::nil(),
            event_id: Uuid::nil(),
        };
        let text = render(&operation, json!({"status":"unavailable", "summary":"SECRET\u{001b}[2J", "url":"https://example.com"})).unwrap();
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("https://"));
        assert!(text.contains("No response sent"));
        assert!(render(&operation, json!({"status":"approve"})).is_err());
    }

    #[test]
    fn help_keeps_composer_selection_and_has_no_browser_or_owner_effect() {
        let fixture = super::super::account_test_support::Fixture::new();
        let mut app = super::super::accounts::app_tests::app(fixture.0.path());
        let target = Target {
            route: app.clients.first_route().unwrap(),
            session: Uuid::new_v4(),
        };
        let mut view = super::super::state::View::new(
            serde_json::from_value(json!({
                "session_id":target.session,"incarnation":Uuid::new_v4(),
                "workspace":"/synthetic-workspace","state":"live"
            }))
            .unwrap(),
        );
        view.draft.text = "/inbox".into();
        app.views.insert(target, view);
        app.selected = Some(target);
        app.send().unwrap();
        assert_eq!(app.selected, Some(target));
        assert_eq!(app.views[&target].draft.text, "/inbox");
        assert!(
            app.views[&target]
                .panel
                .as_ref()
                .unwrap()
                .contains("never switches voyages")
        );
        assert!(app.views[&target].pending.is_none());
        assert!(app.route_tasks.is_empty());
        assert!(super::super::account_test_support::browsers().is_empty());
    }

    #[test]
    fn producer_gap_and_quiet_hours_are_visible() {
        let operation = NotificationOperation::Inbox {
            destination_id: Uuid::nil(),
            after: 0,
            limit: 50,
        };
        let text = render(
            &operation,
            json!({
                "page":{"entries":[],"next_after":0,"has_more":false},
                "producer":{"after":0,"error":"gap"},"attention_deferred":true,
                "destination_expires_at_ms":100
            }),
        )
        .unwrap();
        assert!(text.contains("Gap"));
        assert!(text.contains("not task success"));
        assert!(text.contains("quiet hours: true"));
    }

    #[test]
    fn synthetic_metadata_never_reports_a_successful_run() {
        let id = Uuid::nil();
        let notification: Notification = serde_json::from_value(json!({
            "event_id":id,"session_id":id,"run_id":null,"incarnation":id,"kind":"test",
            "source_event_id":id,"created_at_ms":1,"expires_at_ms":2,"decision_id":null
        }))
        .unwrap();
        assert!(metadata(&notification).contains("synthetic test, not a run outcome"));
    }
}
