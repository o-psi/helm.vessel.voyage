//! Real entry-point regressions: no provider, supervisor, or parent environment changes.
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output, Stdio},
};

struct Cli(tempfile::TempDir);
impl Cli {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_helm"));
        command
            .env_clear()
            .env("HOME", self.0.path())
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .current_dir(self.0.path())
            .stdin(Stdio::null());
        for (key, dir) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            let path = self.0.path().join(dir);
            fs::create_dir_all(&path).unwrap();
            command.env(key, path);
        }
        // Preserve only LLVM's output destination, not credentials or host configuration.
        if let Some(path) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", path);
        }
        command.args(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
    fn fails(&self, args: &[&str], message: &str) {
        let out = self.run(args);
        assert!(!out.status.success(), "unexpected success: {args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(message),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn configuration_provider_and_access_overrides_reach_real_main() {
    let cli = Cli::new();
    for (provider, model) in [
        ("openai-responses", "gpt-5"),
        ("openai-chat", "gpt-5"),
        ("anthropic", "claude-sonnet-4-0"),
        ("chatgpt-oauth", "gpt-5"),
    ] {
        let text = cli.ok(&["--provider", provider, "config"]);
        let config: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(config["provider"].as_str(), Some(provider));
        assert_eq!(config["model"].as_str(), Some(model));
        assert!(text.contains("Secret values are concealed"));
        let custom = cli.ok(&["--provider", provider, "--model", "offline-model", "config"]);
        assert!(custom.contains("model = \"offline-model\""));
    }
    for access in ["read-only", "approval", "unrestricted"] {
        let text = cli.ok(&[
            "--access",
            access,
            "--verbose",
            "--log-format",
            "json",
            "config",
        ]);
        let config: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(config["access"].as_str(), Some(access));
    }
    for (legacy, access) in [
        ("always", "approval"),
        ("on-risk", "approval"),
        ("never", "unrestricted"),
    ] {
        let text = cli.ok(&["--approval", legacy, "config"]);
        assert!(text.contains(&format!("access = \"{access}\"")));
    }
}

#[test]
fn diagnostic_config_errors_conceal_input_and_cli_rejects_conflicts() {
    let cli = Cli::new();
    for assignment in [
        "unknown=private-canary",
        "max_tokens=private-canary",
        "private-canary",
    ] {
        let out = cli.run(&["--set", assignment, "config"]);
        assert!(!out.status.success());
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains("input details concealed"));
        assert!(!stderr.contains("private-canary"));
    }
    fs::write(
        cli.0.path().join("bad.toml"),
        "api_key = 'private-canary'\n[",
    )
    .unwrap();
    cli.fails(
        &["--config", "bad.toml", "doctor"],
        "input details concealed",
    );
    cli.ok(&["--config", "absent.toml", "config"]);
    cli.fails(
        &["--access", "approval", "--approval", "never", "config"],
        "cannot be used with",
    );
    cli.fails(
        &["--model", "offline", "connect", "list"],
        "connected voyages resolve execution configuration",
    );
    cli.fails(
        &[
            "--policy-profile",
            "balanced",
            "--policy-revision",
            "1",
            "--policy-digest",
            "unused",
            "config",
        ],
        "policy selection requires",
    );
}

#[test]
fn doctor_reports_missing_credentials_without_contacting_provider() {
    let cli = Cli::new();
    for provider in [
        "openai-responses",
        "openai-chat",
        "anthropic",
        "chatgpt-oauth",
    ] {
        let value = cli.json(&["--provider", provider, "--access", "read-only", "doctor"]);
        assert_eq!(value["provider"]["id"], provider);
        assert_eq!(value["provider_ready"], false);
        assert_eq!(value["provider_credential_present"], false);
        assert_eq!(value["status"], "action_required");
        assert_eq!(value["access"], "read-only");
        assert!(
            value["sessions_directory"]
                .as_str()
                .unwrap()
                .starts_with(cli.0.path().to_str().unwrap())
        );
        if provider == "chatgpt-oauth" {
            assert_eq!(value["native_chatgpt_oauth"]["authenticated"], false);
            assert_eq!(value["native_chatgpt_oauth"]["refreshable"], false);
        } else {
            assert!(value["native_chatgpt_oauth"].is_null());
        }
    }
}

#[test]
fn generated_cli_documents_and_local_presets_are_offline() {
    let cli = Cli::new();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let completion = cli.ok(&["completions", shell]);
        assert!(completion.contains("helm"));
        assert!(completion.contains("workflow"));
    }
    assert!(cli.ok(&["manpage"]).contains("doctor"));
    assert!(cli.ok(&["--help"]).contains("Commands:"));
    assert!(cli.ok(&["--version"]).starts_with("helm "));
    assert!(cli.ok(&["local-provider", "presets"]).contains("ollama"));
}

#[test]
fn private_policy_lifecycle_is_revision_checked_and_exportable() {
    let cli = Cli::new();
    let create = cli.json(&["policy", "create", "offline"]);
    assert_eq!(create["snapshot"]["revision"], 1);
    let inspect = cli.json(&["policy", "inspect", "offline"]);
    assert_eq!(inspect["profile"]["rules"]["access"], "approval");
    let digest = inspect["digest"].as_str().unwrap();
    let preview = cli.json(&[
        "policy",
        "preview",
        "offline",
        "--revision",
        "1",
        "--digest",
        digest,
    ]);
    assert!(preview.is_object());
    let exported = cli.ok(&["policy", "export", "offline"]);
    let path = cli.0.path().join("policy.json");
    fs::write(&path, &exported).unwrap();
    let path = path.to_str().unwrap();
    assert_eq!(
        cli.json(&["policy", "import", "imported", "--input", path])["snapshot"]["revision"],
        1
    );
    assert_eq!(
        cli.json(&["policy", "duplicate", "offline", "copy"])["snapshot"]["name"],
        "copy"
    );
    let edit = cli.json(&[
        "policy",
        "edit",
        "offline",
        "--input",
        path,
        "--expected-revision",
        "1",
    ]);
    assert_eq!(edit["snapshot"]["revision"], 2);
    let stale = cli.run(&["policy", "delete", "offline", "--expected-revision", "1"]);
    assert!(!stale.status.success());
    assert_eq!(
        cli.json(&["policy", "inspect", "offline"])["profile"]["revision"],
        2
    );
    cli.ok(&["policy", "delete", "offline", "--expected-revision", "2"]);
    assert!(cli.json(&["policy", "inspect", "offline"])["profile"]["rules"].is_null());
    let listed = cli.json(&["policy", "list", "--limit", "100"]);
    assert!(
        listed["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "imported")
    );
}

const WORKFLOW: &str = r#"schema_version = 1
id = "offline-check"
version = "1"
description = "Offline subprocess fixture"
prompt = "Review {{subject}} with {{count}} checks; enabled={{enabled}}."
[parameters.subject]
type = "string"
required = true
[parameters.count]
type = "integer"
default = 2
minimum = 1
maximum = 5
[parameters.enabled]
type = "boolean"
default = true
"#;

#[test]
fn workflow_administration_and_typed_preview_do_not_launch_a_voyage() {
    let cli = Cli::new();
    let directory = cli.0.path().join("workflows");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("offline-check.toml"), WORKFLOW).unwrap();
    let directory = directory.to_str().unwrap();
    let list = cli.json(&["workflow", "--user-directory", directory, "--json", "list"]);
    assert_eq!(list["workflows"].as_array().unwrap().len(), 1);
    let inspect = cli.json(&[
        "workflow",
        "--user-directory",
        directory,
        "--json",
        "inspect",
        "offline-check",
    ]);
    assert_eq!(inspect["document"]["id"], "offline-check");
    let valid = cli.json(&[
        "workflow",
        "--user-directory",
        directory,
        "--json",
        "validate",
        "offline-check",
    ]);
    assert_eq!(valid["valid"], true);
    let preview = cli.json(&[
        "workflow",
        "--user-directory",
        directory,
        "--json",
        "preview",
        "offline-check",
        "--input",
        "subject=parser",
    ]);
    assert!(preview.to_string().contains("parser"));
    assert!(preview.to_string().contains("enabled=true"));
    for input in [
        "count=6",
        "count=bad",
        "enabled=maybe",
        "unknown=x",
        "malformed",
    ] {
        assert!(
            !cli.run(&[
                "workflow",
                "--user-directory",
                directory,
                "preview",
                "offline-check",
                "--input",
                "subject=parser",
                "--input",
                input
            ])
            .status
            .success(),
            "{input}"
        );
    }
    assert!(
        !cli.run(&[
            "workflow",
            "--user-directory",
            directory,
            "preview",
            "offline-check"
        ])
        .status
        .success()
    );
    cli.fails(
        &["workflow", "--json", "run", "offline-check"],
        "--json is for list",
    );
    cli.fails(
        &[
            "workflow",
            "--user-directory",
            directory,
            "inspect",
            "missing",
        ],
        "workflow not found",
    );
}

#[test]
fn inference_allowance_preview_commit_retry_audit_and_history_are_offline() {
    let cli = Cli::new();
    let initial = cli.json(&["inference", "inspect"]);
    assert_eq!(initial["unit"], "local_inference_dispatch_permits");
    let operation = "76c49f94-745d-4d27-8af3-cf7dd778104e";
    let args = [
        "inference",
        "configure",
        "--operation",
        operation,
        "--expected-revision",
        "0",
        "--limit",
        "10",
        "--warning",
        "8",
        "--reason",
        "offline regression",
    ];
    let preview = cli.json(&args);
    assert_eq!(preview["committed"], false);
    let mut commit = args.to_vec();
    commit.push("--confirm");
    let applied = cli.json(&commit);
    assert_eq!(applied["committed"], true);
    let duplicate = cli.json(&commit);
    assert!(duplicate.to_string().contains(operation));
    let audit = cli.json(&["inference", "audit"]);
    assert!(audit.to_string().contains("offline regression"));
    let history = cli.json(&[
        "inference",
        "history",
        "--from",
        "2020-01-01T00:00:00Z",
        "--until",
        "2030-01-01T00:00:00Z",
    ]);
    assert!(history.is_object());
    assert!(
        !cli.run(&[
            "inference",
            "configure",
            "--operation",
            "386128e0-47b8-43f8-a9c2-6612f4d5d280",
            "--expected-revision",
            "0",
            "--unlimited",
            "--reason",
            "stale",
            "--confirm"
        ])
        .status
        .success()
    );
}
#[test]
fn onboarding_cli_only_publishes_explicit_reviewed_guidance() {
    let cli = Cli::new();
    fs::write(
        cli.0.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    let inspected = cli.json(&["onboard", "inspect", "--json"]);
    assert!(inspected.to_string().contains("Cargo.toml"));
    let draft = cli.0.path().join("review.md");
    cli.ok(&[
        "onboard",
        "preview",
        "--output",
        draft.to_str().unwrap(),
        "--confirm",
    ]);
    let bytes = fs::read(&draft).unwrap();
    assert!(!bytes.is_empty());
    assert!(!cli.0.path().join("AGENTS.md").exists());
    cli.fails(
        &[
            "onboard",
            "preview",
            "--output",
            draft.to_str().unwrap(),
            "--confirm",
        ],
        "never replaced",
    );
    cli.fails(
        &[
            "onboard",
            "accept",
            "--draft",
            draft.to_str().unwrap(),
            "--sha256",
            &"0".repeat(64),
            "--output",
            "AGENTS.md",
        ],
        "digest",
    );
    assert!(!cli.0.path().join("AGENTS.md").exists());
}

#[test]
fn policy_defaults_cli_initializes_enables_previews_and_activates_exact_workspace() {
    let cli = Cli::new();
    let dir = cli.0.path().join("defaults");
    let profiles = cli.0.path().join("profiles");
    let output = cli.0.path().join("enabled.toml");
    let anchor = cli.json(&[
        "policy",
        "defaults",
        "init",
        "--directory",
        dir.to_str().unwrap(),
    ]);
    let id = anchor["store_id"].as_str().unwrap();
    cli.ok(&[
        "policy",
        "defaults",
        "enable",
        "--directory",
        dir.to_str().unwrap(),
        "--store-id",
        id,
        "--output",
        output.to_str().unwrap(),
    ]);
    let profile = cli.json(&[
        "--policy-directory",
        profiles.to_str().unwrap(),
        "policy",
        "inspect",
        "restricted",
    ]);
    let digest = profile["digest"].as_str().unwrap();
    cli.ok(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "set",
        "--global",
        "--profile-directory",
        profiles.to_str().unwrap(),
        "restricted",
        "--revision",
        "1",
        "--digest",
        digest,
        "--expected-revision",
        "0",
    ]);
    let preview = cli.json(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "preview",
    ]);
    let confirmation = preview["preview"]["transition_digest"]
        .as_str()
        .or_else(|| preview["transition_digest"].as_str())
        .unwrap();
    cli.ok(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "activate",
        "--expected-revision",
        "0",
        "--confirm",
        confirmation,
    ]);
    let listing = cli.json(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "list",
    ]);
    assert!(listing.to_string().contains("restricted"));
    cli.ok(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "inspect",
        "--global",
    ]);
    cli.ok(&[
        "--config",
        output.to_str().unwrap(),
        "policy",
        "defaults",
        "clear",
        "--global",
        "--expected-revision",
        "1",
    ]);
    assert!(output.exists());
}

#[test]
fn inference_history_rejects_invalid_windows_and_group_cursors_without_dispatch() {
    let cli = Cli::new();
    cli.json(&["inference", "inspect"]);
    for args in [
        vec![
            "inference",
            "history",
            "--from",
            "2030-01-01T00:00:00Z",
            "--until",
            "2020-01-01T00:00:00Z",
        ],
        vec![
            "inference",
            "history",
            "--from",
            "2020-01-01T00:00:00Z",
            "--until",
            "2030-01-01T00:00:00Z",
            "--offset",
            "1",
        ],
        vec![
            "inference",
            "history",
            "--from",
            "2020-01-01T00:00:00Z",
            "--until",
            "2030-01-01T00:00:00Z",
            "--group",
            "invalid",
        ],
        vec!["inference", "inspect", "--limit", "0"],
        vec![
            "inference",
            "inspect",
            "--session",
            "36be6361-1a1d-4547-a1d0-ec6e8b954ee6",
        ],
    ] {
        assert!(!cli.run(&args).status.success(), "{args:?}");
    }
}
