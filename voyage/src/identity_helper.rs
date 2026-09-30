//! One-shot identity-scoped observation. Never an agent loop or a session owner.
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;
use voyage_protocol::identity_helper::*;

fn digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut value = Sha256::new();
    value.update(domain);
    value.update(bytes);
    hex::encode(value.finalize())
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
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("launch parent missing"))?;
    let name = path.file_name().and_then(|v| v.to_str())
        .ok_or_else(|| anyhow::anyhow!("launch name unavailable"))?;
    let directory = crate::attachment::local_actor::storage::Directory::open(parent)?;
    let bytes = directory.read_bounded(name, IDENTITY_HELPER_BYTES)?
        .ok_or_else(|| anyhow::anyhow!("launch file unavailable"))?;
    Ok(bytes)
}

#[cfg(unix)]
fn review_config(workspace: &Path, path: &Path) -> Result<IdentityConfigFacts> {
    let bytes = frozen_config(path)?;
    let launch: crate::launch_config::LaunchConfig = serde_json::from_slice(&bytes)?;
    let config = launch.resolve(workspace)?;
    // A review may not silently select a mutable default, legacy cache or root
    // login credential. The explicit context comes from the supervisor's exec
    // environment, shared with the eventual independent runtime.
    let account = config.account.clone().ok_or_else(|| anyhow::anyhow!("explicit account required"))?;
    let registry = crate::accounts::Registry::default_host()?;
    let descriptor = registry.validate_binding(&account)?;
    config.validate_account()?;
    let policy = crate::runtime_policy::RuntimePolicy::resolve(&config, workspace)?;
    policy.policy().check_current()?;
    let root = dirs::data_local_dir().ok_or_else(|| anyhow::anyhow!("account namespace unavailable"))?
        .join("helm").join("accounts");
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
        account_root_digest: digest(b"voyage/identity-account-namespace/v1\0", root.as_os_str().as_encoded_bytes()),
        uid,
        gid,
        supplementary_groups,
    })
}

pub async fn run() -> Result<()> {
    use voyage_protocol::process::{read_frame, write_frame};
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let request: IdentityHelperRequest = read_frame(&mut tokio::io::stdin()).await?;
        ensure!(serde_json::to_vec(&request)?.len() <= IDENTITY_HELPER_BYTES, "identity helper request exceeds bounds");
        ensure!(request.schema == IDENTITY_HELPER_SCHEMA && request.workspace.is_absolute(), "invalid identity helper request");
        let workspace = request.workspace.canonicalize()?;
        ensure!(workspace == request.workspace && workspace.is_dir(), "canonical workspace required");
        #[cfg(unix)]
        let response = match request.operation {
            IdentityHelperOperation::ReviewConfig { config_path } => IdentityHelperResponse::Facts {
                facts: review_config(&workspace, &config_path)?,
            },
        };
        #[cfg(not(unix))]
        let response = IdentityHelperResponse::Unavailable;
        Ok::<_, anyhow::Error>(response)
    }).await;
    // Errors are deliberately collapsed, including parser and filesystem errors.
    let response = match result {
        Ok(Ok(response)) => response,
        _ => IdentityHelperResponse::Unavailable,
    };
    write_frame(&mut tokio::io::stdout(), &response).await?;
    Ok(())
}
