//! Presentation-only setup readiness. Producers must supply observed facts; connection
//! alone is never execution readiness. No installer, credentials or execution owner lives here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Installation,
    Services,
    ExecutionIdentity,
    Gateway,
    BrowserTransport,
    Pairing,
    Workspace,
    AiAccount,
    OrdinaryTask,
    AdministratorTask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Unknown,
    Pending,
    Verified,
    Blocked(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupTarget {
    pub host: String,
    pub deployment: String,
    pub owner: String,
    pub workspace: String,
    pub administrator_enabled: bool,
}

/// In-memory view of a single setup transaction, not an authority or durable receipt.
/// On reconnect a caller must reload facts from their canonical producers.
#[derive(Clone, Debug)]
pub struct HostSetup {
    target: SetupTarget,
    observations: Vec<(Check, Observation)>,
}

const ORDINARY: [Check; 9] = [
    Check::Installation,
    Check::Services,
    Check::ExecutionIdentity,
    Check::Gateway,
    Check::BrowserTransport,
    Check::Pairing,
    Check::Workspace,
    Check::AiAccount,
    Check::OrdinaryTask,
];

impl HostSetup {
    pub fn new(target: SetupTarget) -> Result<Self, &'static str> {
        if [
            &target.host,
            &target.deployment,
            &target.owner,
            &target.workspace,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err("Choose a computer, Helm deployment, owner and workspace first.");
        }
        Ok(Self {
            target,
            observations: Vec::new(),
        })
    }

    pub fn target(&self) -> &SetupTarget {
        &self.target
    }

    /// Reject stale or cross-account observations without changing the current view.
    /// A target match is correlation only; callers still authenticate the producer.
    pub fn observe(
        &mut self,
        target: &SetupTarget,
        check: Check,
        observation: Observation,
    ) -> Result<(), &'static str> {
        if target != &self.target {
            return Err("Setup result belongs to another target.");
        }
        if check == Check::AdministratorTask && !self.target.administrator_enabled {
            return Err("Administrator work has not been enabled.");
        }
        // Losing a prerequisite invalidates dependent results. Never carry an old
        // successful task through expired pairing or a failed identity/service check.
        if observation != Observation::Verified {
            let order: Vec<_> = self.required_checks().collect();
            if let Some(index) = order.iter().position(|candidate| *candidate == check) {
                self.observations
                    .retain(|entry| !order[index + 1..].contains(&entry.0));
            }
        }
        if let Some(entry) = self.observations.iter_mut().find(|entry| entry.0 == check) {
            entry.1 = observation;
        } else {
            self.observations.push((check, observation));
        }
        Ok(())
    }

    pub fn observation(&self, check: Check) -> &Observation {
        self.observations
            .iter()
            .find(|entry| entry.0 == check)
            .map(|entry| &entry.1)
            .unwrap_or(&Observation::Unknown)
    }

    pub fn required_checks(&self) -> impl Iterator<Item = Check> + '_ {
        ORDINARY.into_iter().chain(
            self.target
                .administrator_enabled
                .then_some(Check::AdministratorTask),
        )
    }

    pub fn ready(&self) -> bool {
        self.required_checks()
            .all(|check| self.observation(check) == &Observation::Verified)
    }

    pub fn next_check(&self) -> Option<Check> {
        self.required_checks()
            .find(|check| self.observation(*check) != &Observation::Verified)
    }
}

impl Check {
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Installation => {
                "Verify the supported host installation before configuring services."
            }
            Self::Services => {
                "Check that the installed services start and survive reconnect or reboot."
            }
            Self::ExecutionIdentity => {
                "Verify the permissions the agent will use for ordinary work."
            }
            Self::Gateway => "Check the local gateway and your external HTTPS routing separately.",
            Self::BrowserTransport => {
                "Verify authenticated HTTPS and WSS from the selected Helm browser."
            }
            Self::Pairing => {
                "Approve this computer for the selected Helm account; expired approval needs renewal."
            }
            Self::Workspace => {
                "Choose a workspace and verify access using the intended execution identity."
            }
            Self::AiAccount => {
                "Connect an AI account privately on the execution computer; do not paste credentials into chat."
            }
            Self::OrdinaryTask => {
                "Authorize a small ordinary task and inspect its result; model usage requires approval."
            }
            Self::AdministratorTask => {
                "Separately authorize and verify administrator work; ordinary task success is not proof."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> SetupTarget {
        SetupTarget {
            host: "host".into(),
            deployment: "https://helm.example".into(),
            owner: "owner".into(),
            workspace: "/work".into(),
            administrator_enabled: false,
        }
    }
    #[test]
    fn connectivity_is_not_readiness() {
        let target = target();
        let mut setup = HostSetup::new(target.clone()).unwrap();
        setup
            .observe(&target, Check::BrowserTransport, Observation::Verified)
            .unwrap();
        assert!(!setup.ready());
        assert_eq!(setup.next_check(), Some(Check::Installation));
    }
    #[test]
    fn changed_target_cannot_reuse_results() {
        let target = target();
        let mut setup = HostSetup::new(target.clone()).unwrap();
        for check in setup.required_checks().collect::<Vec<_>>() {
            setup
                .observe(&target, check, Observation::Verified)
                .unwrap();
        }
        assert!(setup.ready());
        for field in 0..5 {
            let mut other = target.clone();
            match field {
                0 => other.host.push('x'),
                1 => other.deployment.push('x'),
                2 => other.owner.push('x'),
                3 => other.workspace.push('x'),
                _ => other.administrator_enabled = true,
            }
            assert!(
                setup
                    .observe(
                        &other,
                        Check::Services,
                        Observation::Blocked("failed".into())
                    )
                    .is_err()
            );
            assert!(setup.ready());
        }
        setup
            .observe(&target, Check::Services, Observation::Pending)
            .unwrap();
        assert!(!setup.ready());
        assert_eq!(
            setup.observation(Check::OrdinaryTask),
            &Observation::Unknown
        );
        setup
            .observe(&target, Check::Services, Observation::Verified)
            .unwrap();
        assert!(!setup.ready());
    }
    #[test]
    fn administrator_requires_separate_evidence() {
        let mut target = target();
        let mut ordinary = HostSetup::new(target.clone()).unwrap();
        assert!(
            ordinary
                .observe(&target, Check::AdministratorTask, Observation::Verified)
                .is_err()
        );
        target.administrator_enabled = true;
        let mut setup = HostSetup::new(target.clone()).unwrap();
        for check in ORDINARY {
            setup
                .observe(&target, check, Observation::Verified)
                .unwrap();
        }
        assert_eq!(setup.next_check(), Some(Check::AdministratorTask));
        assert!(!setup.ready());
        setup
            .observe(&target, Check::AdministratorTask, Observation::Verified)
            .unwrap();
        assert!(setup.ready());
    }
    #[test]
    fn incomplete_targets_refuse() {
        let mut target = target();
        target.owner = " ".into();
        assert!(HostSetup::new(target).is_err());
    }
}
