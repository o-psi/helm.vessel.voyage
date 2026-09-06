use std::{collections::BTreeMap, fs, net::TcpListener, process::Command};

use helm::config::{Config, McpServerConfig};

fn command(root: &std::path::Path, config: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_helm"));
    command
        .env_clear()
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("APPDATA", root.join("data"))
        .env("LOCALAPPDATA", root.join("data"))
        .env("HELM_DIAGNOSTIC_KEY", "process-only-provider-canary")
        .current_dir(root)
        .arg("--config")
        .arg(config);
    command
}

#[test]
fn config_diagnostics_conceal_bindings_without_network_or_source_changes() {
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let secrets = [
        "env-canary-雪\n\"\\",
        "mcp-canary-λ\t\"",
        "redact-canary-🦀\r\n",
    ];
    let config = Config {
        model: "retained-model".into(),
        api_key_env: "HELM_DIAGNOSTIC_KEY".into(),
        base_url: Some(format!("http://{}", listener.local_addr().unwrap())),
        env: BTreeMap::from([
            ("SERVICE_TOKEN".into(), secrets[0].into()),
            ("SHORT".into(), "x".into()),
            ("EMPTY".into(), String::new()),
        ]),
        redact_values: vec![secrets[2].into(), "y".into(), String::new()],
        mcp_servers: BTreeMap::from([(
            "fixture-server".into(),
            McpServerConfig {
                command: "must-not-be-launched".into(),
                args: vec!["retained-argument".into()],
                env: BTreeMap::from([
                    ("MCP_TOKEN".into(), secrets[1].into()),
                    ("SHORT".into(), "z".into()),
                ]),
            },
        )]),
        ..Config::default()
    };
    let path = directory.path().join("helm.toml");
    let original = toml::to_string_pretty(&config).unwrap();
    fs::write(&path, &original).unwrap();
    for operation in ["config", "doctor"] {
        let output = command(directory.path(), &path)
            .arg(operation)
            .output()
            .unwrap();
        assert!(output.status.success(), "diagnostic failed");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        for secret in secrets.into_iter().chain(["process-only-provider-canary"]) {
            for text in [&stdout, &stderr] {
                assert!(!text.contains(secret));
                assert!(!text.contains(&serde_json::to_string(secret).unwrap()));
            }
        }
        assert!(stdout.contains("HELM_DIAGNOSTIC_KEY"));
        assert!(stdout.contains("not_probed"));
        assert!(stdout.contains("fixture-server"));
        if operation == "config" {
            assert!(stdout.contains("cannot restore secret bindings"));
            let displayed: Config = toml::from_str(&stdout).unwrap();
            assert_eq!(displayed.model, "retained-model");
            assert_eq!(displayed.base_url, config.base_url);
            assert_eq!(displayed.max_tokens, config.max_tokens);
            assert_eq!(displayed.access, Some(config.access_mode()));
            assert!(displayed.env.values().all(|value| value == "[REDACTED]"));
            assert!(
                displayed
                    .redact_values
                    .iter()
                    .all(|value| value == "[REDACTED]")
            );
            let server = &displayed.mcp_servers["fixture-server"];
            assert_eq!(server.command, "must-not-be-launched");
            assert_eq!(server.args, ["retained-argument"]);
            assert!(server.env.values().all(|value| value == "[REDACTED]"));
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(loaded.env, config.env);
        assert_eq!(
            loaded.mcp_servers["fixture-server"].env,
            config.mcp_servers["fixture-server"].env
        );
        assert_eq!(loaded.redact_values, config.redact_values);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
fn config_diagnostic_errors_do_not_echo_secret_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("helm.toml");
    let cases = [
        ("[env]\nTOKEN = \"invalid-canary-雪\" trailing", None),
        ("redact_values = [\"invalid-canary-雪\", 42]", None),
        (
            "[mcp_servers.test.env]\nTOKEN = [\"invalid-canary-雪\"]",
            None,
        ),
        ("", Some("invalid-canary-雪")),
        ("", Some("env.TOKEN=[\"invalid-canary-雪\"]")),
        ("", Some("redact_values=[\"invalid-canary-雪\",42]")),
    ];
    for (contents, assignment) in cases {
        fs::write(&path, contents).unwrap();
        for operation in ["config", "doctor"] {
            let mut invocation = command(directory.path(), &path);
            if let Some(assignment) = assignment {
                invocation.arg("--set").arg(assignment);
            }
            let output = invocation.arg(operation).output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(!stderr.contains("invalid-canary"));
            assert!(stderr.contains("configuration input or override is invalid or unavailable"));
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }
}

#[test]
fn config_diagnostics_conceal_runtime_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("helm.toml");
    fs::write(&path, "model = \"source-model\"\n").unwrap();
    let output = command(directory.path(), &path)
        .args([
            "--set",
            "env.SERVICE_TOKEN=override-env-canary",
            "--set",
            "redact_values=[\"override-redact-canary\"]",
            "--set",
            "mcp_servers.fixture={command=\"unused\",env={MCP_TOKEN=\"override-mcp-canary\"}}",
            "--set",
            "model=override-model",
            "config",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stdout.contains("canary"));
    assert!(!stderr.contains("canary"));
    let displayed: Config = toml::from_str(&stdout).unwrap();
    assert_eq!(displayed.model, "override-model");
    assert_eq!(displayed.env["SERVICE_TOKEN"], "[REDACTED]");
    assert_eq!(displayed.redact_values, ["[REDACTED]"]);
    assert_eq!(
        displayed.mcp_servers["fixture"].env["MCP_TOKEN"],
        "[REDACTED]"
    );
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "model = \"source-model\"\n"
    );
}
