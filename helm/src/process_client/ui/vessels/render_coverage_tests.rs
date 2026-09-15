use super::*;
use crate::process_client::connections::{Metadata, Workspace};
#[test]
fn metadata_describes_expiry_capabilities_and_all_provider_readiness_states() {
    for (expiry, expected) in [
        (None, "not advertised"),
        (Some(1), "ACCESS EXPIRED"),
        (
            Some((chrono::Utc::now() + chrono::Duration::hours(2)).timestamp_millis() as u64),
            "expires within 24 hours",
        ),
    ] {
        let metadata_value = Metadata {
            version: Some("synthetic-version".into()),
            rights: vec!["inspect".into()],
            expires_at_ms: expiry,
            workspaces: [Some(true), Some(false), None]
                .into_iter()
                .map(|provider_ready| Workspace {
                    id: Uuid::new_v4(),
                    name: "Synthetic workspace".into(),
                    path: "/synthetic-path".into(),
                    provider_ready,
                })
                .collect(),
            features: vec!["synthetic-feature".into()],
            grant_revision: Some(7),
        };
        let mut rows = Vec::new();
        metadata(&mut rows, &metadata_value);
        let text = rows
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            expected,
            "synthetic-version",
            "inspect",
            "synthetic-feature",
            "Some(7)",
            "/synthetic-path",
            "provider ready: ready",
            "authorize on execution host",
            "not yet checked",
        ] {
            assert!(text.contains(needle), "{text}");
        }
    }
    let mut rows = Vec::new();
    metadata(&mut rows, &Metadata::default());
    assert!(rows[0].text.contains("not advertised"));
}
#[test]
fn wrapping_keeps_unicode_columns_bounded_and_scope_is_explicit() {
    assert_eq!(wrap("ab界cd", 4), ["ab界", "cd"]);
    assert_eq!(wrap("abc", 0), [""]);
    assert_eq!(wrap("界a", 1), ["a"]);
    let session = Uuid::new_v4();
    let text = scope(&Scope::Session {
        session_id: session,
    });
    assert!(text.contains(&session.to_string()));
    assert!(text.contains("no new conversations"));
    assert!(
        scope(&Scope::Workspaces {
            workspace_ids: vec![Uuid::new_v4(), Uuid::new_v4()]
        })
        .contains("2 authorized")
    );
}
