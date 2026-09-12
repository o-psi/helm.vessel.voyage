//! Private account observations never enter the conversation/update stream.
use super::*;

impl App {
    pub(super) fn refresh_account_usage(&mut self, refresh: bool) -> Result<()> {
        ensure!(
            self.accounts.usage_reply.is_none(),
            "Usage refresh already pending"
        );
        let p = self
            .accounts
            .picker
            .as_mut()
            .context("Account view closed")?;
        ensure!(!p.busy && !p.disconnected, "Wait for the current request");
        let account = p
            .choices()
            .get(p.selected)
            .and_then(|(_, b)| b.clone())
            .context("Select an available account to observe usage")?;
        p.usage_requested.insert(account.account_id);
        let workspace = p.workspace.clone();
        let client = self.clients[p.route].clone();
        let id = p.id;
        let (tx, rx) = oneshot::channel();
        self.accounts.usage_reply = Some((id, rx));
        p.notice = if refresh {
            "Refreshing usage…"
        } else {
            "Reading cached usage…"
        }
        .into();
        tokio::spawn(async move {
            let result = async {
                let value = tokio::time::timeout(
                    Duration::from_secs(20),
                    client.request(VesselCommand::AccountUsage {
                        workspace,
                        account: account.clone(),
                        refresh,
                    }),
                )
                .await??;
                let observation: AccountUsageObservation = serde_json::from_value(value)?;
                ensure!(observation.account == account, "Usage identity mismatch");
                Ok(Reply::Usage(observation))
            }
            .await;
            let _ = tx.send(result.map_err(|_: anyhow::Error| anyhow::anyhow!(
                "Usage unavailable. Cached observation retained; account selection still works. F5 retries explicitly."
            )));
        });
        Ok(())
    }

    pub(super) fn set_default_account(&mut self) -> Result<()> {
        let p = self
            .accounts
            .picker
            .as_mut()
            .context("Account view closed")?;
        ensure!(!p.busy && !p.disconnected, "Wait for the current request");
        ensure!(
            !p.default_change_pending,
            "Reopen to inspect the earlier default change; do not repeat it"
        );
        ensure!(
            p.catalogue.can_set_default,
            "Only the host owner can change the default account"
        );
        let account = p
            .choices()
            .get(p.selected)
            .and_then(|(_, b)| b.clone())
            .context("Select an available account first")?;
        let workspace = p.workspace.clone();
        let expected_revision = p.catalogue.default_revision;
        let client = self.clients[p.route].clone();
        let id = p.id;
        let (tx, rx) = oneshot::channel();
        self.accounts.reply = Some((id, rx));
        p.busy = true;
        p.default_change_pending = true;
        p.notice = "Saving default for new voyages…".into();
        tokio::spawn(async move {
            let result = async {
                let value = client
                    .request(VesselCommand::AccountSetDefault {
                        command_id: Uuid::new_v4(),
                        workspace: workspace.clone(),
                        account: account.clone(),
                        expected_revision,
                    })
                    .await?;
                let actual: AccountBinding =
                    serde_json::from_value(value["default_account"].clone())?;
                let _revision = value["default_revision"]
                    .as_u64()
                    .context("Missing default revision")?;
                ensure!(actual == account, "Default identity mismatch");
                let catalogue: Catalogue = serde_json::from_value(
                    client
                        .request(VesselCommand::Accounts {
                            workspace,
                            transport: None,
                        })
                        .await?,
                )?;
                ensure!(
                    catalogue.accounts.len() <= 128 && catalogue.connections.len() <= 64,
                    "Account catalogue exceeds limits"
                );
                Ok(Reply::DefaultAccount(catalogue))
            }
            .await;
            // No automatic retry: reopen to observe the host's actual default first.
            let _ = tx.send(result.map_err(|_: anyhow::Error| anyhow::anyhow!(
                "Default change not confirmed. Close and reopen to inspect it before trying another change."
            )));
        });
        Ok(())
    }
}

pub(super) fn usage_text(observation: Option<&AccountUsageObservation>, now: i64) -> String {
    let Some(observation) = observation else {
        return "Usage unknown · F5 Refresh".into();
    };
    let status = match observation.refresh_status {
        AccountUsageRefreshStatus::NeverObserved => "Not observed",
        AccountUsageRefreshStatus::Available => "Observed",
        AccountUsageRefreshStatus::Unsupported => "Usage not supported",
        AccountUsageRefreshStatus::SignInRequired => "Sign in required",
        AccountUsageRefreshStatus::RateLimited => "Refresh rate limited",
        AccountUsageRefreshStatus::Unavailable => "Refresh unavailable",
        AccountUsageRefreshStatus::InvalidResponse => "Unrecognized usage response",
    };
    let Some(snapshot) = &observation.snapshot else {
        return format!("{status} · usage unknown · F5 Refresh");
    };
    let age = now.saturating_sub(snapshot.fetched_at).max(0);
    let stale = age > 300 || observation.refresh_status != AccountUsageRefreshStatus::Available;
    let mut rows = vec![format!(
        "Codex allowance · {status} · {}{}s ago",
        if stale { "STALE · " } else { "" },
        age
    )];
    for window in snapshot.windows.iter().take(2) {
        let name = match window.window_seconds {
            Some(s) if s > 0 && s % 86400 == 0 => format!("{} day", s / 86400),
            Some(s) if s > 0 && s % 3600 == 0 => format!("{} hour", s / 3600),
            Some(s) => format!("{s} second"),
            None => "Unknown-duration".into(),
        };
        let reset = match window.resets_at {
            Some(t) if t <= now => "reset time passed; refresh to observe".into(),
            Some(t) => chrono::DateTime::from_timestamp(t, 0)
                .map(|t| format!("resets {} UTC", t.format("%m-%d %H:%M")))
                .unwrap_or_else(|| "reset unknown".into()),
            None => "reset unknown".into(),
        };
        let percent =
            if window.used_percent.is_finite() && (0.0..=100.0).contains(&window.used_percent) {
                format!("{:.0}% used", window.used_percent)
            } else {
                "usage unknown".into()
            };
        rows.push(format!("{name}: {percent} · {reset}"));
    }
    if snapshot.windows.is_empty() {
        rows.push("Usage windows unknown".into());
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_stale_and_elapsed_reset_do_not_imply_refill() {
        assert!(usage_text(None, 1000).contains("unknown"));
        let observation = AccountUsageObservation {
            account: super::super::tests::binding(),
            capability_revision: 1,
            snapshot: Some(AccountUsageSnapshot {
                fetched_at: 10,
                windows: vec![AccountUsageWindow {
                    kind: UsageWindowKind::Primary,
                    used_percent: 100.0,
                    window_seconds: Some(18000),
                    resets_at: Some(20),
                }],
            }),
            refresh_status: AccountUsageRefreshStatus::Unavailable,
            attempted_at: Some(999),
        };
        let text = usage_text(Some(&observation), 1000);
        assert!(text.contains("STALE") && text.contains("5 hour: 100% used"));
        assert!(text.contains("reset time passed") && text.contains("Refresh unavailable"));
    }
}
