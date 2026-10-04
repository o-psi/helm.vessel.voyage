//! Native ordinary-process adapters; backend remains unqualified.
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub mod macos_session;
#[cfg(unix)]
pub mod unix_registry;
#[cfg(unix)]
pub mod unix_transport;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", deny_unknown_fields)]
pub enum Owner {
    MacOs { uid: u32, home: PathBuf },
    Windows { sid: String, profile: PathBuf },
}
impl Owner {
    // Structural account identity only; native token/group authority must be
    // attested separately before launch. A SID prefix does not prove non-admin.
    pub fn ordinary(&self) -> Result<()> {
        match self {
            Self::MacOs { uid, home } => ensure!(
                *uid != 0 && home.is_absolute(),
                "Native ordinary owner must not be root"
            ),
            Self::Windows { sid, profile } => {
                ensure!(
                    sid.starts_with("S-1-5-21-")
                        && sid.len() <= 184
                        && sid
                            .bytes()
                            .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-'),
                    "Exact ordinary account SID required"
                );
                ensure!(
                    !profile.as_os_str().is_empty(),
                    "Exact native profile required"
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub launch_command_id: Uuid,
    pub owner: Owner,
    pub binary_sha256: String,
    pub workspace: PathBuf,
    /// PID is never sufficient: platform creation-time/process handle proof needed.
    pub pid: Option<u32>,
    pub process_identity: Option<String>,
    pub phase: Phase,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Intent,
    SpawnUncertain,
    Running,
    Exited,
    CleanupPending,
    Cleaned,
}
impl Registration {
    pub fn validate(&self) -> Result<()> {
        self.owner.ordinary()?;
        ensure!(
            self.schema_version == 1
                && !self.session_id.is_nil()
                && !self.incarnation.is_nil()
                && !self.launch_command_id.is_nil(),
            "Invalid native registration identity"
        );
        ensure!(
            self.binary_sha256.len() == 64
                && self
                    .binary_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Exact native executable digest required"
        );
        ensure!(!self.workspace.as_os_str().is_empty(), "Workspace required");
        if self.phase == Phase::Running {
            ensure!(
                self.pid.is_some_and(|pid| pid != 0)
                    && self
                        .process_identity
                        .as_ref()
                        .is_some_and(|v| !v.is_empty()),
                "Running native process lacks creation identity"
            );
        }
        Ok(())
    }
    pub fn observe_running(&mut self, pid: u32, identity: String) -> Result<()> {
        ensure!(
            self.phase == Phase::Intent,
            "Uncertain native launch cannot be replayed"
        );
        let mut candidate = self.clone();
        candidate.pid = Some(pid);
        candidate.process_identity = Some(identity);
        candidate.phase = Phase::Running;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn observe_exit(&mut self, expected_identity: &str) -> Result<()> {
        ensure!(
            self.phase == Phase::Running
                && self.process_identity.as_deref() == Some(expected_identity),
            "Native process identity changed or exit unobserved"
        );
        self.phase = Phase::Exited;
        Ok(())
    }
    pub fn cleanup_observed(&mut self, resource_obligations_empty: bool) -> Result<()> {
        ensure!(
            matches!(self.phase, Phase::Exited | Phase::CleanupPending)
                && resource_obligations_empty,
            "Native cleanup unresolved"
        );
        self.phase = Phase::Cleaned;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_administrator_is_not_inferred_from_installation() {
        assert!(
            Owner::MacOs {
                uid: 0,
                home: PathBuf::from("/root")
            }
            .ordinary()
            .is_err()
        );
        assert!(
            Owner::Windows {
                sid: "S-1-5-18".into(),
                profile: PathBuf::from("C:\\system")
            }
            .ordinary()
            .is_err()
        );
    }
    #[test]
    fn uncertain_launch_and_unobserved_cleanup_refuse() {
        let mut r = Registration {
            schema_version: 1,
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            launch_command_id: Uuid::new_v4(),
            owner: Owner::MacOs {
                uid: 501,
                home: PathBuf::from("/Users/test"),
            },
            binary_sha256: "a".repeat(64),
            workspace: PathBuf::from("/work"),
            pid: None,
            process_identity: None,
            phase: Phase::Intent,
        };
        r.observe_running(42, "creation-identity".into()).unwrap();
        assert!(r.observe_running(42, "replay".into()).is_err());
        assert!(r.observe_exit("reused-pid").is_err());
        r.observe_exit("creation-identity").unwrap();
        assert!(r.cleanup_observed(false).is_err());
        r.cleanup_observed(true).unwrap();
    }
}
