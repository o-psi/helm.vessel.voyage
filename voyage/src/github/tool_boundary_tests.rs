//! Live model-facing adapter boundaries. All effects refuse before constructing
//! a remote request or opening the private publication journal.
use super::*;
fn object() -> Value {
    json!({"repository":{"owner":"example","name":"project"},"kind":"issue","number":7})
}
fn draft() -> Value {
    json!({"object":object(),"action":{"kind":"comment","body":"exact offline body"}})
}
#[test]
fn live_registry_definition_has_only_narrow_reviewed_github_actions() {
    let definition = GithubTool.definition();
    assert_eq!(definition.name, "github");
    assert_eq!(definition.input_schema["additionalProperties"], false);
    assert_eq!(
        definition.input_schema["properties"]["action"]["enum"],
        json!([
            "read", "logs", "prepare", "inspect", "list", "publish", "cancel"
        ])
    );
    assert!(
        definition
            .description
            .contains("Never repeat an uncertain send")
    );
    assert!(!definition.description.contains("HELM_GITHUB_TOKEN"));
}
#[tokio::test]
async fn malformed_model_arguments_do_not_echo_untrusted_private_input() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    for value in [
        json!(null),
        json!([]),
        json!({"action":"CLI_PRIVATE_CANARY"}),
        json!({"action":"inspect","id":"CLI_PRIVATE_CANARY"}),
        json!({"action":"publish","id":uuid::Uuid::new_v4(),"digest":"private","approval":true}),
        json!({"action":"list","offset":4294967296u64}),
        json!({"action":"read","request":{"object":object(),"section":{"kind":"operator-admin"},"page":1}}),
    ] {
        let error = GithubTool.execute(value, &context).await.unwrap_err();
        assert!(matches!(&error, ToolError::InvalidArguments(_)));
        assert!(!error.to_string().contains("CLI_PRIVATE_CANARY"));
        assert!(!error.to_string().contains("operator-admin"));
    }
}
#[tokio::test]
async fn all_private_actions_require_a_real_owning_run_even_with_explicit_credential() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.github = Some(crate::github::Credential(std::sync::Arc::new(
        zeroize::Zeroizing::new("OFFLINE_PRIVATE_TOKEN".into()),
    )));
    let id = uuid::Uuid::new_v4();
    for value in [
        json!({"action":"prepare","draft":draft()}),
        json!({"action":"inspect","id":id}),
        json!({"action":"list"}),
        json!({"action":"publish","id":id,"digest":"a".repeat(64)}),
        json!({"action":"cancel","id":id,"digest":"a".repeat(64)}),
    ] {
        let error = GithubTool.execute(value, &context).await.unwrap_err();
        assert!(matches!(&error, ToolError::Denied(_)));
        assert!(error.to_string().contains("active owning voyage/run"));
        assert!(!error.to_string().contains("OFFLINE_PRIVATE_TOKEN"));
    }
}
#[tokio::test]
async fn read_and_logs_cannot_use_credential_when_local_capability_is_disabled() {
    let root = tempfile::tempdir().unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.github = Some(crate::github::Credential(std::sync::Arc::new(
        zeroize::Zeroizing::new("OFFLINE_PRIVATE_TOKEN".into()),
    )));
    for value in [
        json!({"action":"read","request":{"object":object(),"section":{"kind":"details"},"page":1}}),
        json!({"action":"logs","object":object(),"job":1}),
    ] {
        let error = GithubTool.execute(value, &context).await.unwrap_err();
        assert!(matches!(&error, ToolError::Denied(_)));
        assert!(error.to_string().contains("current local authority"));
        assert!(!error.to_string().contains("OFFLINE_PRIVATE_TOKEN"));
    }
}
#[tokio::test]
async fn neither_model_approval_claim_nor_digest_authorizes_private_publication() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let id = uuid::Uuid::new_v4();
    for field in [
        "approved",
        "owner",
        "actor",
        "policy_digest",
        "administrator",
        "command",
    ] {
        let mut value = json!({"action":"publish","id":id,"digest":"a".repeat(64)});
        value[field] = json!(true);
        assert!(matches!(
            GithubTool.execute(value, &context).await.unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
    }
}
#[tokio::test]
async fn empty_or_missing_read_inputs_refuse_before_remote_discovery() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    for value in [
        json!({"action":"read"}),
        json!({"action":"logs","object":object()}),
        json!({"action":"prepare","draft":{"object":object(),"action":{"kind":"comment"}}}),
        json!({"action":"inspect"}),
        json!({"action":"publish","id":uuid::Uuid::new_v4()}),
        json!({"action":"cancel","digest":"a".repeat(64)}),
    ] {
        assert!(matches!(
            GithubTool.execute(value, &context).await.unwrap_err(),
            ToolError::InvalidArguments(_)
        ));
    }
}
