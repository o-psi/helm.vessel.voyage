//! Private, bounded provider observations. No raw provider text is retained.
use super::*;
use serde_json::Value;

impl Registry {
    pub fn default_account(&self) -> Result<(u64, Option<AccountBinding>)> {
        self.transaction(|db| Ok((db.default_revision, db.default_account.clone())))
    }
    pub fn set_default_account(
        &self,
        command_id: Uuid,
        workspace: &std::path::Path,
        expected: u64,
        binding: AccountBinding,
    ) -> Result<(u64, AccountBinding)> {
        self.transaction(|db| {
            ensure!(!command_id.is_nil(), "default command ID required");
            if let Some((prior_workspace, revision, prior, result)) =
                db.default_commands.get(&command_id)
            {
                ensure!(
                    *revision == expected
                        && *prior == binding
                        && prior_workspace == &workspace.to_string_lossy(),
                    "default command ID conflict"
                );
                return Ok((*result, prior.clone()));
            }
            ensure!(
                db.default_revision == expected,
                "default account changed; refresh before applying"
            );
            ensure!(
                db.default_commands.len() < 256,
                "default command retention full"
            );
            checked(db, &binding)?;
            db.default_revision = db
                .default_revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("default revision exhausted"))?;
            db.default_account = Some(binding.clone());
            db.default_commands.insert(
                command_id,
                (
                    workspace.to_string_lossy().into_owned(),
                    expected,
                    binding.clone(),
                    db.default_revision,
                ),
            );
            Ok((db.default_revision, binding))
        })
    }
    pub fn usage_cached(&self, binding: &AccountBinding) -> Result<AccountUsageObservation> {
        self.transaction(|db| {
            let revision = checked(db, binding)?.descriptor.capability_revision;
            Ok(db
                .usage
                .get(&binding.account_id)
                .filter(|u| u.account == *binding && u.capability_revision == revision)
                .cloned()
                .unwrap_or(AccountUsageObservation {
                    account: binding.clone(),
                    capability_revision: revision,
                    snapshot: None,
                    refresh_status: if binding.transport == Transport::ChatgptOauth {
                        AccountUsageRefreshStatus::NeverObserved
                    } else {
                        AccountUsageRefreshStatus::Unsupported
                    },
                    attempted_at: None,
                }))
        })
    }
    pub fn publish_usage(&self, mut observation: AccountUsageObservation) -> Result<()> {
        self.transaction(|db| {
            ensure!(
                checked(db, &observation.account)?
                    .descriptor
                    .capability_revision
                    == observation.capability_revision,
                "account authority changed"
            );
            if db
                .usage
                .get(&observation.account.account_id)
                .is_some_and(|old| old.attempted_at > observation.attempted_at)
            {
                return Ok(());
            }
            if observation.refresh_status != AccountUsageRefreshStatus::Available {
                observation.snapshot = db
                    .usage
                    .get(&observation.account.account_id)
                    .filter(|u| {
                        u.account == observation.account
                            && u.capability_revision == observation.capability_revision
                    })
                    .and_then(|u| u.snapshot.clone());
            }
            db.usage.insert(observation.account.account_id, observation);
            Ok(())
        })
    }
}

pub(crate) fn parse(value: &Value, fetched_at: i64) -> Result<AccountUsageSnapshot> {
    let limits = value
        .get("rate_limit")
        .filter(|v| v.is_object())
        .ok_or_else(|| anyhow::anyhow!("usage limits absent"))?;
    let mut windows = Vec::new();
    for (key, kind) in [
        ("primary_window", UsageWindowKind::Primary),
        ("secondary_window", UsageWindowKind::Secondary),
    ] {
        let Some(window) = limits.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let used_percent = window
            .get("used_percent")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
            .ok_or_else(|| anyhow::anyhow!("invalid usage percentage"))?;
        let window_seconds = match window.get("limit_window_seconds").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.as_u64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid usage duration"))?,
            ),
        };
        let resets_at = match window.get("reset_at").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid usage reset"))?,
            ),
        };
        windows.push(AccountUsageWindow {
            kind,
            used_percent,
            window_seconds,
            resets_at,
        });
    }
    ensure!(!windows.is_empty(), "usage windows absent");
    Ok(AccountUsageSnapshot {
        fetched_at,
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_allowlist_and_missing_semantics() {
        let v = serde_json::json!({"private_identity":"never retained", "rate_limit":{"primary_window":{"used_percent":42.0,"limit_window_seconds":18000,"reset_at":123}}});
        let s = parse(&v, 90).unwrap();
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.windows[0].resets_at, Some(123));
        assert!(
            !serde_json::to_string(&s)
                .unwrap()
                .contains("private_identity")
        );
        for v in [
            serde_json::json!({}),
            serde_json::json!({"rate_limit":{}}),
            serde_json::json!({"rate_limit":{"primary_window":{"used_percent":101}}}),
            serde_json::json!({"rate_limit":{"primary_window":{}}}),
        ] {
            assert!(parse(&v, 1).is_err());
        }
        let s = parse(
            &serde_json::json!({"rate_limit":{"primary_window":{"used_percent":0}}}),
            2,
        )
        .unwrap();
        assert_eq!(s.windows[0].window_seconds, None);
        assert_eq!(s.windows[0].resets_at, None);
    }
}
