//! One-shot identity-scoped observation. Never an agent loop or a session owner.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;
use voyage_protocol::identity_helper::*;

const PRIVATE_CONFIG_BYTES: usize = 65_536;
#[cfg(target_os = "linux")]
mod authority;
mod initialization;
mod recovery;

fn digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut value = Sha256::new();
    value.update(domain);
    value.update(bytes);
    hex::encode(value.finalize())
}

pub(crate) fn config_digest(bytes: &[u8]) -> String {
    digest(b"voyage/identity-launch-config/v1\0", bytes)
}

#[cfg(unix)]
fn actual_identity() -> Result<(u32, u32, Vec<u32>)> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    ensure!(
        unsafe { libc::getuid() } == uid && unsafe { libc::getgid() } == gid,
        "inconsistent helper identity"
    );
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    ensure!((0..=64).contains(&count), "helper group bound exceeded");
    let mut groups = vec![0; count as usize];
    ensure!(
        unsafe { libc::getgroups(count, groups.as_mut_ptr()) } == count,
        "helper groups unavailable"
    );
    groups.retain(|group| *group != gid);
    groups.sort_unstable();
    groups.dedup();
    Ok((uid, gid, groups))
}

/// Open only through the executing identity's private-file boundary. Root never
/// reads this file on behalf of an ordinary identity.
fn frozen_config(path: &Path) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "absolute private launch file required");
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("launch parent missing"))?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| anyhow::anyhow!("launch name unavailable"))?;
    let directory = crate::attachment::local_actor::storage::Directory::open_existing(parent)?;
    let bytes = directory
        .read_bounded(name, PRIVATE_CONFIG_BYTES)?
        .ok_or_else(|| anyhow::anyhow!("launch file unavailable"))?;
    Ok(bytes)
}

fn allowed(
    scope: &IdentityAccountScope,
    registry: &crate::accounts::Registry,
    account: &voyage_protocol::accounts::AccountBinding,
    workspace: &Path,
) -> bool {
    scope.can_use
        && (scope.full_access
            || scope.account_ids.contains(&account.account_id)
            || (scope.can_enroll
                && scope
                    .enrollment_connections
                    .contains(&account.connection_id)
                && registry
                    .enrollment_actor(account.account_id)
                    .ok()
                    .flatten()
                    .as_ref()
                    == Some(&scope.actor)
                && scope.actor.workspace == workspace.to_string_lossy()))
}

fn use_account(
    scope: &IdentityAccountScope,
    registry: &crate::accounts::Registry,
    account: &voyage_protocol::accounts::AccountBinding,
    workspace: &Path,
) -> Result<voyage_protocol::accounts::AccountDescriptor> {
    ensure!(
        allowed(scope, registry, account, workspace),
        "account use denied"
    );
    registry.validate_binding(account)
}

fn config_for(workspace: &Path, base: Option<&Path>) -> Result<crate::Config> {
    match base {
        Some(path) => {
            serde_json::from_slice::<crate::launch_config::LaunchConfig>(&frozen_config(path)?)?
                .resolve(workspace)
        }
        None => crate::Config::load(None),
    }
}

fn select(
    scope: &IdentityAccountScope,
    workspace: &Path,
    account: Option<voyage_protocol::accounts::AccountBinding>,
    base: Option<&Path>,
) -> Result<crate::Config> {
    let registry = crate::accounts::Registry::default_host()?;
    let mut config = config_for(workspace, base)?;
    let account = match account.or_else(|| config.account.clone()) {
        Some(account) => account,
        None => registry
            .default_account()?
            .1
            .ok_or_else(|| anyhow::anyhow!("default account required"))?,
    };
    use_account(scope, &registry, &account, workspace)?;
    config.select_account(account)?;
    Ok(config)
}

fn validate_profile(
    scope: &IdentityAccountScope,
    workspace: &Path,
    profile: &voyage_protocol::execution_profiles::ExecutionProfile,
) -> Result<()> {
    let mut config = select(scope, workspace, Some(profile.account.clone()), None)?;
    config.model = profile.model.clone();
    config.reasoning_effort = profile.reasoning_effort.clone();
    config.service_tier = profile.service_tier.clone();
    crate::provider::validate_inference_settings(&config)?;
    let redactor = crate::build::redactor(&config);
    ensure!(
        [&profile.name, &profile.model]
            .into_iter()
            .all(|text| !redactor.contains_secret(text)),
        "private profile content"
    );
    config.validate_account()?;
    Ok(())
}

fn capture_launch(
    scope: &IdentityAccountScope,
    workspace: &Path,
    directory: &Path,
    request: &str,
    base: Option<&Path>,
    account: Option<voyage_protocol::accounts::AccountBinding>,
    settings: &voyage_protocol::start_settings::StartSettings,
) -> Result<serde_json::Value> {
    use crate::attachment::local_actor::storage::Directory;
    ensure!(
        request.len() == 64 && request.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid frozen request digest"
    );
    let storage = Directory::open_existing(directory)?;
    let _lock = storage.lock()?;
    let path = directory.join("launch.json");
    if let Some(prior) = storage.read_bounded("request.json", 256)? {
        ensure!(prior == request.as_bytes(), "frozen request conflict");
        // An interrupted capture is retained as uncertain; never select changed
        // defaults or overwrite a previously frozen launch on retry.
        let bytes = frozen_config(&path)?;
        let launch: crate::launch_config::LaunchConfig = serde_json::from_slice(&bytes)?;
        let config = serde_json::from_slice::<crate::launch_config::LaunchConfig>(&bytes)?
            .resolve(workspace)?;
        let registry = crate::accounts::Registry::default_host()?;
        use_account(
            scope,
            &registry,
            config
                .account
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("explicit account required"))?,
            workspace,
        )?;
        let mut expected = config.clone();
        if let Some(account) = account {
            expected.select_account(account)?;
        }
        crate::start_settings::apply(&mut expected, settings)?;
        ensure!(
            launch.matches_config(&expected)?,
            "frozen configuration differs from retained request"
        );
        return Ok(
            serde_json::json!({"config_path":path,"config_digest":digest(b"voyage/identity-launch-config/v1\0", &bytes)}),
        );
    }
    ensure!(
        storage
            .read_bounded("launch.json", PRIVATE_CONFIG_BYTES)?
            .is_none(),
        "unmatched frozen launch file"
    );
    let mut config = select(scope, workspace, account, base)?;
    crate::start_settings::apply(&mut config, settings)?;
    let launch = crate::launch_config::LaunchConfig::capture(&config, workspace)?;
    let bytes = serde_json::to_vec(&launch)?;
    ensure!(
        bytes.len() <= PRIVATE_CONFIG_BYTES,
        "frozen configuration exceeds bounds"
    );
    // Publication intent precedes the effect. Missing configuration after this
    // point is an unresolved capture, rather than permission to recapture.
    storage.publish_new("request.json", request.as_bytes())?;
    storage.publish_new("launch.json", &bytes)?;
    Ok(
        serde_json::json!({"config_path":path,"config_digest":digest(b"voyage/identity-launch-config/v1\0", &bytes)}),
    )
}

async fn account_operation(
    workspace: &Path,
    operation: IdentityHelperOperation,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let registry = crate::accounts::Registry::default_host()?;
    match operation {
        operation @ (IdentityHelperOperation::Enrollment { .. }
        | IdentityHelperOperation::Usage { .. }
        | IdentityHelperOperation::SetDefault { .. }) => {
            #[cfg(target_os = "linux")]
            {
                network_operation(workspace, registry, operation).await
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = operation;
                anyhow::bail!("identity account authority unavailable")
            }
        }
        IdentityHelperOperation::Accounts { scope, transport } => {
            ensure!(
                scope.can_use || scope.can_enroll,
                "account observation denied"
            );
            registry.ensure_chatgpt_connection()?;
            let (revision, all) = registry.list(|_| scope.can_use)?;
            let accounts: Vec<_> = all
                .into_iter()
                .filter(|account| {
                    scope.full_access
                        || scope.account_ids.contains(&account.id)
                        || (scope.can_enroll
                            && scope
                                .enrollment_connections
                                .contains(&account.connection_id)
                            && registry
                                .enrollment_actor(account.id)
                                .ok()
                                .flatten()
                                .as_ref()
                                == Some(&scope.actor))
                })
                .filter(|account| {
                    transport.is_none_or(|t| {
                        registry
                            .connection(account.connection_id)
                            .is_ok_and(|connection| connection.transports.contains(&t))
                    })
                })
                .collect();
            let connections: Vec<_> = registry
                .connections()?
                .into_iter()
                .filter(|connection| {
                    accounts
                        .iter()
                        .any(|account| account.connection_id == connection.id)
                        || (scope.can_enroll
                            && (scope.full_access
                                || scope.enrollment_connections.contains(&connection.id)))
                })
                .filter(|connection| transport.is_none_or(|t| connection.transports.contains(&t)))
                .collect();
            let (default_revision, default_account) = registry.default_account()?;
            let default_account = default_account.filter(|binding| {
                accounts
                    .iter()
                    .any(|account| account.id == binding.account_id)
            });
            Ok(
                json!({"revision":revision,"accounts":accounts,"connections":connections,"default_revision":default_revision,"default_account":default_account,"can_set_default":false}),
            )
        }
        IdentityHelperOperation::Defaults {
            scope,
            profile,
            profiles_revision,
        } => {
            if profiles_revision > 0 && profile.is_none() {
                return Ok(json!({"code":"default_profile_required","account":null}));
            }
            if profile.is_none()
                && config_for(workspace, None)?.account.is_none()
                && registry.default_account()?.1.is_none()
            {
                return Ok(json!({"code":"default_account_required","account":null}));
            }
            let mut config = select(
                &scope,
                workspace,
                profile.as_ref().map(|p| p.account.clone()),
                None,
            )?;
            if let Some(profile) = profile {
                validate_profile(&scope, workspace, &profile)?;
                config.model = profile.model;
                config.reasoning_effort = profile.reasoning_effort;
                config.service_tier = profile.service_tier;
            }
            let redactor = crate::build::redactor(&config);
            crate::provider::validate_models_for_display(
                &[crate::provider::ModelInfo::minimal(config.model.clone())],
                |text| redactor.contains_secret(text),
            )?;
            Ok(
                json!({"account":config.account,"provider":config.provider,"model":config.model,"reasoning_effort":config.reasoning_effort,"service_tier":config.service_tier}),
            )
        }
        IdentityHelperOperation::Models { scope, account } => {
            let descriptor = use_account(&scope, &registry, &account, workspace)?;
            let mut config = select(&scope, workspace, Some(account.clone()), None)?;
            config.reasoning_effort = None;
            config.service_tier = None;
            let context = crate::provider::inference_context(&config).await;
            let models = crate::server::models::discover(&config, workspace).await?;
            ensure!(
                context.is_some()
                    && crate::provider::inference_context(&config).await == context
                    && registry.validate_binding(&account)?.capability_revision
                        == descriptor.capability_revision,
                "model account context changed"
            );
            Ok(
                json!({"account":account,"account_label":descriptor.label,"capability_revision":descriptor.capability_revision,"models":models}),
            )
        }
        IdentityHelperOperation::ValidateAccount { scope, account } => {
            let descriptor = use_account(&scope, &registry, &account, workspace)?;
            Ok(json!({"account":account,"capability_revision":descriptor.capability_revision}))
        }
        IdentityHelperOperation::ObserveAccountIntent { scope, account } => {
            ensure!(
                allowed(&scope, &registry, &account, workspace),
                "account intent observation denied"
            );
            Ok(json!({"permitted":true}))
        }
        IdentityHelperOperation::ValidateProfile { scope, profile } => {
            validate_profile(&scope, workspace, &profile)?;
            Ok(json!({"valid":true,"account":profile.account}))
        }
        IdentityHelperOperation::CaptureInitialization {
            scope,
            directory,
            request_digest,
            base_config_path,
            initialize,
        } => initialization::capture(
            &scope,
            workspace,
            &directory,
            &request_digest,
            base_config_path.as_deref(),
            &initialize,
        ),
        IdentityHelperOperation::CaptureLaunch {
            scope,
            directory,
            request_digest,
            base_config_path,
            account,
            settings,
        } => capture_launch(
            &scope,
            workspace,
            &directory,
            &request_digest,
            base_config_path.as_deref(),
            account,
            &settings,
        ),
        IdentityHelperOperation::ObserveTransferArtifact { artifact_path } => {
            initialization::observe_transfer(&artifact_path)
        }
        IdentityHelperOperation::RecoverBound { .. } => {
            anyhow::bail!("recovery requires isolated helper dispatch")
        }
        IdentityHelperOperation::ReviewConfig { .. } => {
            anyhow::bail!("unsupported account operation")
        }
    }
}

#[cfg(target_os = "linux")]
struct UsageAuthority {
    pipe: std::sync::Arc<authority::Pipe>,
    scope: IdentityAccountScope,
    registry: crate::accounts::Registry,
    workspace: std::path::PathBuf,
    account: voyage_protocol::accounts::AccountBinding,
    capability_revision: u64,
}
#[cfg(target_os = "linux")]
impl std::fmt::Debug for UsageAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdentityUsageAuthority")
    }
}
#[cfg(target_os = "linux")]
impl crate::policy::ExecutionAuthority for UsageAuthority {
    fn check(&self) -> Result<()> {
        ensure!(
            self.pipe.allows(
                &self.scope.actor,
                IdentityAuthorityRight::Use,
                self.account.connection_id,
                Some(self.account.account_id)
            ),
            "account authority unavailable"
        );
        ensure!(
            use_account(&self.scope, &self.registry, &self.account, &self.workspace)?
                .capability_revision
                == self.capability_revision,
            "account capability changed"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn network_operation(
    workspace: &Path,
    registry: crate::accounts::Registry,
    operation: IdentityHelperOperation,
) -> Result<serde_json::Value> {
    use crate::policy::ExecutionAuthority;
    use serde_json::json;
    use std::sync::Arc;
    use voyage_protocol::accounts::*;
    let pipe = Arc::new(authority::Pipe::open()?);
    match operation {
        IdentityHelperOperation::Enrollment { scope, operation } => {
            ensure!(
                scope.can_enroll && scope.actor.workspace == workspace.to_string_lossy(),
                "enrollment scope refused"
            );
            registry.ensure_chatgpt_connection()?;
            let current = scope.clone();
            let peer = pipe.clone();
            let service = crate::accounts::device::DeviceService::new(
                registry,
                Arc::new(move |actor, connection| {
                    actor == &current.actor
                        && (current.full_access
                            || current.enrollment_connections.contains(&connection))
                        && peer.allows(actor, IdentityAuthorityRight::Enroll, connection, None)
                }),
            );
            match operation {
                IdentityEnrollmentOperation::Start { request } => {
                    ensure!(request.actor == scope.actor, "enrollment actor changed");
                    Ok(serde_json::to_value(service.start(request).await?)?)
                }
                IdentityEnrollmentOperation::Resolve { request } => {
                    ensure!(request.actor == scope.actor, "enrollment actor changed");
                    Ok(serde_json::to_value(service.resolve(request)?)?)
                }
                IdentityEnrollmentOperation::Drive { enrollment_id } => Ok(serde_json::to_value(
                    service.drive(enrollment_id, &scope.actor).await?,
                )?),
                IdentityEnrollmentOperation::Status { enrollment_id } => Ok(serde_json::to_value(
                    service.status(enrollment_id, &scope.actor)?,
                )?),
                IdentityEnrollmentOperation::Cancel {
                    command_id,
                    enrollment_id,
                } => Ok(serde_json::to_value(service.cancel(
                    command_id,
                    enrollment_id,
                    &scope.actor,
                )?)?),
            }
        }
        IdentityHelperOperation::Usage {
            scope,
            account,
            refresh,
        } => {
            let descriptor = use_account(&scope, &registry, &account, workspace)?;
            let authority = Arc::new(UsageAuthority {
                pipe,
                scope: scope.clone(),
                registry: registry.clone(),
                workspace: workspace.to_owned(),
                account: account.clone(),
                capability_revision: descriptor.capability_revision,
            });
            authority.check()?;
            let mut observation = registry.usage_cached(&account)?;
            if refresh && account.transport == Transport::ChatgptOauth {
                let mut config = select(&scope, workspace, Some(account.clone()), None)?;
                config.select_account(account.clone())?;
                crate::runtime_policy::RuntimePolicy::resolve(&config, workspace)?
                    .policy()
                    .check_current()?;
                let provider = registry
                    .oauth_provider(&account)?
                    .with_authority(Some(authority.clone()));
                observation.attempted_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_secs() as i64,
                );
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    provider.account_usage(),
                )
                .await;
                observation.refresh_status = match result {
                    Ok(Ok(snapshot)) => {
                        observation.snapshot = Some(snapshot);
                        AccountUsageRefreshStatus::Available
                    }
                    Ok(Err(error)) => match error.category() {
                        "authentication" => AccountUsageRefreshStatus::SignInRequired,
                        "rate_limit" => AccountUsageRefreshStatus::RateLimited,
                        "invalid_response" => AccountUsageRefreshStatus::InvalidResponse,
                        _ => AccountUsageRefreshStatus::Unavailable,
                    },
                    Err(_) => AccountUsageRefreshStatus::Unavailable,
                };
                authority.check()?;
                registry.publish_usage(observation)?;
                observation = registry.usage_cached(&account)?;
            }
            authority.check()?;
            ensure!(
                observation.capability_revision == descriptor.capability_revision,
                "account usage context changed"
            );
            Ok(serde_json::to_value(observation)?)
        }
        IdentityHelperOperation::SetDefault {
            scope,
            command_id,
            account,
            expected_revision,
        } => {
            ensure!(
                scope.full_access && scope.actor.principal == "owner",
                "host owner is required for default account changes"
            );
            use_account(&scope, &registry, &account, workspace)?;
            ensure!(
                pipe.allows(
                    &scope.actor,
                    IdentityAuthorityRight::Use,
                    account.connection_id,
                    Some(account.account_id)
                ),
                "default account authority changed"
            );
            let (revision, binding) =
                registry.set_default_account(command_id, workspace, expected_revision, account)?;
            Ok(json!({"default_revision":revision,"default_account":binding}))
        }
        _ => anyhow::bail!("identity account operation refused"),
    }
}

#[cfg(unix)]
fn review_config(workspace: &Path, path: &Path) -> Result<IdentityConfigFacts> {
    let bytes = frozen_config(path)?;
    let launch: crate::launch_config::LaunchConfig = serde_json::from_slice(&bytes)?;
    let config = launch.resolve(workspace)?;
    // A review may not silently select a mutable default, legacy cache or root
    // login credential. The explicit context comes from the supervisor's exec
    // environment, shared with the eventual independent runtime.
    let account = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("explicit account required"))?;
    let registry = crate::accounts::Registry::default_host()?;
    let descriptor = registry.validate_binding(&account)?;
    config.validate_account()?;
    let policy = crate::runtime_policy::RuntimePolicy::resolve(&config, workspace)?;
    policy.policy().check_current()?;
    let root = dirs::data_local_dir()
        .ok_or_else(|| anyhow::anyhow!("account namespace unavailable"))?
        .join("helm")
        .join("accounts");
    let root = root.canonicalize()?;
    let (uid, gid, supplementary_groups) = actual_identity()?;
    ensure!(
        registry.validate_binding(&account)?.capability_revision == descriptor.capability_revision
            && frozen_config(path)? == bytes,
        "review context changed during observation"
    );
    policy.policy().check_current()?;
    Ok(IdentityConfigFacts {
        account,
        capability_revision: descriptor.capability_revision,
        policy_digest: policy.policy().effective().digest().to_owned(),
        config_digest: digest(b"voyage/identity-launch-config/v1\0", &bytes),
        account_root_digest: digest(
            b"voyage/identity-account-namespace/v1\0",
            root.as_os_str().as_encoded_bytes(),
        ),
        uid,
        gid,
        supplementary_groups,
    })
}

pub async fn run() -> Result<()> {
    use voyage_protocol::process::{read_frame, write_frame};
    let result = tokio::time::timeout(std::time::Duration::from_secs(50), async {
        let request: IdentityHelperRequest = read_frame(&mut tokio::io::stdin()).await?;
        ensure!(
            serde_json::to_vec(&request)?.len() <= IDENTITY_HELPER_BYTES,
            "identity helper request exceeds bounds"
        );
        ensure!(
            request.schema == IDENTITY_HELPER_SCHEMA && request.workspace.is_absolute(),
            "invalid identity helper request"
        );
        #[cfg(unix)]
        let response = match request.operation {
            IdentityHelperOperation::RecoverBound {
                request: recovery_request,
            } => {
                // Retired journal bookkeeping uses the stored workspace as an
                // identity label. It does not enter or recreate the project.
                recovery::validate_workspace(&request.workspace, &recovery_request)?;
                IdentityHelperResponse::Value {
                    value: recovery::recover(recovery_request).await?,
                }
            }
            IdentityHelperOperation::ReviewConfig { config_path } => {
                let workspace = canonical_workspace(&request.workspace)?;
                IdentityHelperResponse::Facts {
                    facts: review_config(&workspace, &config_path)?,
                }
            }
            operation => {
                let workspace = canonical_workspace(&request.workspace)?;
                IdentityHelperResponse::Value {
                    value: account_operation(&workspace, operation).await?,
                }
            }
        };
        #[cfg(not(unix))]
        let response = IdentityHelperResponse::Unavailable {};
        Ok::<_, anyhow::Error>(response)
    })
    .await;
    // Errors are deliberately collapsed, including parser and filesystem errors.
    let response = match result {
        Ok(Ok(response)) => response,
        _ => IdentityHelperResponse::Unavailable {},
    };
    write_frame(&mut tokio::io::stdout(), &response).await?;
    Ok(())
}

#[cfg(unix)]
fn canonical_workspace(requested: &Path) -> Result<std::path::PathBuf> {
    let workspace = requested.canonicalize()?;
    ensure!(
        workspace == requested && workspace.is_dir(),
        "canonical workspace required"
    );
    Ok(workspace)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn ordinary_workspace_checks_still_refuse_missing_or_aliased_projects() {
        let fixture = tempfile::tempdir().unwrap();
        let canonical = fixture.path().canonicalize().unwrap();
        assert_eq!(canonical_workspace(&canonical).unwrap(), canonical);
        let missing = canonical.join("missing-project");
        assert!(canonical_workspace(&missing).is_err());
        assert!(!missing.exists());
        let alias = canonical.join("project-alias");
        std::os::unix::fs::symlink(&canonical, &alias).unwrap();
        assert!(canonical_workspace(&alias).is_err());
    }

    #[test]
    fn frozen_observation_is_private_bounded_and_does_not_create_a_missing_parent() {
        let fixture = tempfile::tempdir().unwrap();
        let absent = fixture.path().join("missing").join("launch.json");
        assert!(frozen_config(&absent).is_err());
        assert!(!absent.parent().unwrap().exists());
        let path = fixture.path().join("launch.json");
        std::fs::write(&path, b"private frozen bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(frozen_config(&path).unwrap(), b"private frozen bytes");
        std::fs::write(&path, vec![b'x'; PRIVATE_CONFIG_BYTES + 1]).unwrap();
        assert!(frozen_config(&path).is_err());
        std::fs::write(&path, b"private frozen bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(frozen_config(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let alias = fixture.path().join("alias.json");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(frozen_config(&path).is_err());
    }

    #[test]
    fn helper_failure_frame_cannot_include_private_diagnostics_or_configuration() {
        let encoded = serde_json::to_value(IdentityHelperResponse::Unavailable {}).unwrap();
        assert_eq!(encoded, serde_json::json!({"result":"unavailable"}));
        let malformed = serde_json::json!({"result":"unavailable","error":"secret configuration"});
        assert!(serde_json::from_value::<IdentityHelperResponse>(malformed).is_err());
    }
}
