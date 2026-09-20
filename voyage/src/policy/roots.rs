//! Run-owned root overlay, never shared with subordinate policies.
use super::*;
use crate::tools::roots::RootPermission;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub(crate) struct RootCandidate {
    path: PathBuf,
    permission: RootPermission,
    #[cfg(unix)]
    identity: (u64, u64),
}
impl RootCandidate {
    fn verify(&self) -> Result<()> {
        anyhow::ensure!(
            self.path.canonicalize()? == self.path && self.path.is_dir(),
            "root moved or disappeared"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(&self.path)?;
            anyhow::ensure!((m.dev(), m.ino()) == self.identity, "root identity changed");
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Grant {
    root: RootCandidate,
    cancellation: CancellationToken,
    access: u64,
}
#[derive(Debug, Default)]
pub(crate) struct RunRoots {
    grants: Vec<Grant>,
    generation: u64,
    run: Option<uuid::Uuid>,
}
impl Policy {
    pub(crate) fn for_run_dispatch(&self, run: uuid::Uuid) -> Self {
        if let Some(shared) = &self.run_roots
            && let Ok(mut state) = shared.lock()
            && state.run != Some(run)
        {
            state.grants.clear();
            state.generation = state.generation.wrapping_add(1);
            state.run = Some(run);
        }
        self.for_dispatch()
    }
    pub(crate) fn has_root_overlay(&self) -> bool {
        self.dispatch_sandbox.is_some()
    }
    pub(crate) fn enable_run_roots(mut self) -> Self {
        self.run_roots = Some(Arc::new(Mutex::new(RunRoots::default())));
        self
    }
    fn validate_root_scope(&self) -> Result<()> {
        anyhow::ensure!(
            cfg!(all(target_os = "linux", target_arch = "x86_64")),
            "runtime root grants are unsupported on this platform"
        );
        anyhow::ensure!(
            !self.live_ceiling && self.run_roots.is_some() && self.live_access.is_some(),
            "root grants require a foreground managed run"
        );
        self.check_current()?;
        Ok(())
    }
    pub(crate) fn prepare_root(
        &self,
        path: &Path,
        permission: RootPermission,
    ) -> Result<RootCandidate> {
        self.validate_root_scope()?;
        anyhow::ensure!(
            path.is_absolute() && path.canonicalize()? == path && path.is_dir(),
            "root must be the exact canonical absolute existing directory"
        );
        anyhow::ensure!(
            path.to_str()
                .is_some_and(|p| !p.chars().any(char::is_control)),
            "root path must be printable UTF-8"
        );
        let candidate = RootCandidate {
            path: path.into(),
            permission,
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                let m = std::fs::metadata(path)?;
                (m.dev(), m.ino())
            },
        };
        self.root_sandbox(std::slice::from_ref(&candidate))?;
        Ok(candidate)
    }
    // Resolve through the executing host ceiling, then require the EXACT requested
    // directory to survive. Intersection with a narrower subtree is not consent.
    fn root_sandbox(&self, roots: &[RootCandidate]) -> Result<crate::sandbox::Sandbox> {
        let rules = self.snapshot.effective.rules();
        let strings = |paths: &[PathBuf]| -> Result<Vec<String>> {
            paths
                .iter()
                .map(|p| {
                    p.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 root"))
                })
                .collect()
        };
        let mut base = crate::policy_profile::Rules {
            access: rules.access,
            unattended: rules.unattended.clone(),
            read_roots: strings(&rules.read_roots)?,
            write_roots: strings(&rules.write_roots)?,
            legacy_deny_commands: vec![],
            inherit_env: rules.inherit_env.clone(),
            github_enabled: rules.github_enabled,
        };
        for root in roots {
            root.verify()?;
            base.read_roots.push(root.path.to_str().unwrap().into());
            if root.permission == RootPermission::Write {
                base.write_roots.push(root.path.to_str().unwrap().into());
            }
        }
        let effective = crate::policy_profile::resolve_runtime_layers(&self.workspace, &base, &[])?;
        self.check_root_ceiling(roots, &effective)?;
        Ok(crate::sandbox::Sandbox::new(
            &self.snapshot.sandbox.settings,
            &effective.rules().read_roots,
            &effective.rules().write_roots,
        )?)
    }
    fn check_root_ceiling(
        &self,
        roots: &[RootCandidate],
        effective: &crate::policy_profile::EffectivePolicy,
    ) -> Result<()> {
        anyhow::ensure!(
            effective.ceiling_digest() == self.snapshot.effective.ceiling_digest(),
            "host ceiling changed"
        );
        for root in roots {
            anyhow::ensure!(
                within_any(&root.path, &effective.rules().read_roots),
                "host ceiling refuses read root"
            );
            if root.permission == RootPermission::Write {
                anyhow::ensure!(
                    self.access_mode() != AccessMode::ReadOnly
                        && effective.rules().access != AccessMode::ReadOnly
                        && within_any(&root.path, &effective.rules().write_roots),
                    "host ceiling or access mode refuses write root"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn install_root(
        &self,
        root: RootCandidate,
        cancellation: CancellationToken,
    ) -> Result<()> {
        self.validate_root_scope()?;
        root.verify()?;
        let mut state = self
            .run_roots
            .as_ref()
            .unwrap()
            .lock()
            .map_err(|_| anyhow::anyhow!("root state poisoned"))?;
        self.check_execution_authority_without_roots()?;
        anyhow::ensure!(
            self.dispatch_roots.is_none_or(|g| g == state.generation),
            "root state changed during consent"
        );
        anyhow::ensure!(!cancellation.is_cancelled(), "run cancelled");
        anyhow::ensure!(state.grants.len() < 64, "run root grant limit reached");
        let mut combined: Vec<_> = state.grants.iter().map(|g| g.root.clone()).collect();
        combined.push(root.clone());
        self.root_sandbox(&combined)?;
        self.check_execution_authority_without_roots()?;
        anyhow::ensure!(!cancellation.is_cancelled(), "run cancelled");
        state.grants.push(Grant {
            root,
            cancellation,
            access: self.live_access.as_ref().unwrap().snapshot(),
        });
        state.generation = state.generation.wrapping_add(1);
        Ok(())
    }
    pub(super) fn snapshot_roots(&mut self) -> Result<()> {
        if self.live_ceiling {
            return Ok(());
        }
        let Some(shared) = &self.run_roots else {
            return Ok(());
        };
        let mut state = shared
            .lock()
            .map_err(|_| anyhow::anyhow!("root state poisoned"))?;
        let access = self.live_access.as_ref().map(|a| a.snapshot());
        let before = state.grants.len();
        state
            .grants
            .retain(|g| !g.cancellation.is_cancelled() && Some(g.access) == access);
        if before != state.grants.len() {
            state.generation = state.generation.wrapping_add(1);
        }
        let roots: Vec<_> = state.grants.iter().map(|g| g.root.clone()).collect();
        if !roots.is_empty() {
            self.dispatch_sandbox = Some(self.root_sandbox(&roots)?);
            for root in roots {
                self.readable.push(root.path.clone());
                if root.permission == RootPermission::Write {
                    self.writable.push(root.path);
                }
            }
        }
        self.dispatch_roots = Some(state.generation);
        Ok(())
    }
    pub(super) fn check_roots(&self) -> Result<()> {
        if let (Some(shared), Some(generation)) = (&self.run_roots, self.dispatch_roots) {
            let state = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("root state poisoned"))?;
            anyhow::ensure!(
                state.generation == generation,
                "root grants changed; retry dispatch"
            );
            for grant in &state.grants {
                anyhow::ensure!(
                    !grant.cancellation.is_cancelled()
                        && self
                            .live_access
                            .as_ref()
                            .is_some_and(|a| a.snapshot() == grant.access),
                    "root grant revoked"
                );
                grant.root.verify()?;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;
    fn policy(path: &Path) -> Policy {
        Policy::new(
            &crate::Config {
                access: Some(AccessMode::Unrestricted),
                ..Default::default()
            },
            path.into(),
        )
        .unwrap()
        .with_live_access(Some(Arc::new(LiveAccess::new(AccessMode::Unrestricted))))
        .enable_run_roots()
    }
    #[test]
    fn root_grants_are_exact_revocable_and_not_inherited() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let p = policy(workspace.path());
        let run = uuid::Uuid::new_v4();
        let dispatch = p.for_run_dispatch(run);
        assert!(dispatch.resolve_read(external.path()).is_err());
        assert!(
            dispatch
                .prepare_root(Path::new("."), RootPermission::Read)
                .is_err()
        );
        let alias = workspace.path().join("alias");
        std::os::unix::fs::symlink(external.path(), &alias).unwrap();
        assert!(dispatch.prepare_root(&alias, RootPermission::Read).is_err());
        let cancel = CancellationToken::new();
        dispatch
            .install_root(
                dispatch
                    .prepare_root(external.path(), RootPermission::Read)
                    .unwrap(),
                cancel.clone(),
            )
            .unwrap();
        assert!(dispatch.check_current().is_err()); // old admissions invalidated
        let current = p.for_run_dispatch(run);
        assert!(current.resolve_read(external.path()).is_ok());
        assert!(current.resolve_write(&external.path().join("new")).is_err());
        assert!(current.check_delegated_workspace(external.path()).is_err());
        let mut child = policy(workspace.path());
        child.inherit_execution_authority(&current);
        assert!(
            child
                .for_run_dispatch(run)
                .resolve_read(external.path())
                .is_err()
        );
        assert!(
            child
                .prepare_root(external.path(), RootPermission::Read)
                .is_err()
        );
        cancel.cancel();
        assert!(current.resolve_read(external.path()).is_err());
        assert!(
            p.for_run_dispatch(run)
                .resolve_read(external.path())
                .is_err()
        );
        let d = p.for_run_dispatch(run);
        d.install_root(
            d.prepare_root(external.path(), RootPermission::Write)
                .unwrap(),
            CancellationToken::new(),
        )
        .unwrap();
        assert!(
            p.for_run_dispatch(run)
                .resolve_write(&external.path().join("new"))
                .is_ok()
        );
        p.live_access
            .as_ref()
            .unwrap()
            .update(AccessMode::Unrestricted);
        assert!(
            p.for_run_dispatch(run)
                .resolve_read(external.path())
                .is_err()
        );
        let d = p.for_run_dispatch(run);
        d.install_root(
            d.prepare_root(external.path(), RootPermission::Read)
                .unwrap(),
            CancellationToken::new(),
        )
        .unwrap();
        assert!(
            p.for_run_dispatch(uuid::Uuid::new_v4())
                .resolve_read(external.path())
                .is_err()
        );
    }
    #[test]
    fn root_replacement_and_readonly_write_consent_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        let outer = tempfile::tempdir().unwrap();
        let path = outer.path().join("root");
        std::fs::create_dir(&path).unwrap();
        let p = policy(workspace.path());
        let d = p.for_run_dispatch(uuid::Uuid::new_v4());
        let root = d.prepare_root(&path, RootPermission::Read).unwrap();
        std::fs::rename(&path, outer.path().join("old")).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(d.install_root(root, CancellationToken::new()).is_err());
        p.live_access.as_ref().unwrap().update(AccessMode::ReadOnly);
        assert!(
            p.for_dispatch()
                .prepare_root(&path, RootPermission::Write)
                .is_err()
        );
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod sandbox_tests {
    use super::*;
    #[test]
    fn root_overlay_changes_new_sandbox_mounts_without_widening_siblings() {
        let workspace = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("approved");
        let sibling = parent.path().join("not-approved");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        std::fs::write(root.join("data"), "visible").unwrap();
        std::fs::write(sibling.join("data"), "private").unwrap();
        let config = crate::Config {
            access: Some(AccessMode::Unrestricted),
            sandbox: crate::sandbox::Settings {
                mode: crate::sandbox::Mode::Required,
                ..Default::default()
            },
            ..Default::default()
        };
        let policy = Policy::new(&config, workspace.path().into())
            .unwrap()
            .with_live_access(Some(Arc::new(LiveAccess::new(AccessMode::Unrestricted))))
            .enable_run_roots();
        let run = uuid::Uuid::new_v4();
        let dispatch = policy.for_run_dispatch(run);
        dispatch
            .install_root(
                dispatch.prepare_root(&root, RootPermission::Read).unwrap(),
                CancellationToken::new(),
            )
            .unwrap();
        let current = policy.for_run_dispatch(run);
        let mut command = current.process_command("sh", workspace.path()).unwrap();
        command.arg("-c").arg(format!(
            "cat '{}/data'; test ! -e '{}/data'; test ! -w '{}'",
            root.display(),
            sibling.display(),
            root.display()
        ));
        current
            .isolate_process(&mut command, workspace.path())
            .unwrap();
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"visible");
        let dispatch = policy.for_run_dispatch(run);
        dispatch
            .install_root(
                dispatch.prepare_root(&root, RootPermission::Write).unwrap(),
                CancellationToken::new(),
            )
            .unwrap();
        let current = policy.for_run_dispatch(run);
        let mut command = current.process_command("sh", workspace.path()).unwrap();
        command
            .arg("-c")
            .arg(format!("printf changed > '{}/data'", root.display()));
        current
            .isolate_process(&mut command, workspace.path())
            .unwrap();
        assert!(command.status().unwrap().success());
        assert_eq!(
            std::fs::read_to_string(root.join("data")).unwrap(),
            "changed"
        );
    }
    #[test]
    fn root_request_must_survive_host_ceiling_as_exact_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let base = crate::policy_profile::Rules {
            access: AccessMode::Unrestricted,
            unattended: crate::config::UnattendedApprovalMode::Deny,
            read_roots: vec![workspace.path().to_str().unwrap().into()],
            write_roots: vec![workspace.path().to_str().unwrap().into()],
            legacy_deny_commands: vec![],
            inherit_env: vec![],
            github_enabled: false,
        };
        let ceiling = crate::policy_profile::CeilingDocument {
            schema: 1,
            rules: base.clone(),
        };
        let effective =
            crate::policy_profile::resolve_loaded(workspace.path(), &base, &[], Some(&ceiling))
                .unwrap();
        let mut policy = Policy::new(&crate::Config::default(), workspace.path().into()).unwrap();
        policy.snapshot.effective = effective.clone();
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(external.path()).unwrap();
        let candidate = RootCandidate {
            path: external.path().into(),
            permission: RootPermission::Read,
            identity: (m.dev(), m.ino()),
        };
        assert!(policy.check_root_ceiling(&[candidate], &effective).is_err());
    }
}
