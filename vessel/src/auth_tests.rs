use super::*;
#[test]
fn token_status_exposes_only_safe_state_not_account_identity() {
    for authenticated in [false, true] {
        for refreshable in [false, true] {
            for expires_at in [None, Some(123456789)] {
                let status = voyage_runtime::provider::TokenStatus {
                    authenticated,
                    refreshable,
                    expires_at,
                    account_id: Some("private-fixture-account".into()),
                };
                assert_eq!(
                    token_status_json(&status),
                    serde_json::json!({"authenticated":authenticated,"expires_at":expires_at,"refreshable":refreshable})
                );
                assert!(
                    !token_status_json(&status)
                        .to_string()
                        .contains("private-fixture")
                );
            }
        }
    }
}
