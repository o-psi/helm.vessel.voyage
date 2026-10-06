//! Real framed helper entry in an env-cleared ordinary child. No root authority,
//! provider request, executor, listener, inherited account or human data.
use super::*;
use crate::accounts::{ApiKeyInput, Registry};
use crate::attachment::local_actor::storage::Directory;
use crate::tools::process::owned_lifetime_tests::owned;
use std::{
    fs,
    io::Write,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};
use uuid::Uuid;
use voyage_protocol::accounts::{AccountBinding, EnrollmentActor, Transport};

macro_rules! case {
    ($name:ident,$body:block) => { #[test] fn $name() {
        owned::run(concat!(module_path!(),"::",stringify!($name)), || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async $body)
        });
    }};
}

pub(crate) fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap())
        .canonicalize()
        .unwrap()
}
pub(crate) fn frame(value: &serde_json::Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(value).unwrap();
    let mut frame = u32::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
    frame.extend(bytes);
    frame
}
struct Stdio {
    originals: [OwnedFd; 2],
}
impl Stdio {
    fn redirect(input: &fs::File, output: &fs::File) -> Self {
        std::io::stdout().flush().unwrap();
        let originals = [0, 1].map(|fd| {
            let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 64) };
            assert!(duplicate >= 64);
            unsafe { OwnedFd::from_raw_fd(duplicate) }
        });
        assert_eq!(unsafe { libc::dup2(input.as_raw_fd(), 0) }, 0);
        assert_eq!(unsafe { libc::dup2(output.as_raw_fd(), 1) }, 1);
        Self { originals }
    }
}
impl Drop for Stdio {
    fn drop(&mut self) {
        for (fd, original) in self.originals.iter().enumerate() {
            assert_eq!(
                unsafe { libc::dup2(original.as_raw_fd(), fd as i32) },
                fd as i32
            );
        }
    }
}
/// The libtest runner keeps its own private log; only the production helper's
/// one invocation uses these exact private request/response descriptors. Stdio
/// changes are confined to an exact single-test child and always restored.
pub(crate) async fn invoke<T>(
    bytes: &[u8],
    action: impl std::future::Future<Output = T>,
) -> (T, Vec<u8>) {
    let request = root().join("helper-request.bin");
    let response = root().join("helper-response.bin");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&request)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let input = fs::File::open(&request).unwrap();
    let output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&response)
        .unwrap();
    let redirect = Stdio::redirect(&input, &output);
    let result = action.await;
    drop(redirect);
    output.sync_all().unwrap();
    assert!(fs::metadata(&response).unwrap().len() <= IDENTITY_HELPER_BYTES as u64 + 4);
    (result, fs::read(response).unwrap())
}
async fn dispatch(bytes: &[u8]) -> serde_json::Value {
    let (result, output) = invoke(bytes, super::run()).await;
    result.unwrap();
    assert!(output.len() >= 4);
    let length = u32::from_be_bytes(output[..4].try_into().unwrap()) as usize;
    assert_eq!(output.len(), 4 + length);
    let response: IdentityHelperResponse = serde_json::from_slice(&output[4..]).unwrap();
    serde_json::to_value(response).unwrap()
}
fn unavailable(response: serde_json::Value) {
    assert_eq!(response, serde_json::json!({"result":"unavailable"}));
}
struct Fixture {
    workspace: PathBuf,
    registry: Registry,
    account: AccountBinding,
    scope: IdentityAccountScope,
}
impl Fixture {
    fn new() -> Self {
        let workspace = root().join("workspace");
        Directory::open(&workspace).unwrap();
        let registry = Registry::default_host().unwrap();
        let connection = registry
            .add_connection(
                "offline helper fixture".into(),
                "http://127.0.0.1:1/v1".into(),
                vec![Transport::OpenaiResponses],
            )
            .unwrap();
        let account = registry
            .add_api(
                connection.id,
                "fixture".into(),
                "Owned private fixture".into(),
                ApiKeyInput::Stored("synthetic-only-helper-key-353".into()),
            )
            .unwrap();
        let account = registry
            .freeze(account.id, Transport::OpenaiResponses)
            .unwrap();
        let scope = IdentityAccountScope {
            full_access: false,
            can_use: true,
            can_enroll: false,
            account_ids: vec![account.account_id],
            enrollment_connections: vec![],
            actor: EnrollmentActor {
                principal: "ordinary fixture".into(),
                workspace: workspace.to_string_lossy().into_owned(),
            },
        };
        Self {
            workspace,
            registry,
            account,
            scope,
        }
    }
    fn request(&self, operation: IdentityHelperOperation) -> Vec<u8> {
        frame(
            &serde_json::to_value(IdentityHelperRequest {
                schema: IDENTITY_HELPER_SCHEMA,
                workspace: self.workspace.clone(),
                operation,
            })
            .unwrap(),
        )
    }
    fn capture(&self, name: &str) -> (PathBuf, serde_json::Value) {
        let directory = root().join(name);
        Directory::open(&directory).unwrap();
        let receipt = capture_launch(
            &self.scope,
            &self.workspace,
            &directory,
            &"a".repeat(64),
            None,
            Some(self.account.clone()),
            &Default::default(),
        )
        .unwrap();
        (directory, receipt)
    }
}
fn scope(path: &Path) -> IdentityAccountScope {
    IdentityAccountScope {
        full_access: false,
        can_use: false,
        can_enroll: false,
        account_ids: vec![],
        enrollment_connections: vec![],
        actor: EnrollmentActor {
            principal: "fixture".into(),
            workspace: path.to_string_lossy().into_owned(),
        },
    }
}
fn observation(workspace: PathBuf, schema: u32) -> Vec<u8> {
    frame(
        &serde_json::to_value(IdentityHelperRequest {
            schema,
            operation: IdentityHelperOperation::Accounts {
                scope: scope(&workspace),
                transport: None,
            },
            workspace,
        })
        .unwrap(),
    )
}
fn recovery(workspace: &Path) -> voyage_protocol::identity_helper::BoundRecoveryRequest {
    voyage_protocol::identity_helper::BoundRecoveryRequest {
        directory: root().join("never-created-recovery"),
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        local_process_retired: true,
        current_scope_cleanup_observed: false,
        actor: EnrollmentActor {
            principal: "ordinary fixture".into(),
            workspace: workspace.to_string_lossy().into_owned(),
        },
        acknowledge_cleanup: None,
        acknowledge_resources: vec![],
        reconcile_tools: None,
        expected_revision: None,
    }
}

case!(zero_frame_is_a_single_collapsed_unavailable_response, {
    unavailable(dispatch(&[0, 0, 0, 0]).await);
});
case!(truncated_header_cannot_disclose_parser_or_io_diagnostics, {
    unavailable(dispatch(&[0, 1]).await);
});
case!(truncated_payload_cannot_publish_partial_results, {
    unavailable(dispatch(&[0, 0, 0, 20, b'{']).await);
});
case!(malformed_json_and_private_canary_collapse_without_echo, {
    let bytes = b"synthetic-private-parse-canary";
    let mut wire = (bytes.len() as u32).to_be_bytes().to_vec();
    wire.extend(bytes);
    unavailable(dispatch(&wire).await);
});
case!(unknown_wire_fields_fail_before_account_storage_effects, {
    let workspace = root();
    let mut value: serde_json::Value =
        serde_json::from_slice(&observation(workspace, 1)[4..]).unwrap();
    value["untrusted_extra"] = serde_json::json!("synthetic-private-field");
    unavailable(dispatch(&frame(&value)).await);
    assert!(!root().join("data/helm/accounts").exists());
});
case!(unsupported_schema_cannot_create_account_registry, {
    unavailable(dispatch(&observation(root(), IDENTITY_HELPER_SCHEMA + 1)).await);
    assert!(!root().join("data/helm/accounts").exists());
});
case!(relative_workspace_is_refused_without_creating_a_project, {
    unavailable(dispatch(&observation(PathBuf::from("uncreated-project"), 1)).await);
    assert!(!root().join("uncreated-project").exists());
});
case!(missing_workspace_is_refused_without_recreating_it, {
    let absent = root().join("missing-workspace");
    unavailable(dispatch(&observation(absent.clone(), 1)).await);
    assert!(!absent.exists());
});
case!(aliased_workspace_cannot_become_a_canonical_account_realm, {
    let alias = root().join("alias");
    std::os::unix::fs::symlink(root(), &alias).unwrap();
    unavailable(dispatch(&observation(alias, 1)).await);
    assert!(!root().join("data/helm/accounts").exists());
});
case!(
    semantic_request_bound_precedes_filesystem_account_observation,
    {
        let workspace = PathBuf::from(format!("/{}", "x".repeat(IDENTITY_HELPER_BYTES)));
        unavailable(dispatch(&observation(workspace, 1)).await);
        assert!(!root().join("data/helm/accounts").exists());
    }
);
case!(
    ordinary_framed_account_validation_returns_only_exact_binding_revision,
    {
        let f = Fixture::new();
        let revision = f
            .registry
            .validate_binding(&f.account)
            .unwrap()
            .capability_revision;
        let response = dispatch(&f.request(IdentityHelperOperation::ValidateAccount {
            scope: f.scope.clone(),
            account: f.account.clone(),
        }))
        .await;
        assert_eq!(
            response,
            serde_json::json!({"result":"value","value":{"account":f.account,"capability_revision":revision}})
        );
        assert!(
            !response
                .to_string()
                .contains("synthetic-only-helper-key-353")
        );
    }
);
case!(denied_account_intent_cannot_mutate_the_fixture_registry, {
    let f = Fixture::new();
    let path = root().join("data/helm/accounts/registry.json");
    let before = fs::read(&path).unwrap();
    unavailable(
        dispatch(&f.request(IdentityHelperOperation::ObserveAccountIntent {
            scope: scope(&f.workspace),
            account: f.account.clone(),
        }))
        .await,
    );
    assert_eq!(fs::read(path).unwrap(), before);
});
case!(
    framed_launch_capture_publishes_exact_private_digest_without_credentials,
    {
        let f = Fixture::new();
        let directory = root().join("fresh-capture");
        Directory::open(&directory).unwrap();
        let response = dispatch(&f.request(IdentityHelperOperation::CaptureLaunch {
            scope: f.scope.clone(),
            directory: directory.clone(),
            request_digest: "b".repeat(64),
            base_config_path: None,
            account: Some(f.account.clone()),
            settings: Default::default(),
        }))
        .await;
        let bytes = frozen_config(&directory.join("launch.json")).unwrap();
        assert_eq!(
            response,
            serde_json::json!({"result":"value","value":{"config_path":directory.join("launch.json"),"config_digest":config_digest(&bytes)}})
        );
        assert_eq!(
            fs::read(directory.join("request.json")).unwrap(),
            "b".repeat(64).as_bytes()
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-only-helper-key-353"));
    }
);
case!(
    framed_capture_retry_reuses_retained_bytes_and_exact_request,
    {
        let f = Fixture::new();
        let (directory, receipt) = f.capture("retained-capture");
        let before = frozen_config(&directory.join("launch.json")).unwrap();
        let response = dispatch(&f.request(IdentityHelperOperation::CaptureLaunch {
            scope: f.scope.clone(),
            directory: directory.clone(),
            request_digest: "a".repeat(64),
            base_config_path: None,
            account: Some(f.account.clone()),
            settings: Default::default(),
        }))
        .await;
        assert_eq!(
            response,
            serde_json::json!({"result":"value","value":receipt})
        );
        assert_eq!(
            frozen_config(&directory.join("launch.json")).unwrap(),
            before
        );
    }
);
case!(
    framed_capture_conflict_preserves_original_frozen_configuration,
    {
        let f = Fixture::new();
        let (directory, _) = f.capture("conflicting-capture");
        let before = frozen_config(&directory.join("launch.json")).unwrap();
        unavailable(
            dispatch(&f.request(IdentityHelperOperation::CaptureLaunch {
                scope: f.scope.clone(),
                directory: directory.clone(),
                request_digest: "c".repeat(64),
                base_config_path: None,
                account: Some(f.account.clone()),
                settings: Default::default(),
            }))
            .await,
        );
        assert_eq!(
            frozen_config(&directory.join("launch.json")).unwrap(),
            before
        );
    }
);
case!(
    framed_review_returns_current_ordinary_identity_and_private_namespace_digest,
    {
        let f = Fixture::new();
        let (directory, _) = f.capture("review");
        let response = dispatch(&f.request(IdentityHelperOperation::ReviewConfig {
            config_path: directory.join("launch.json"),
        }))
        .await;
        assert_eq!(response["result"], "facts");
        let facts: IdentityConfigFacts = serde_json::from_value(response["facts"].clone()).unwrap();
        assert_eq!(facts.account, f.account);
        assert_eq!(facts.uid, unsafe { libc::geteuid() });
        assert_eq!(facts.gid, unsafe { libc::getegid() });
        assert_eq!(
            facts.config_digest,
            config_digest(&frozen_config(&directory.join("launch.json")).unwrap())
        );
        assert!(
            !response
                .to_string()
                .contains("synthetic-only-helper-key-353")
        );
    }
);
case!(
    retired_missing_project_label_does_not_authorize_recovery_or_recreate_state,
    {
        let workspace = root().join("deleted-project");
        let request = recovery(&workspace);
        let directory = request.directory.clone();
        let bytes = frame(
            &serde_json::to_value(IdentityHelperRequest {
                schema: 1,
                workspace: workspace.clone(),
                operation: IdentityHelperOperation::RecoverBound { request },
            })
            .unwrap(),
        );
        unavailable(dispatch(&bytes).await);
        assert!(!workspace.exists() && !directory.exists());
    }
);
case!(
    recovery_workspace_mismatch_is_refused_before_any_journal_effect,
    {
        let workspace = root().join("deleted-project");
        let request = recovery(&root().join("different-project"));
        let directory = request.directory.clone();
        let bytes = frame(
            &serde_json::to_value(IdentityHelperRequest {
                schema: 1,
                workspace,
                operation: IdentityHelperOperation::RecoverBound { request },
            })
            .unwrap(),
        );
        unavailable(dispatch(&bytes).await);
        assert!(!directory.exists());
    }
);
case!(
    recovery_without_positive_retirement_never_opens_private_state,
    {
        let workspace = root();
        let mut request = recovery(&workspace);
        request.local_process_retired = false;
        let directory = request.directory.clone();
        let bytes = frame(
            &serde_json::to_value(IdentityHelperRequest {
                schema: 1,
                workspace,
                operation: IdentityHelperOperation::RecoverBound { request },
            })
            .unwrap(),
        );
        unavailable(dispatch(&bytes).await);
        assert!(!directory.exists());
    }
);
case!(
    framed_usage_without_root_pipe_cannot_refresh_or_publish_usage,
    {
        let f = Fixture::new();
        let before = f.registry.usage_cached(&f.account).unwrap();
        unavailable(
            dispatch(&f.request(IdentityHelperOperation::Usage {
                scope: f.scope.clone(),
                account: f.account.clone(),
                refresh: true,
            }))
            .await,
        );
        assert_eq!(
            serde_json::to_value(f.registry.usage_cached(&f.account).unwrap()).unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
);
case!(
    framed_default_change_without_root_pipe_preserves_host_default_revision,
    {
        let f = Fixture::new();
        let before = f.registry.default_account().unwrap();
        unavailable(
            dispatch(&f.request(IdentityHelperOperation::SetDefault {
                scope: f.scope.clone(),
                command_id: Uuid::new_v4(),
                account: f.account.clone(),
                expected_revision: before.0,
            }))
            .await,
        );
        assert_eq!(f.registry.default_account().unwrap(), before);
    }
);

case!(
    launch_preparation_validates_under_identity_without_frozen_config,
    {
        let f = Fixture::new();
        let before = fs::read(root().join("data/helm/accounts/registry.json")).unwrap();
        let response = dispatch(&f.request(IdentityHelperOperation::PrepareLaunch {
            scope: f.scope.clone(),
            account: f.account.clone(),
            settings: voyage_protocol::start_settings::StartSettings {
                max_output_tokens: Some(0),
                ..Default::default()
            },
        }))
        .await;
        assert_eq!(response["result"], "value");
        assert_eq!(
            response["value"]["account"],
            serde_json::to_value(&f.account).unwrap()
        );
        assert_eq!(response["value"]["settings"]["max_output_tokens"], 0);
        assert!(
            !response
                .to_string()
                .contains("synthetic-only-helper-key-353")
        );
        assert!(!response.to_string().contains("config_path"));
        assert_eq!(
            fs::read(root().join("data/helm/accounts/registry.json")).unwrap(),
            before
        );
        fs::remove_file(root().join("helper-request.bin")).unwrap();
        fs::remove_file(root().join("helper-response.bin")).unwrap();
        unavailable(
            dispatch(&f.request(IdentityHelperOperation::PrepareLaunch {
                scope: scope(&f.workspace),
                account: f.account.clone(),
                settings: Default::default(),
            }))
            .await,
        );
    }
);
