//! Ordinary-identity helper contracts in independent private fixtures. No live
//! provider, root peer, inherited credential store or human terminal is used.
use super::*;
use crate::{
    accounts::{ApiKeyInput, Registry},
    attachment::local_actor::storage::Directory,
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use voyage_protocol::{
    accounts::*, execution_profiles::ExecutionProfile, start_settings::StartSettings,
};

const SECRET: &str = "synthetic-private-api-credential-353";
const CHILD: &str = "identity_helper::coverage_tests::identity_observation_child";

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    registry: Registry,
    account: AccountBinding,
    other: AccountBinding,
    scope: IdentityAccountScope,
}
impl Fixture {
    fn new(endpoint: String) -> Self {
        let root = PathBuf::from(std::env::var_os("VOYAGE_IDENTITY_FIXTURE_ROOT").unwrap())
            .canonicalize()
            .unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let registry = Registry::default_host().unwrap();
        let first = registry
            .add_connection(
                "first connection".into(),
                endpoint,
                vec![Transport::OpenaiResponses],
            )
            .unwrap();
        let second = registry
            .add_connection(
                "second connection".into(),
                "http://127.0.0.1:1/v1".into(),
                vec![Transport::Anthropic],
            )
            .unwrap();
        let first = registry
            .add_api(
                first.id,
                "first".into(),
                "First private account".into(),
                ApiKeyInput::Stored(SECRET.into()),
            )
            .unwrap();
        let second = registry
            .add_api(
                second.id,
                "second".into(),
                "Second private account".into(),
                ApiKeyInput::Stored("synthetic-other-private-credential-353".into()),
            )
            .unwrap();
        let account = registry
            .freeze(first.id, Transport::OpenaiResponses)
            .unwrap();
        let other = registry.freeze(second.id, Transport::Anthropic).unwrap();
        let scope = IdentityAccountScope {
            full_access: true,
            can_use: true,
            can_enroll: true,
            account_ids: vec![],
            enrollment_connections: vec![],
            actor: EnrollmentActor {
                principal: "owner".into(),
                workspace: workspace.to_string_lossy().into_owned(),
            },
        };
        Self {
            root,
            workspace,
            registry,
            account,
            other,
            scope,
        }
    }
    fn stage(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        Directory::open(&path).unwrap();
        path
    }
    fn scoped(&self) -> IdentityAccountScope {
        IdentityAccountScope {
            full_access: false,
            account_ids: vec![self.account.account_id],
            ..self.scope.clone()
        }
    }
    fn profile(&self) -> ExecutionProfile {
        ExecutionProfile {
            id: Uuid::new_v4(),
            name: "Private host preference".into(),
            account: self.account.clone(),
            model: "synthetic-profile-model".into(),
            reasoning_effort: Some("high".into()),
            service_tier: None,
        }
    }
    fn capture(&self, stage: &Path) -> serde_json::Value {
        capture_launch(
            &self.scope,
            &self.workspace,
            stage,
            &"a".repeat(64),
            None,
            Some(self.account.clone()),
            &StartSettings::default(),
        )
        .unwrap()
    }
}

async fn inventory(f: &Fixture) {
    f.registry
        .set_default_account(Uuid::new_v4(), &f.workspace, 0, f.other.clone())
        .unwrap();
    let scoped = account_operation(
        &f.workspace,
        IdentityHelperOperation::Accounts {
            scope: f.scoped(),
            transport: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(scoped["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(
        scoped["accounts"][0]["id"],
        f.account.account_id.to_string()
    );
    assert_eq!(scoped["connections"].as_array().unwrap().len(), 1);
    assert!(scoped["default_account"].is_null());
    assert_eq!(scoped["can_set_default"], false);
    assert!(!scoped.to_string().contains(SECRET));
    assert!(!scoped.to_string().contains("Second private account"));
    let filtered = account_operation(
        &f.workspace,
        IdentityHelperOperation::Accounts {
            scope: f.scoped(),
            transport: Some(Transport::Anthropic),
        },
    )
    .await
    .unwrap();
    assert_eq!(filtered["accounts"], serde_json::json!([]));
    assert_eq!(filtered["connections"], serde_json::json!([]));
    let connection = f.registry.ensure_chatgpt_connection().unwrap();
    let enroll_only = IdentityAccountScope {
        full_access: false,
        can_use: false,
        account_ids: vec![],
        enrollment_connections: vec![connection.id],
        ..f.scope.clone()
    };
    let metadata = account_operation(
        &f.workspace,
        IdentityHelperOperation::Accounts {
            scope: enroll_only,
            transport: Some(Transport::ChatgptOauth),
        },
    )
    .await
    .unwrap();
    assert_eq!(metadata["accounts"], serde_json::json!([]));
    assert_eq!(metadata["connections"].as_array().unwrap().len(), 1);
    assert_eq!(metadata["connections"][0]["id"], connection.id.to_string());
    let before = std::fs::read(f.root.join("data/helm/accounts/registry.json")).unwrap();
    let denied = IdentityAccountScope {
        can_use: false,
        can_enroll: false,
        ..f.scope.clone()
    };
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::Accounts {
                scope: denied,
                transport: None
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(f.root.join("data/helm/accounts/registry.json")).unwrap(),
        before
    );
}

async fn bindings(f: &Fixture) {
    let observed = account_operation(
        &f.workspace,
        IdentityHelperOperation::ValidateAccount {
            scope: f.scoped(),
            account: f.account.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        observed["account"],
        serde_json::to_value(&f.account).unwrap()
    );
    assert_eq!(
        observed["capability_revision"],
        f.registry
            .validate_binding(&f.account)
            .unwrap()
            .capability_revision
    );
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::ValidateAccount {
                scope: f.scoped(),
                account: f.other.clone()
            }
        )
        .await
        .is_err()
    );
    let mut stale = f.account.clone();
    stale.identity_generation += 1;
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::ValidateAccount {
                scope: f.scope.clone(),
                account: stale
            }
        )
        .await
        .is_err()
    );
    f.registry.logout(f.account.account_id, false).unwrap();
    // Exact retained intent observation is metadata, not validation or execution.
    let intent = account_operation(
        &f.workspace,
        IdentityHelperOperation::ObserveAccountIntent {
            scope: f.scoped(),
            account: f.account.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(intent, serde_json::json!({"permitted":true}));
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::ValidateAccount {
                scope: f.scope.clone(),
                account: f.account.clone()
            }
        )
        .await
        .is_err()
    );
}

async fn authority_absent(f: &Fixture) {
    let before = std::fs::read(f.root.join("data/helm/accounts/registry.json")).unwrap();
    let request = EnrollmentRequest {
        command_id: Uuid::new_v4(),
        enrollment_id: Uuid::new_v4(),
        connection_id: f.account.connection_id,
        alias: "never-published".into(),
        label: "No authority".into(),
        actor: f.scope.actor.clone(),
    };
    let enrollment = [
        IdentityEnrollmentOperation::Start {
            request: request.clone(),
        },
        IdentityEnrollmentOperation::Resolve {
            request: request.clone(),
        },
        IdentityEnrollmentOperation::Drive {
            enrollment_id: request.enrollment_id,
        },
        IdentityEnrollmentOperation::Status {
            enrollment_id: request.enrollment_id,
        },
        IdentityEnrollmentOperation::Cancel {
            command_id: Uuid::new_v4(),
            enrollment_id: request.enrollment_id,
        },
    ];
    for operation in enrollment {
        assert!(
            account_operation(
                &f.workspace,
                IdentityHelperOperation::Enrollment {
                    scope: f.scope.clone(),
                    operation
                }
            )
            .await
            .is_err()
        );
    }
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::Usage {
                scope: f.scope.clone(),
                account: f.account.clone(),
                refresh: true
            }
        )
        .await
        .is_err()
    );
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::SetDefault {
                scope: f.scope.clone(),
                command_id: Uuid::new_v4(),
                account: f.account.clone(),
                expected_revision: 0
            }
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(f.root.join("data/helm/accounts/registry.json")).unwrap(),
        before
    );
    assert_eq!(f.registry.default_account().unwrap(), (0, None));
}

async fn enrollment_provenance(f: &Fixture) {
    let connection = f.registry.ensure_chatgpt_connection().unwrap();
    let account = f
        .registry
        .add_oauth(
            connection.id,
            "enrolled".into(),
            "Synthetic enrolled account".into(),
            crate::accounts::OAuthTokens {
                access_token: "synthetic-oauth-access-353".into(),
                refresh_token: "synthetic-oauth-refresh-353".into(),
                id_token: None,
                expires_at: u64::MAX,
                account_id: "synthetic-login-353".into(),
            },
        )
        .unwrap();
    let binding = f
        .registry
        .freeze(account.id, Transport::ChatgptOauth)
        .unwrap();
    // This is controlled retained publication metadata, not an upstream OAuth
    // success claim. It exercises the helper's provenance/actor filter only.
    let directory = Directory::open_existing(&f.root.join("data/helm/accounts")).unwrap();
    let mut database: serde_json::Value =
        serde_json::from_slice(&directory.read("registry.json").unwrap().unwrap()).unwrap();
    let enrollment_id = Uuid::new_v4();
    database["enrollments"].as_array_mut().unwrap().push(serde_json::json!({
        "request":{"command_id":Uuid::new_v4(),"enrollment_id":enrollment_id,"connection_id":connection.id,"alias":"enrolled","label":"Synthetic enrolled account","actor":f.scope.actor},
        "status":{"enrollment_id":enrollment_id,"state":"succeeded","account_id":account.id,"expires_at":1,"effects_may_have_occurred":true},
        "device":null,"next_poll":0,"cancellations":[],"polling":false,"failure":null
    }));
    directory
        .publish("registry.json", &serde_json::to_vec(&database).unwrap())
        .unwrap();
    let scope = IdentityAccountScope {
        full_access: false,
        enrollment_connections: vec![connection.id],
        ..f.scope.clone()
    };
    assert!(allowed(&scope, &f.registry, &binding, &f.workspace));
    let inventory = account_operation(
        &f.workspace,
        IdentityHelperOperation::Accounts {
            scope: scope.clone(),
            transport: Some(Transport::ChatgptOauth),
        },
    )
    .await
    .unwrap();
    assert_eq!(inventory["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(inventory["accounts"][0]["id"], account.id.to_string());
    assert!(!inventory.to_string().contains("synthetic-oauth"));
    assert!(!inventory.to_string().contains("synthetic-login"));
    for field in 0..4 {
        let mut changed = scope.clone();
        match field {
            0 => changed.actor.principal = "different-principal".into(),
            1 => changed.enrollment_connections.clear(),
            2 => changed.can_enroll = false,
            _ => changed.can_use = false,
        };
        assert!(!allowed(&changed, &f.registry, &binding, &f.workspace));
    }
    assert!(!allowed(&scope, &f.registry, &binding, &f.root));
    database["enrollments"][0]["status"]["state"] = "cancelled".into();
    directory
        .publish("registry.json", &serde_json::to_vec(&database).unwrap())
        .unwrap();
    assert!(!allowed(&scope, &f.registry, &binding, &f.workspace));
}

async fn defaults(f: &Fixture) {
    for (profiles_revision, code) in [
        (0, "default_account_required"),
        (1, "default_profile_required"),
    ] {
        let value = account_operation(
            &f.workspace,
            IdentityHelperOperation::Defaults {
                scope: f.scope.clone(),
                profile: None,
                profiles_revision,
            },
        )
        .await
        .unwrap();
        assert_eq!(value["code"], code);
        assert!(value["account"].is_null());
    }
    f.registry
        .set_default_account(Uuid::new_v4(), &f.workspace, 0, f.account.clone())
        .unwrap();
    let value = account_operation(
        &f.workspace,
        IdentityHelperOperation::Defaults {
            scope: f.scoped(),
            profile: None,
            profiles_revision: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(value["account"], serde_json::to_value(&f.account).unwrap());
    let profile = f.profile();
    let value = account_operation(
        &f.workspace,
        IdentityHelperOperation::Defaults {
            scope: f.scoped(),
            profile: Some(profile.clone()),
            profiles_revision: 4,
        },
    )
    .await
    .unwrap();
    assert_eq!(value["model"], profile.model);
    assert_eq!(value["reasoning_effort"], "high");
    assert!(!value.to_string().contains(SECRET));
    assert!(value.get("api_key").is_none());
    let mut denied = f.scoped();
    denied.can_use = false;
    assert!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::Defaults {
                scope: denied,
                profile: Some(profile),
                profiles_revision: 4
            }
        )
        .await
        .is_err()
    );
}

async fn profiles(f: &Fixture) {
    let mut profile = f.profile();
    assert_eq!(
        account_operation(
            &f.workspace,
            IdentityHelperOperation::ValidateProfile {
                scope: f.scoped(),
                profile: profile.clone()
            }
        )
        .await
        .unwrap()["valid"],
        true
    );
    profile.name = format!("private {SECRET}");
    assert!(validate_profile(&f.scoped(), &f.workspace, &profile).is_err());
    profile = f.profile();
    profile.model = SECRET.into();
    assert!(validate_profile(&f.scope, &f.workspace, &profile).is_err());
    profile = f.profile();
    profile.reasoning_effort = Some("unknown-effort".into());
    assert!(validate_profile(&f.scope, &f.workspace, &profile).is_err());
    profile = f.profile();
    profile.account = f.other.clone();
    assert!(validate_profile(&f.scoped(), &f.workspace, &profile).is_err());
}

fn frozen(f: &Fixture) {
    let stage = f.stage("frozen");
    let receipt = f.capture(&stage);
    let bytes = frozen_config(&stage.join("launch.json")).unwrap();
    assert_eq!(receipt["config_digest"], config_digest(&bytes));
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    assert!(receipt.get("config").is_none());
    f.registry
        .set_default_account(Uuid::new_v4(), &f.workspace, 0, f.other.clone())
        .unwrap();
    assert_eq!(f.capture(&stage), receipt);
    let settings = StartSettings {
        model: Some("changed-model".into()),
        ..Default::default()
    };
    assert!(
        capture_launch(
            &f.scope,
            &f.workspace,
            &stage,
            &"a".repeat(64),
            None,
            Some(f.account.clone()),
            &settings
        )
        .is_err()
    );
    assert!(
        capture_launch(
            &f.scope,
            &f.workspace,
            &stage,
            &"b".repeat(64),
            None,
            Some(f.account.clone()),
            &Default::default()
        )
        .is_err()
    );
    let denied = IdentityAccountScope {
        can_use: false,
        ..f.scope.clone()
    };
    assert!(
        capture_launch(
            &denied,
            &f.workspace,
            &stage,
            &"a".repeat(64),
            None,
            None,
            &Default::default()
        )
        .is_err()
    );
    assert_eq!(frozen_config(&stage.join("launch.json")).unwrap(), bytes);
}

fn uncertain(f: &Fixture) {
    let stage = f.stage("uncertain");
    let storage = Directory::open_existing(&stage).unwrap();
    storage
        .publish_new("request.json", "a".repeat(64).as_bytes())
        .unwrap();
    assert!(
        capture_launch(
            &f.scope,
            &f.workspace,
            &stage,
            &"a".repeat(64),
            None,
            Some(f.account.clone()),
            &Default::default()
        )
        .is_err()
    );
    assert!(!stage.join("launch.json").exists());
    assert_eq!(
        storage.read_bounded("request.json", 256).unwrap().unwrap(),
        "a".repeat(64).as_bytes()
    );
    let complete = f.stage("complete");
    f.capture(&complete);
    let unmatched = f.stage("unmatched");
    Directory::open_existing(&unmatched)
        .unwrap()
        .publish_new(
            "launch.json",
            &frozen_config(&complete.join("launch.json")).unwrap(),
        )
        .unwrap();
    assert!(
        capture_launch(
            &f.scope,
            &f.workspace,
            &unmatched,
            &"a".repeat(64),
            None,
            None,
            &Default::default()
        )
        .is_err()
    );
    assert!(!unmatched.join("request.json").exists());
    for (i, request) in ["short".to_string(), "z".repeat(64)]
        .into_iter()
        .enumerate()
    {
        let invalid = f.stage(&format!("invalid-{i}"));
        assert!(
            capture_launch(
                &f.scope,
                &f.workspace,
                &invalid,
                &request,
                None,
                None,
                &Default::default()
            )
            .is_err()
        );
        assert!(!invalid.join("request.json").exists());
    }
    let missing = f.root.join("never-created");
    assert!(
        capture_launch(
            &f.scope,
            &f.workspace,
            &missing,
            &"a".repeat(64),
            None,
            None,
            &Default::default()
        )
        .is_err()
    );
    assert!(!missing.exists());
}

fn reviews(f: &Fixture) {
    let stage = f.stage("review");
    f.capture(&stage);
    let path = stage.join("launch.json");
    let facts = review_config(&f.workspace, &path).unwrap();
    assert_eq!(facts.account, f.account);
    assert_eq!(facts.uid, unsafe { libc::geteuid() });
    assert_eq!(facts.gid, unsafe { libc::getegid() });
    assert_eq!(
        facts.config_digest,
        config_digest(&frozen_config(&path).unwrap())
    );
    assert_eq!(
        facts.capability_revision,
        f.registry
            .validate_binding(&f.account)
            .unwrap()
            .capability_revision
    );
    assert_eq!(facts.policy_digest.len(), 64);
    assert_eq!(facts.account_root_digest.len(), 64);
    assert!(!serde_json::to_string(&facts).unwrap().contains(SECRET));
    let other = f.stage("other-workspace");
    assert!(review_config(&other, &path).is_err());
    let anonymous = f.stage("anonymous");
    let launch =
        crate::launch_config::LaunchConfig::capture(&crate::Config::default(), &f.workspace)
            .unwrap();
    Directory::open_existing(&anonymous)
        .unwrap()
        .publish_new("launch.json", &serde_json::to_vec(&launch).unwrap())
        .unwrap();
    assert!(review_config(&f.workspace, &anonymous.join("launch.json")).is_err());
    f.registry.logout(f.account.account_id, false).unwrap();
    assert!(review_config(&f.workspace, &path).is_err());
}

fn base_config(f: &Fixture) {
    let original = f.stage("original");
    f.capture(&original);
    let stage = f.stage("derived");
    let settings = StartSettings {
        model: Some("derived-model".into()),
        ..Default::default()
    };
    let receipt = capture_launch(
        &f.scope,
        &f.workspace,
        &stage,
        &"c".repeat(64),
        Some(&original.join("launch.json")),
        None,
        &settings,
    )
    .unwrap();
    let bytes = frozen_config(&stage.join("launch.json")).unwrap();
    assert_eq!(receipt["config_digest"], config_digest(&bytes));
    let launch: crate::launch_config::LaunchConfig = serde_json::from_slice(&bytes).unwrap();
    let config = launch.resolve(&f.workspace).unwrap();
    assert_eq!(config.model, "derived-model");
    assert_eq!(config.account, Some(f.account.clone()));
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    let wrong_workspace = f.stage("wrong-workspace");
    let refused = f.stage("refused");
    assert!(
        capture_launch(
            &f.scope,
            &wrong_workspace,
            &refused,
            &"d".repeat(64),
            Some(&original.join("launch.json")),
            None,
            &Default::default()
        )
        .is_err()
    );
    assert!(!refused.join("request.json").exists());
}

fn initialization_capture(f: &Fixture) {
    use voyage_protocol::process::{ParticipantPolicy, RuntimeInitialization};
    f.registry
        .set_default_account(Uuid::new_v4(), &f.workspace, 0, f.account.clone())
        .unwrap();
    let stage = f.stage("participant");
    let initialize = RuntimeInitialization::Participant {
        assignment_id: Uuid::new_v4(),
        parent_vessel_id: Uuid::new_v4(),
        parent_session_id: Uuid::new_v4(),
        parent_run_id: Uuid::new_v4(),
        policy: ParticipantPolicy {
            access: "read-only".into(),
            legacy_deny_commands: vec![],
            inherit_env: vec![],
            github_enabled: false,
            timeout_secs: 1,
            max_output_bytes: 1024,
            max_subagents: 1,
        },
    };
    let first = initialization::capture(
        &f.scoped(),
        &f.workspace,
        &stage,
        &"e".repeat(64),
        None,
        &initialize,
    )
    .unwrap();
    assert!(stage.join("initialization-intent.json").is_file());
    assert!(stage.join("request.json").is_file());
    let before = frozen_config(&stage.join("launch.json")).unwrap();
    f.registry
        .set_default_account(Uuid::new_v4(), &f.workspace, 1, f.other.clone())
        .unwrap();
    assert_eq!(
        initialization::capture(
            &f.scoped(),
            &f.workspace,
            &stage,
            &"e".repeat(64),
            None,
            &initialize
        )
        .unwrap(),
        first
    );
    let mut changed = initialize;
    if let RuntimeInitialization::Participant { parent_run_id, .. } = &mut changed {
        *parent_run_id = Uuid::new_v4();
    }
    assert!(
        initialization::capture(
            &f.scoped(),
            &f.workspace,
            &stage,
            &"e".repeat(64),
            None,
            &changed
        )
        .is_err()
    );
    assert_eq!(frozen_config(&stage.join("launch.json")).unwrap(), before);
    assert!(first.get("initialize").is_none());
    assert!(!first.to_string().contains(SECRET));
}

async fn models(f: &Fixture, listener: tokio::net::TcpListener, rotate: bool) {
    let workspace = f.workspace.clone();
    let scope = f.scoped();
    let account = f.account.clone();
    let operation = tokio::spawn(async move {
        account_operation(
            &workspace,
            IdentityHelperOperation::Models { scope, account },
        )
        .await
    });
    let (mut stream, peer) = listener.accept().await.unwrap();
    assert!(peer.ip().is_loopback());
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).await.unwrap(), 1);
        request.push(byte[0]);
        assert!(request.len() <= 16_384);
    }
    let request = String::from_utf8(request).unwrap();
    assert!(request.starts_with("GET /v1/models "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {SECRET}"))
    );
    if rotate {
        f.registry
            .rotate_api(
                &f.account,
                ApiKeyInput::Stored("synthetic-rotated-private-key-353".into()),
                true,
            )
            .unwrap();
    }
    let body = r#"{"data":[{"id":"synthetic-discovered-model"}]}"#;
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
    let result = operation.await.unwrap();
    if rotate {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("model account context changed")
        );
    } else {
        let result = result.unwrap();
        assert_eq!(result["account"], serde_json::to_value(&f.account).unwrap());
        assert!(
            result["models"]
                .as_array()
                .unwrap()
                .iter()
                .any(|model| model["id"] == "synthetic-discovered-model")
        );
        assert!(!result.to_string().contains(SECRET));
    }
}

#[test]
fn identity_observation_child() {
    let Ok(mode) = std::env::var("VOYAGE_IDENTITY_FIXTURE_MODE") else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = if mode.starts_with("models_") {
            Some(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap())
        } else {
            None
        };
        let endpoint = listener
            .as_ref()
            .map(|l| format!("http://{}/v1", l.local_addr().unwrap()))
            .unwrap_or_else(|| "http://127.0.0.1:1/v1".into());
        let fixture = Fixture::new(endpoint);
        tokio::time::timeout(Duration::from_secs(20), async {
            match mode.as_str() {
                "inventory" => inventory(&fixture).await,
                "bindings" => bindings(&fixture).await,
                "authority_absent" => authority_absent(&fixture).await,
                "enrollment_provenance" => enrollment_provenance(&fixture).await,
                "defaults" => defaults(&fixture).await,
                "profiles" => profiles(&fixture).await,
                "frozen" => frozen(&fixture),
                "uncertain" => uncertain(&fixture),
                "reviews" => reviews(&fixture),
                "base_config" => base_config(&fixture),
                "initialization_capture" => initialization_capture(&fixture),
                "models_clean" | "models_rotate" => {
                    models(&fixture, listener.unwrap(), mode == "models_rotate").await
                }
                _ => panic!("unknown isolated helper fixture"),
            }
        })
        .await
        .unwrap();
        std::fs::write(fixture.root.join("done"), &mode).unwrap();
    });
}

fn child(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args(["--exact", CHILD, "--nocapture"])
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("VOYAGE_IDENTITY_FIXTURE_ROOT", root.path())
        .env("VOYAGE_IDENTITY_FIXTURE_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", value);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated helper fixture timed out: {mode}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "isolated helper contract failed: {mode}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("done")).unwrap(),
        mode
    );
}

#[test]
fn account_inventory_applies_scope_transport_and_default_visibility() {
    child("inventory");
}
#[test]
fn account_validation_is_distinct_from_retained_intent_observation() {
    child("bindings");
}

#[test]
fn absent_root_authority_prevents_enrollment_usage_and_host_default_effects() {
    child("authority_absent");
}

#[test]
fn enrolled_account_visibility_requires_exact_published_actor_and_connection() {
    child("enrollment_provenance");
}
#[test]
fn defaults_require_explicit_current_account_and_profile_context() {
    child("defaults");
}
#[test]
fn profiles_reject_denied_accounts_secrets_and_unsupported_preferences() {
    child("profiles");
}
#[test]
fn frozen_launch_retry_retains_account_and_rejects_changed_envelope() {
    child("frozen");
}
#[test]
fn incomplete_frozen_publication_never_recaptures_or_creates_missing_stage() {
    child("uncertain");
}
#[test]
fn private_review_returns_only_current_identity_account_and_policy_facts() {
    child("reviews");
}

#[test]
fn derived_launch_preserves_exact_private_base_and_refuses_another_workspace() {
    child("base_config");
}

#[test]
fn initialization_retry_retains_original_default_and_exact_parent_provenance() {
    child("initialization_capture");
}
#[test]
fn model_observation_uses_only_the_selected_local_fixture_endpoint() {
    child("models_clean");
}
#[test]
fn model_observation_rejects_capability_change_during_discovery() {
    child("models_rotate");
}
