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
fn retired_inference_allowance_commands_are_unavailable_in_real_main() {
    let cli = Cli::new();
    let help = cli.ok(&["--help"]);
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("inference "))
    );
    for command in ["inspect", "configure", "audit", "history"] {
        cli.fails(
            &["inference", command],
            "unrecognized subcommand 'inference'",
        );
    }
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
fn connected_configuration_rejection_precedes_network_and_config_loading() {
    let cli = Cli::new();
    for options in [
        vec!["--config", "/missing/synthetic.toml"],
        vec!["--set", "model=synthetic"],
        vec!["--model", "synthetic"],
        vec!["--provider", "openai-chat"],
        vec!["--workspace", "/missing/workspace"],
        vec!["--access", "read-only"],
        vec!["--approval", "always"],
    ] {
        let mut args = options;
        args.extend(["connect"]);
        cli.fails(
            &args,
            "connected voyages resolve execution configuration on their host",
        );
    }
}

#[test]
fn explicitly_disabled_start_never_initializes_an_absent_vessel() {
    let cli = Cli::new();
    let absent = cli.0.path().join("absent-vessel");
    cli.fails(
        &[
            "connect",
            "--directory",
            absent.to_str().unwrap(),
            "--no-start",
            "list",
        ],
        "automatic startup is disabled",
    );
    assert!(
        !absent.exists(),
        "a read-only connection refusal must not start or initialize Vessel"
    );
}

#[test]
fn route_count_and_selection_fail_before_opening_scoped_credentials() {
    let cli = Cli::new();
    let missing = cli.0.path().join("never-opened-access.json");
    let path = missing.to_str().unwrap();
    let mut args = vec!["connect"];
    for _ in 0..17 {
        args.extend(["--access-file", path]);
    }
    args.push("list");
    cli.fails(&args, "at most 16 grant routes");
    cli.fails(
        &[
            "connect",
            "--access-file",
            path,
            "--access-file",
            path,
            "list",
        ],
        "single selected Vessel",
    );
    assert!(!missing.exists());
}

#[cfg(unix)]
#[test]
fn connected_private_directory_refuses_other_user_write_before_discovery() {
    use std::os::unix::fs::PermissionsExt;
    let cli = Cli::new();
    let directory = cli.0.path().join("unsafe-vessel");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
    let output = cli.run(&[
        "connect",
        "--directory",
        directory.to_str().unwrap(),
        "--no-start",
        "list",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("private"));
    assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
}

fn declarative_package(cli: &Cli, text: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let directory = cli.0.path().join("offline-package");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("guide.md"), text).unwrap();
    let manifest = serde_json::json!({"format":1,"id":"offline-guide","version":"1.0.0","helm":env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0,"capabilities":["model_context"],"contents":[{"path":"guide.md","kind":"skill","sha256":format!("{:x}",Sha256::digest(text.as_bytes()))}],"entrypoints":["guide.md"]});
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    directory
}

#[test]
fn extension_cli_pack_install_review_and_resource_never_execute_code() {
    let cli = Cli::new();
    let text = "Offline guidance\n\u{1b}[31mnot terminal authority\u{1b}[0m";
    let source = declarative_package(&cli, text);
    let archive = cli.0.path().join("guide.helmpkg");
    let digest = cli.ok(&[
        "extension",
        "pack",
        source.to_str().unwrap(),
        archive.to_str().unwrap(),
    ]);
    assert_eq!(digest.trim().len(), 64);
    let before = fs::read(&archive).unwrap();
    assert!(
        !cli.run(&[
            "extension",
            "pack",
            source.to_str().unwrap(),
            archive.to_str().unwrap()
        ])
        .status
        .success()
    );
    assert_eq!(fs::read(&archive).unwrap(), before);
    assert_eq!(
        cli.ok(&["extension", "install", archive.to_str().unwrap()])
            .trim(),
        digest.trim()
    );
    let installed = cli.json(&["extension", "inspect", "offline-guide"]);
    assert_eq!(installed["active"], false);
    assert_eq!(installed["execution_reviewed"], false);
    let output = cli.ok(&["extension", "resource", "offline-guide", "guide.md"]);
    assert!(!output.contains('\u{1b}'));
    assert_eq!(serde_json::from_str::<String>(&output).unwrap(), text);
    assert!(
        cli.json(&["extension", "execution-grants"])
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(!cli.0.path().join("runtime.sock").exists());
}

#[test]
fn extension_cli_enable_update_and_remove_are_exact_digest_fenced() {
    let cli = Cli::new();
    let directory = declarative_package(&cli, "first reviewed bytes");
    let source = directory.to_str().unwrap();
    let digest = cli.ok(&["extension", "install", source]);
    let digest = digest.trim();
    assert!(
        !cli.run(&[
            "extension",
            "enable",
            "offline-guide",
            "--expected",
            &"0".repeat(64)
        ])
        .status
        .success()
    );
    assert_eq!(
        cli.json(&["extension", "inspect", "offline-guide"])["active"],
        false
    );
    cli.ok(&["extension", "enable", "offline-guide", "--expected", digest]);
    assert_eq!(
        cli.json(&["extension", "inspect", "offline-guide"])["active"],
        true
    );
    let updated = declarative_package(&cli, "replacement bytes require another review");
    assert!(
        !cli.run(&[
            "extension",
            "update",
            "offline-guide",
            updated.to_str().unwrap(),
            "--expected",
            &"0".repeat(64)
        ])
        .status
        .success()
    );
    assert_eq!(
        cli.json(&["extension", "inspect", "offline-guide"])["digest"],
        digest
    );
    let replacement = cli.ok(&[
        "extension",
        "update",
        "offline-guide",
        updated.to_str().unwrap(),
        "--expected",
        digest,
    ]);
    let replacement = replacement.trim();
    assert_ne!(replacement, digest);
    assert_eq!(
        cli.json(&["extension", "inspect", "offline-guide"])["active"],
        false
    );
    assert!(
        !cli.run(&["extension", "remove", "offline-guide", "--expected", digest])
            .status
            .success()
    );
    cli.ok(&[
        "extension",
        "remove",
        "offline-guide",
        "--expected",
        replacement,
    ]);
    assert!(
        cli.json(&["extension", "list"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn declarative_cli_cannot_gain_executable_review_or_orphan_authority() {
    let cli = Cli::new();
    let source = declarative_package(&cli, "model context only");
    let digest = cli.ok(&["extension", "install", source.to_str().unwrap()]);
    for capability in ["execute", "host.file.read", "invented-shell"] {
        assert!(
            !cli.run(&[
                "extension",
                "review-executable",
                "offline-guide",
                "--expected",
                digest.trim(),
                "--capability",
                capability
            ])
            .status
            .success()
        );
    }
    let installed = cli.json(&["extension", "inspect", "offline-guide"]);
    assert_eq!(installed["active"], false);
    assert_eq!(installed["execution_reviewed"], false);
    assert!(
        cli.json(&["extension", "execution-grants"])
            .as_object()
            .unwrap()
            .is_empty()
    );
    let binding = installed["binding"].as_str().unwrap();
    assert!(
        !cli.run(&[
            "extension",
            "revoke-execution",
            binding,
            "--expected",
            digest.trim()
        ])
        .status
        .success()
    );
}

#[test]
fn extension_cli_activation_revocation_is_separate_from_package_retention() {
    let cli = Cli::new();
    let source = declarative_package(&cli, "reviewed context");
    let digest = cli.ok(&["extension", "install", source.to_str().unwrap()]);
    let digest = digest.trim();
    cli.ok(&["extension", "enable", "offline-guide", "--expected", digest]);
    let installed = cli.json(&["extension", "inspect", "offline-guide"]);
    let binding = installed["binding"].as_str().unwrap();
    let grants = cli.json(&["extension", "grants"]);
    assert_eq!(grants[binding], digest);
    assert!(
        !cli.run(&[
            "extension",
            "revoke-grant",
            binding,
            "--expected",
            &"0".repeat(64)
        ])
        .status
        .success()
    );
    assert_eq!(
        cli.json(&["extension", "inspect", "offline-guide"])["active"],
        true
    );
    cli.ok(&["extension", "revoke-grant", binding, "--expected", digest]);
    let retained = cli.json(&["extension", "inspect", "offline-guide"]);
    assert_eq!(retained["active"], false);
    assert_eq!(retained["digest"], digest);
    assert!(
        !cli.run(&["extension", "revoke-grant", binding, "--expected", digest])
            .status
            .success()
    );
}

#[test]
fn skill_cli_import_requires_the_exact_snapshot_and_stays_inactive() {
    let cli = Cli::new();
    let directory = cli.0.path().join("skill-guide");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("SKILL.md"),"---\nname: skill-guide\ndescription: Offline source fixture\n---\nReviewed instructions only.\n").unwrap();
    let snapshot = cli.json(&["extension", "inspect-skill", directory.to_str().unwrap()]);
    let digest = snapshot["digest"].as_str().unwrap();
    assert!(
        !cli.run(&[
            "extension",
            "import-skill",
            directory.to_str().unwrap(),
            "--expected",
            &"0".repeat(64)
        ])
        .status
        .success()
    );
    assert!(
        cli.json(&["extension", "list"])
            .as_array()
            .unwrap()
            .is_empty()
    );
    cli.ok(&[
        "extension",
        "import-skill",
        directory.to_str().unwrap(),
        "--expected",
        digest,
    ]);
    let installed = cli.json(&["extension", "inspect", "skill-guide"]);
    assert_eq!(installed["active"], false);
    assert_eq!(installed["execution_reviewed"], false);
    fs::write(
        directory.join("SKILL.md"),
        "---\nname: skill-guide\ndescription: Changed bytes\n---\nChanged instructions.\n",
    )
    .unwrap();
    assert!(
        !cli.run(&[
            "extension",
            "import-skill",
            directory.to_str().unwrap(),
            "--expected",
            digest
        ])
        .status
        .success()
    );
    assert_eq!(
        cli.json(&["extension", "inspect", "skill-guide"])["digest"],
        installed["digest"]
    );
}

#[test]
fn extension_fetch_rejects_untrusted_origin_before_network_or_install() {
    let cli = Cli::new();
    let index = cli.0.path().join("local-index.json");
    for origin in [
        "http://127.0.0.1:1/index",
        "file:///not-a-package",
        "https://user:private-canary@offline.invalid/index",
        "https://offline.invalid/index?secret=private-canary",
    ] {
        fs::write(
            &index,
            serde_json::to_vec(
                &serde_json::json!({"format":1,"url":origin,"sha256":"a".repeat(64)}),
            )
            .unwrap(),
        )
        .unwrap();
        let output = cli.run(&[
            "extension",
            "fetch",
            index.to_str().unwrap(),
            "offline-guide",
        ]);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-canary"));
        assert!(
            cli.json(&["extension", "list"])
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
