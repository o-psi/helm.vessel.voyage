//! Review-bound setup presentation transaction. This does not grant authority:
//! authenticated installer/pairing adapters must validate their own receipts.
use super::host_setup::SetupTarget;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewedPlan {
    pub transaction: Uuid,
    pub target: SetupTarget,
    pub plan_digest: String,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReceipt {
    pub command: Uuid,
    pub transaction: Uuid,
    pub plan_digest: String,
    pub target: SetupTarget,
    pub observed_complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskProof {
    pub session: Uuid,
    pub run: Uuid,
    pub command: Uuid,
    pub target: SetupTarget,
    pub result_digest: String,
    pub inspected: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Review,
    Pending(Uuid),
    Uncertain(Uuid),
    Complete(Uuid),
}
pub struct SetupTransaction {
    pub plan: ReviewedPlan,
    pub state: State,
}
impl SetupTransaction {
    pub fn new(plan: ReviewedPlan) -> Result<Self, &'static str> {
        if plan.transaction.is_nil() || !digest(&plan.plan_digest) {
            return Err("Invalid reviewed plan identity");
        }
        Ok(Self {
            plan,
            state: State::Review,
        })
    }
    /// Record an exact intent before invoking the authenticated effect adapter.
    pub fn begin(
        &mut self,
        command: Uuid,
        now_ms: u64,
        reviewed: &ReviewedPlan,
    ) -> Result<(), &'static str> {
        if self.state != State::Review
            || reviewed != &self.plan
            || now_ms >= self.plan.expires_at_ms
            || command.is_nil()
        {
            return Err("Review expired, changed or an earlier command remains unresolved");
        }
        self.state = State::Pending(command);
        Ok(())
    }
    pub fn uncertain(&mut self) {
        if let State::Pending(command) = self.state {
            self.state = State::Uncertain(command);
        }
    }
    /// Resolve only the retained exact command. Unknown effects never become replayable.
    pub fn resolve(&mut self, receipt: &CommandReceipt) -> Result<(), &'static str> {
        let command = match self.state {
            State::Pending(c) | State::Uncertain(c) => c,
            _ => return Err("No pending command"),
        };
        if receipt.command != command
            || receipt.transaction != self.plan.transaction
            || receipt.target != self.plan.target
            || receipt.plan_digest != self.plan.plan_digest
            || !receipt.observed_complete
        {
            return Err("Receipt does not prove completion of the reviewed operation");
        }
        self.state = State::Complete(command);
        Ok(())
    }
    pub fn task_proven(&self, proof: &TaskProof) -> bool {
        matches!(self.state, State::Complete(_))
            && proof.target == self.plan.target
            && !proof.session.is_nil()
            && !proof.run.is_nil()
            && !proof.command.is_nil()
            && proof.inspected
            && digest(&proof.result_digest)
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uncertain_command_retains_exact_identity_and_requires_inspection() {
        let target = SetupTarget {
            host: "host".into(),
            deployment: "https://helm.example".into(),
            owner: "owner".into(),
            workspace: "/work".into(),
            administrator_enabled: false,
        };
        let plan = ReviewedPlan {
            transaction: Uuid::from_u128(1),
            target: target.clone(),
            plan_digest: "a".repeat(64),
            expires_at_ms: 10,
        };
        let mut setup = SetupTransaction::new(plan.clone()).unwrap();
        let command = Uuid::from_u128(2);
        setup.begin(command, 1, &plan).unwrap();
        setup.uncertain();
        assert!(setup.begin(Uuid::from_u128(3), 2, &plan).is_err());
        let mut receipt = CommandReceipt {
            command: Uuid::from_u128(3),
            transaction: plan.transaction,
            plan_digest: plan.plan_digest.clone(),
            target: target.clone(),
            observed_complete: true,
        };
        assert!(setup.resolve(&receipt).is_err());
        receipt.command = command;
        setup.resolve(&receipt).unwrap();
        let mut proof = TaskProof {
            session: Uuid::from_u128(4),
            run: Uuid::from_u128(5),
            command: Uuid::from_u128(6),
            target,
            result_digest: "b".repeat(64),
            inspected: false,
        };
        assert!(!setup.task_proven(&proof));
        proof.inspected = true;
        assert!(setup.task_proven(&proof));
        proof.target.owner = "other".into();
        assert!(!setup.task_proven(&proof));
    }
}
