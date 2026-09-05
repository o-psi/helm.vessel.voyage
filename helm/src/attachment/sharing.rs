//! In-memory single-owner attachment policy; not authentication or persistence.
//! Callers must supply trusted installation/authentication scopes, never request claims.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Disclosure {
    None,
    Metadata,
    Live,
    Transcript,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    ViewMetadata,
    ViewLive,
    ViewHistory,
    Submit,
    Cancel,
    Rename,
    ChangeModel,
    Branch,
    Archive,
    Delete,
    ChangeSharing,
    ApproveWrite,
    ApproveCommand,
}

/// An epoch change revokes all policies and authentication from earlier epochs.
/// No Deserialize: construction must validate identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    machine: Uuid,
    owner: Uuid,
    epoch: u64,
}

impl Scope {
    pub fn new(machine: Uuid, owner: Uuid, epoch: u64) -> Option<Self> {
        (!machine.is_nil() && !owner.is_nil() && epoch > 0 && epoch <= i64::MAX as u64).then_some(
            Self {
                machine,
                owner,
                epoch,
            },
        )
    }
}

#[derive(Clone, Debug)]
pub struct SessionSharing {
    session_id: Uuid,
    scope: Scope,
    disclosure: Disclosure,
    archived: bool,
    approve_write: bool,
    approve_command: bool,
}

/// Deliberately contains no owner identity, credentials, provider structs, or transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Projection {
    session_id: Uuid,
    disclosure: Disclosure,
    archived: bool,
}

impl SessionSharing {
    /// Trusted local policy creation, not an attachment-request deserializer.
    pub fn new(
        session_id: Uuid,
        scope: Scope,
        disclosure: Disclosure,
        archived: bool,
    ) -> Option<Self> {
        (!session_id.is_nil()).then_some(Self {
            session_id,
            scope,
            disclosure,
            archived,
            approve_write: false,
            approve_command: false,
        })
    }

    /// Explicit trusted-local opt-ins; neither disclosure nor branching grants approvals.
    /// This only permits requesting approval; it does not execute or approve an operation.
    pub fn with_approvals(mut self, write: bool, command: bool) -> Self {
        self.approve_write = write;
        self.approve_command = command;
        self
    }

    /// Bind authorization to the requested ID as well as both trusted scopes.
    /// Archived sessions cannot run or branch, but may be unarchived, deleted or
    /// unshared by the owner through separately confirmed lifecycle operations.
    pub fn authorize(
        &self,
        session_id: Uuid,
        current: &Scope,
        authenticated: &Scope,
        action: Action,
    ) -> bool {
        if session_id != self.session_id || self.scope != *current || current != authenticated {
            return false;
        }
        if self.disclosure == Disclosure::None {
            return false;
        }
        if action == Action::ViewMetadata {
            return true;
        }
        if self.disclosure == Disclosure::Metadata {
            return false;
        }
        if self.archived {
            return matches!(
                action,
                Action::Archive | Action::Delete | Action::ChangeSharing
            ) || (action == Action::ViewHistory
                && self.disclosure == Disclosure::Transcript);
        }
        match action {
            Action::ViewHistory | Action::Branch => self.disclosure == Disclosure::Transcript,
            Action::ApproveWrite => self.approve_write,
            Action::ApproveCommand => self.approve_command,
            _ => true,
        }
    }

    pub fn project(
        &self,
        session_id: Uuid,
        current: &Scope,
        authenticated: &Scope,
    ) -> Option<Projection> {
        self.authorize(session_id, current, authenticated, Action::ViewMetadata)
            .then_some(Projection {
                session_id: self.session_id,
                disclosure: self.disclosure,
                archived: self.archived,
            })
    }

    /// Attachment branching requires history access. Child sharing can only narrow,
    /// never widen; use this path rather than `new` for attachment-originated branches.
    /// No ancestry is projected, and approval opt-ins are not inherited.
    pub fn branch(
        &self,
        current: &Scope,
        authenticated: &Scope,
        child_id: Uuid,
        disclosure: Disclosure,
    ) -> Option<Self> {
        if child_id == self.session_id
            || disclosure > self.disclosure
            || !self.authorize(self.session_id, current, authenticated, Action::Branch)
        {
            return None;
        }
        Self::new(child_id, self.scope, disclosure, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [Action; 13] = [
        Action::ViewMetadata,
        Action::ViewLive,
        Action::ViewHistory,
        Action::Submit,
        Action::Cancel,
        Action::Rename,
        Action::ChangeModel,
        Action::Branch,
        Action::Archive,
        Action::Delete,
        Action::ChangeSharing,
        Action::ApproveWrite,
        Action::ApproveCommand,
    ];
    const DISCLOSURES: [Disclosure; 4] = [
        Disclosure::None,
        Disclosure::Metadata,
        Disclosure::Live,
        Disclosure::Transcript,
    ];
    fn scope() -> Scope {
        Scope::new(Uuid::from_u128(1), Uuid::from_u128(2), 3).unwrap()
    }
    fn policy(disclosure: Disclosure, archived: bool) -> SessionSharing {
        SessionSharing::new(Uuid::from_u128(4), scope(), disclosure, archived).unwrap()
    }
    fn allowed(p: &SessionSharing, action: Action) -> bool {
        p.authorize(p.session_id, &scope(), &scope(), action)
    }

    #[test]
    fn full_matrix_including_archival_and_independent_approval_opt_ins() {
        // Explicit non-approval matrix, in ACTIONS order; approval columns default false.
        let matrix = [
            [false; 13],
            [
                true, false, false, false, false, false, false, false, false, false, false, false,
                false,
            ],
            [
                true, true, false, true, true, true, true, false, true, true, true, false, false,
            ],
            [
                true, true, true, true, true, true, true, true, true, true, true, false, false,
            ],
        ];
        for (row, disclosure) in DISCLOSURES.into_iter().enumerate() {
            for archived in [false, true] {
                for write in [false, true] {
                    for command in [false, true] {
                        let p = policy(disclosure, archived).with_approvals(write, command);
                        for (column, action) in ACTIONS.into_iter().enumerate() {
                            let mut expected = matrix[row][column];
                            if row >= 2 {
                                if action == Action::ApproveWrite {
                                    expected = write;
                                }
                                if action == Action::ApproveCommand {
                                    expected = command;
                                }
                            }
                            if archived {
                                expected &= matches!(
                                    action,
                                    Action::ViewMetadata
                                        | Action::ViewHistory
                                        | Action::Archive
                                        | Action::Delete
                                        | Action::ChangeSharing
                                );
                            }
                            assert_eq!(
                                allowed(&p, action),
                                expected,
                                "{disclosure:?} {action:?} archived={archived} write={write} command={command}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn identity_epoch_revocation_and_guessed_ids() {
        let p = policy(Disclosure::Transcript, false).with_approvals(true, true);
        let scopes = [
            Scope::new(Uuid::from_u128(9), scope().owner, 3).unwrap(),
            Scope::new(scope().machine, Uuid::from_u128(9), 3).unwrap(),
            Scope::new(scope().machine, scope().owner, 4).unwrap(),
        ];
        for action in ACTIONS {
            for other in scopes {
                assert!(!p.authorize(p.session_id, &scope(), &other, action));
                assert!(!p.authorize(p.session_id, &other, &scope(), action));
                assert!(!p.authorize(p.session_id, &other, &other, action));
            }
            assert!(!p.authorize(Uuid::from_u128(99), &scope(), &scope(), action));
            assert!(!allowed(&policy(Disclosure::None, false), action));
        }
        assert!(Scope::new(scope().machine, scope().owner, 0).is_none());
        assert!(Scope::new(scope().machine, scope().owner, u64::MAX).is_none());
        assert!(Scope::new(Uuid::nil(), scope().owner, 0).is_none());
        assert!(Scope::new(scope().machine, Uuid::nil(), 0).is_none());
        assert!(SessionSharing::new(Uuid::nil(), scope(), Disclosure::Live, false).is_none());
    }

    #[test]
    fn private_ancestry_and_branch_approval_reset() {
        for parent in DISCLOSURES {
            let p = policy(parent, false).with_approvals(true, true);
            for child in DISCLOSURES {
                let branch = p.branch(&scope(), &scope(), Uuid::from_u128(5), child);
                assert_eq!(branch.is_some(), parent == Disclosure::Transcript);
                if let Some(branch) = branch {
                    assert!(branch.disclosure <= p.disclosure);
                    assert!(!allowed(&branch, Action::ApproveWrite));
                    assert!(!allowed(&branch, Action::ApproveCommand));
                    if child == Disclosure::None {
                        assert!(
                            branch
                                .project(branch.session_id, &scope(), &scope())
                                .is_none()
                        );
                        assert!(
                            branch
                                .branch(&scope(), &scope(), Uuid::from_u128(6), parent)
                                .is_none()
                        );
                    }
                }
            }
        }
        let p = policy(Disclosure::Transcript, false);
        assert!(
            p.branch(&scope(), &scope(), p.session_id, Disclosure::None)
                .is_none()
        );
        assert!(
            p.branch(&scope(), &scope(), Uuid::nil(), Disclosure::None)
                .is_none()
        );
        let json =
            serde_json::to_value(p.project(p.session_id, &scope(), &scope()).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"session_id": p.session_id, "disclosure": "Transcript", "archived": false})
        );
    }
}
