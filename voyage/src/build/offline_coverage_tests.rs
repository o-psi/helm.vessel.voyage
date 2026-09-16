use super::*;

#[tokio::test]
async fn offline_tool_assembly_applies_static_read_only_filter_without_external_services() {
    let root = tempfile::tempdir().unwrap();
    for access_mode in [
        AccessMode::ReadOnly,
        AccessMode::Approval,
        AccessMode::Unrestricted,
    ] {
        let config = Config {
            access: Some(access_mode),
            github_enabled: false,
            mcp_servers: Default::default(),
            ..Default::default()
        };
        let policy = Policy::new(&config, root.path().to_owned()).unwrap();
        let tools = build_tools(&config, None, None, None, &policy)
            .await
            .unwrap();
        let names = tools
            .definitions()
            .into_iter()
            .map(|d| d.name)
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "read_file"));
        assert!(!names.iter().any(|name| name == "github"));
        if access_mode == AccessMode::ReadOnly {
            assert!(!names.iter().any(|name| name == "write_file"));
            assert!(!names.iter().any(|name| name == "shell"));
        } else {
            assert!(names.iter().any(|name| name == "write_file"));
            assert!(names.iter().any(|name| name == "shell"));
        }
    }
}

#[test]
fn explicit_tool_environment_and_redactions_are_derived_without_global_mutation() {
    let config = Config {
        inherit_env: vec![],
        env: std::collections::BTreeMap::from([(
            "OFFLINE_KEY".into(),
            "synthetic-fixture-value".into(),
        )]),
        redact_values: vec!["second-fixture-value".into()],
        github_enabled: false,
        api_key_required: false,
        ..Default::default()
    };
    let environment = tool_environment(&config);
    assert_eq!(environment.len(), 1);
    assert_eq!(environment["OFFLINE_KEY"], "synthetic-fixture-value");
    let redactor = redactor(&config);
    assert!(redactor.contains_secret("synthetic-fixture-value"));
    assert!(redactor.contains_secret("second-fixture-value"));
    assert!(
        !redactor
            .redact("synthetic-fixture-value second-fixture-value")
            .contains("fixture-value")
    );
}
