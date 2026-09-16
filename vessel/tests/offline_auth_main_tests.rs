//! Real entry-point regressions: no provider, supervisor, or parent environment changes.
use serde_json::Value;
use std::{
    fs,
    process::{Command, Output, Stdio},
};

struct PrivateDirectory(std::path::PathBuf);
impl PrivateDirectory {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Cli(PrivateDirectory);
impl Cli {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("vessel-offline-cli-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(PrivateDirectory(path))
    }
    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vessel"));
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
        command
            .env("OFFLINE_TEST_KEY", "fixture-not-a-real-key")
            .env("OFFLINE_ROTATED_KEY", "rotated-fixture-not-a-real-key");
        let output = command.args(args).output().unwrap();
        for stream in [&output.stdout, &output.stderr] {
            assert!(!String::from_utf8_lossy(stream).contains("fixture-not-a-real-key"));
        }
        output
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
fn local_oauth_status_logout_and_missing_import_are_offline() {
    let cli = Cli::new();
    let expected =
        serde_json::json!({"authenticated": false, "expires_at": null, "refreshable": false});
    assert_eq!(cli.json(&["auth", "status"]), expected);
    assert!(cli.ok(&["auth", "logout"]).contains("credentials removed"));
    assert_eq!(cli.json(&["auth", "status"]), expected);
    assert!(
        !cli.run(&["auth", "import-codex", "--path", "absent.json"])
            .status
            .success()
    );
    assert_eq!(cli.json(&["auth", "status"]), expected);
    assert!(cli.ok(&["--help"]).contains("auth"));
    assert!(cli.ok(&["--version"]).starts_with("vessel "));
}

#[test]
fn environment_account_lifecycle_preserves_identity_until_logout() {
    let cli = Cli::new();
    assert_eq!(
        cli.json(&["auth", "accounts", "connections"]),
        serde_json::json!([])
    );
    let connection = cli.json(&[
        "auth",
        "accounts",
        "connect",
        "--label",
        "offline",
        "--endpoint",
        "http://127.0.0.1:1/v1",
        "--transports",
        "openai-responses,openai-chat,anthropic",
    ]);
    assert_eq!(connection["revision"], 1);
    assert_eq!(connection["transports"].as_array().unwrap().len(), 3);
    let connection_id = connection["id"].as_str().unwrap();
    assert_eq!(
        cli.json(&["auth", "accounts", "connections"])[0]["id"],
        connection_id
    );
    let account = cli.json(&[
        "auth",
        "accounts",
        "add",
        "--connection",
        connection_id,
        "--account",
        "offline",
        "--env",
        "OFFLINE_TEST_KEY",
    ]);
    let id = account["id"].as_str().unwrap();
    assert_eq!(account["availability"], "available");
    assert_eq!(account["identity_generation"], 1);
    assert_eq!(
        cli.json(&["auth", "accounts", "status", "--account", id])[1][0]["id"],
        id
    );
    let rotated = cli.json(&[
        "auth",
        "accounts",
        "rotate-api",
        "--account",
        id,
        "--generation",
        "1",
        "--env",
        "OFFLINE_ROTATED_KEY",
        "--attest-same-identity",
    ]);
    assert_eq!(rotated["identity_generation"], 1);
    assert_eq!(rotated["credential_revision"], 2);
    cli.ok(&[
        "auth",
        "accounts",
        "rename",
        "--account",
        id,
        "--alias",
        "renamed",
        "--label",
        "Renamed offline account",
    ]);
    let renamed = cli.json(&["auth", "accounts", "list"]);
    assert_eq!(renamed[1][0]["alias"], "renamed");
    assert_eq!(renamed[1][0]["metadata_revision"], 2);
    assert!(
        cli.ok(&["auth", "accounts", "logout", "--account", id])
            .contains("upstream tokens were not revoked")
    );
    let revoked = cli.json(&["auth", "accounts", "status", "--account", id]);
    assert_eq!(revoked[1][0]["state"], "sign_in_required");
    assert_eq!(revoked[1][0]["identity_generation"], 2);
    cli.ok(&["auth", "accounts", "logout", "--account", id, "--remove"]);
    assert_eq!(
        cli.json(&["auth", "accounts", "status", "--account", id])[1][0]["state"],
        "removed"
    );
}

#[test]
fn invalid_account_operations_fail_before_any_provider_request() {
    let cli = Cli::new();
    let absent = "00000000-0000-4000-8000-000000000001";
    cli.fails(
        &[
            "auth",
            "accounts",
            "connect",
            "--label",
            "bad",
            "--endpoint",
            "http://127.0.0.1:1/v1",
            "--transports",
            "unsupported",
        ],
        "unsupported transport",
    );
    assert_eq!(
        cli.json(&["auth", "accounts", "connections"]),
        serde_json::json!([])
    );
    cli.fails(
        &[
            "auth",
            "accounts",
            "login",
            "--connection",
            absent,
            "--account",
            "offline",
        ],
        "private execution-host terminal",
    );
    cli.fails(&["auth", "accounts", "migrate-legacy"], "stop old binaries");
    for args in [
        vec![
            "auth",
            "accounts",
            "add",
            "--connection",
            absent,
            "--account",
            "offline",
            "--env",
            "UNDEFINED_FIXTURE_KEY",
        ],
        vec![
            "auth",
            "accounts",
            "import",
            "--connection",
            absent,
            "--account",
            "offline",
            "--path",
            "absent.json",
        ],
        vec![
            "auth",
            "accounts",
            "reauthenticate",
            "--account",
            absent,
            "--generation",
            "1",
            "--path",
            "absent.json",
        ],
        vec![
            "auth",
            "accounts",
            "rotate-api",
            "--account",
            absent,
            "--generation",
            "1",
            "--env",
            "OFFLINE_TEST_KEY",
        ],
        vec![
            "auth",
            "accounts",
            "enrollment-status",
            "--enrollment",
            absent,
        ],
        vec![
            "auth",
            "accounts",
            "cancel-enrollment",
            "--enrollment",
            absent,
        ],
    ] {
        assert!(!cli.run(&args).status.success(), "{args:?}");
    }
    assert_eq!(
        cli.json(&["auth", "accounts", "list"])[1],
        serde_json::json!([])
    );
}
