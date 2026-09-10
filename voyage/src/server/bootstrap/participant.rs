//! Portable parent limits intersect the executing host's local configuration.
use super::*;
pub(crate) fn limit(config: &mut Config, registration: &ProcessRegistration) -> Result<()> {
    let Some(RuntimeInitialization::Participant { policy, .. }) = &registration.initialize else {
        return Ok(());
    };
    let mode = match policy.access.as_str() {
        "read_only" | "read-only" => crate::config::AccessMode::ReadOnly,
        "approval" => crate::config::AccessMode::Approval,
        "unrestricted" => crate::config::AccessMode::Unrestricted,
        _ => anyhow::bail!("invalid participant access ceiling"),
    };
    ensure!(
        policy.timeout_secs > 0 && policy.max_output_bytes > 0,
        "invalid participant resource ceiling"
    );
    let mut ceiling = config.clone();
    ceiling.access = Some(mode);
    ceiling.allow_read.clear();
    ceiling.allow_write.clear();
    ceiling
        .inherit_env
        .retain(|name| policy.inherit_env.contains(name));
    ceiling.github_enabled &= policy.github_enabled;
    // Parent paths never become paths on this machine. The accepted workspace is
    // the complete filesystem delegation, intersected with host policy below.
    config.allow_read.clear();
    config.allow_write.clear();
    config.participants.clear();
    crate::policy::Policy::new(&ceiling, registration.workspace.clone())?
        .limit_child_config(config, &registration.workspace)?;
    config.command_timeout_secs = config.command_timeout_secs.min(policy.timeout_secs);
    config.max_output_bytes = config.max_output_bytes.min(policy.max_output_bytes);
    config.subagent_max_concurrency = config.subagent_max_concurrency.min(policy.max_subagents);
    crate::runtime_policy::RuntimePolicy::resolve(config, &registration.workspace)?;
    Ok(())
}
