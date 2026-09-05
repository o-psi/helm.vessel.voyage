use super::*;
use std::sync::{Arc, Barrier};
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
const CAPS: [Capability; 16] = [
    Capability::ViewMetadata,
    Capability::ViewLive,
    Capability::ViewHistory,
    Capability::CreateSession,
    Capability::SubmitTurn,
    Capability::CancelOwn,
    Capability::CancelAny,
    Capability::RenameSession,
    Capability::ChangeModel,
    Capability::BranchSession,
    Capability::ArchiveSession,
    Capability::DeleteSession,
    Capability::ChangeSharing,
    Capability::ApproveWrite,
    Capability::ApproveCommand,
    Capability::AdministerEnrollment,
];
const DISCLOSURES: [Disclosure; 4] = [
    Disclosure::None,
    Disclosure::Metadata,
    Disclosure::Live,
    Disclosure::Transcript,
];
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn scope() -> Scope {
    Scope::new(id(1), id(2), 3).unwrap()
}
fn grant(caps: &[Capability]) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal::new(scope(), id(7), caps).unwrap()
}
fn registry(disclosure: Disclosure, archived: bool, write: bool, command: bool) -> SharingRegistry {
    let owner = SharingRegistry::new(scope(), &CAPS);
    owner
        .register_local(
            id(4),
            SharingSettings::new(disclosure, archived).with_approvals(write, command),
        )
        .unwrap();
    owner
}
fn allowed(owner: &SharingRegistry, action: Action) -> bool {
    owner.authorize(
        id(4),
        &grant(&CAPS),
        action,
        Some(&RunAttribution::new(id(4), id(8), id(7)).unwrap()),
    )
}
#[test]
fn full_matrix_including_archival_and_independent_approval_opt_ins() {
    // Preserve the prior complete disclosure/archive/approval matrix, now through
    // the owner with explicit full grants and a matching trusted run target.
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
                    let owner = registry(disclosure, archived, write, command);
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
                            allowed(&owner, action),
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
    let owner = registry(Disclosure::Transcript, false, true, true);
    for other in [
        Scope::new(id(9), scope().owner, 3).unwrap(),
        Scope::new(scope().machine, id(9), 3).unwrap(),
        Scope::new(scope().machine, scope().owner, 4).unwrap(),
    ] {
        let principal = AuthenticatedPrincipal::new(other, id(7), &CAPS).unwrap();
        for action in ACTIONS {
            assert!(!owner.authorize(id(4), &principal, action, None));
        }
        assert!(owner.project(id(4), &principal).is_none());
    }
    for action in ACTIONS {
        assert!(!owner.authorize(id(99), &grant(&CAPS), action, None));
        assert!(!allowed(
            &registry(Disclosure::None, false, true, true),
            action
        ));
    }
    for epoch in [0, i64::MAX as u64, u64::MAX] {
        assert!(Scope::new(id(1), id(2), epoch).is_none());
    }
    assert!(Scope::new(id(1), id(2), i64::MAX as u64 - 1).is_some());
    assert!(Scope::new(Uuid::nil(), id(2), 3).is_none());
    assert!(Scope::new(id(1), Uuid::nil(), 3).is_none());
    assert!(AuthenticatedPrincipal::new(scope(), Uuid::nil(), &CAPS).is_none());
    assert!(
        owner
            .register_local(Uuid::nil(), SharingSettings::new(Disclosure::Live, false))
            .is_err()
    );
}
#[test]
fn private_ancestry_and_branch_approval_reset() {
    for parent in DISCLOSURES {
        let owner = registry(parent, false, true, true);
        for (index, child) in DISCLOSURES.into_iter().enumerate() {
            let child_id = id(20 + index as u128);
            let branch = owner.branch(id(4), 1, &grant(&CAPS), child_id, child);
            assert_eq!(branch.is_ok(), parent == Disclosure::Transcript);
            if let Ok(branch) = branch {
                assert!(branch.settings.disclosure <= parent);
                assert!(!branch.settings.approve_write && !branch.settings.approve_command);
                assert!(!owner.authorize(child_id, &grant(&CAPS), Action::ApproveWrite, None));
                assert!(!owner.authorize(child_id, &grant(&CAPS), Action::ApproveCommand, None));
                if child == Disclosure::None {
                    assert!(owner.project(child_id, &grant(&CAPS)).is_none());
                    assert!(
                        owner
                            .branch(child_id, 1, &grant(&CAPS), id(30), parent)
                            .is_err()
                    );
                }
            }
        }
    }
    let owner = registry(Disclosure::Transcript, false, false, false);
    assert!(
        owner
            .branch(id(4), 1, &grant(&CAPS), id(4), Disclosure::None)
            .is_err()
    );
    assert!(
        owner
            .branch(id(4), 1, &grant(&CAPS), Uuid::nil(), Disclosure::None)
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(owner.project(id(4), &grant(&CAPS)).unwrap()).unwrap(),
        serde_json::json!({"session_id":id(4),"disclosure":"Transcript","archived":false})
    );
}
#[test]
fn live_observation_does_not_grant_mutation_capabilities() {
    let owner = registry(Disclosure::Live, false, true, true);
    let observer = grant(&[Capability::ViewMetadata, Capability::ViewLive]);
    assert!(owner.project(id(4), &observer).is_some());
    assert!(owner.authorize(id(4), &observer, Action::ViewLive, None));
    for action in ACTIONS {
        if !matches!(action, Action::ViewMetadata | Action::ViewLive) {
            assert!(!owner.authorize(id(4), &observer, action, None));
        }
    }
}
#[test]
fn current_unshare_invalidates_previously_observed_policy() {
    let owner = SharingRegistry::new(scope(), &CAPS);
    let old = owner
        .register_local(id(4), SharingSettings::new(Disclosure::Transcript, false))
        .unwrap();
    let old_copy = old.clone();
    let observer = grant(&CAPS);
    assert!(owner.authorize(id(4), &observer, Action::ViewHistory, None));
    assert!(owner.project(id(4), &observer).is_some());
    owner
        .update_local(
            id(4),
            old.revision,
            SharingSettings::new(Disclosure::None, false),
        )
        .unwrap();
    assert_eq!(old_copy.settings.disclosure, Disclosure::Transcript); // Still data, never authority.
    assert!(!owner.authorize(id(4), &observer, Action::ViewHistory, None));
    assert!(owner.project(id(4), &observer).is_none());
    assert!(
        owner
            .branch(
                id(4),
                old_copy.revision,
                &observer,
                id(5),
                Disclosure::Transcript
            )
            .is_err()
    );
    assert_eq!(
        owner
            .update_local(id(4), old_copy.revision, old_copy.settings)
            .unwrap_err(),
        SharingError::Conflict
    );
}
#[test]
fn installation_and_principal_rights_intersect_for_every_action() {
    for action in ACTIONS {
        let owner = SharingRegistry::new(scope(), &[]);
        owner
            .register_local(
                id(4),
                SharingSettings::new(Disclosure::Transcript, false).with_approvals(true, true),
            )
            .unwrap();
        assert!(!allowed(&owner, action));
        let owner = registry(Disclosure::Transcript, false, true, true);
        assert!(!owner.authorize(id(4), &grant(&[]), action, None));
    }
    let admin = grant(&[Capability::AdministerEnrollment]);
    let owner = registry(Disclosure::Transcript, false, true, true);
    for action in ACTIONS {
        assert!(!owner.authorize(id(4), &admin, action, None));
    }
    assert!(owner.project(id(4), &admin).is_none());
    let owner = SharingRegistry::new(scope(), &[Capability::ViewLive]);
    owner
        .register_local(id(4), SharingSettings::new(Disclosure::Live, false))
        .unwrap();
    assert!(!owner.authorize(
        id(4),
        &grant(&[Capability::SubmitTurn]),
        Action::Submit,
        None
    ));
    assert!(!owner.authorize(
        id(4),
        &grant(&[Capability::SubmitTurn]),
        Action::ViewLive,
        None
    ));
}
#[test]
fn own_and_any_cancellation_use_trusted_session_and_principal_attribution() {
    let owner = registry(Disclosure::Live, false, false, false);
    let own = RunAttribution::new(id(4), id(8), id(7)).unwrap();
    let foreign = RunAttribution::new(id(4), id(9), id(99)).unwrap();
    let other_session = RunAttribution::new(id(6), id(8), id(7)).unwrap();
    let principal = grant(&[Capability::CancelOwn]);
    assert!(owner.authorize(id(4), &principal, Action::Cancel, Some(&own)));
    assert!(!owner.authorize(id(4), &principal, Action::Cancel, Some(&foreign)));
    assert!(!owner.authorize(id(4), &principal, Action::Cancel, Some(&other_session)));
    assert!(!owner.authorize(id(4), &principal, Action::Cancel, None));
    let principal = grant(&[Capability::CancelAny]);
    assert!(owner.authorize(id(4), &principal, Action::Cancel, Some(&foreign)));
    assert!(!owner.authorize(id(4), &principal, Action::Cancel, None));
    assert!(RunAttribution::new(Uuid::nil(), id(8), id(7)).is_none());
    assert!(RunAttribution::new(id(4), Uuid::nil(), id(7)).is_none());
    assert!(RunAttribution::new(id(4), id(8), Uuid::nil()).is_none());
}
#[test]
fn approval_requires_both_capability_and_current_local_opt_in() {
    for (action, cap) in [
        (Action::ApproveWrite, Capability::ApproveWrite),
        (Action::ApproveCommand, Capability::ApproveCommand),
    ] {
        let owner = registry(Disclosure::Live, false, false, false);
        assert!(!owner.authorize(id(4), &grant(&[cap]), action, None));
        owner
            .update_local(
                id(4),
                1,
                SharingSettings::new(Disclosure::Live, false).with_approvals(true, true),
            )
            .unwrap();
        assert!(owner.authorize(id(4), &grant(&[cap]), action, None));
        assert!(!owner.authorize(id(4), &grant(&[Capability::ViewLive]), action, None));
        owner
            .update_local(id(4), 2, SharingSettings::new(Disclosure::Live, false))
            .unwrap();
        assert!(!owner.authorize(id(4), &grant(&[cap]), action, None));
    }
}
#[test]
fn branch_requires_history_capability_current_revision_and_unarchived_source() {
    let owner = registry(Disclosure::Transcript, false, true, true);
    assert!(
        owner
            .branch(
                id(4),
                1,
                &grant(&[Capability::BranchSession]),
                id(5),
                Disclosure::Live
            )
            .is_err()
    );
    assert!(
        owner
            .branch(
                id(4),
                1,
                &grant(&[Capability::ViewHistory]),
                id(5),
                Disclosure::Live
            )
            .is_err()
    );
    owner
        .update_local(
            id(4),
            1,
            SharingSettings::new(Disclosure::Transcript, false),
        )
        .unwrap();
    assert_eq!(
        owner
            .branch(id(4), 1, &grant(&CAPS), id(5), Disclosure::Live)
            .unwrap_err(),
        SharingError::Conflict
    );
    owner
        .update_local(id(4), 2, SharingSettings::new(Disclosure::Transcript, true))
        .unwrap();
    assert!(
        owner
            .branch(id(4), 3, &grant(&CAPS), id(5), Disclosure::Live)
            .is_err()
    );
    assert!(!allowed(&owner, Action::Submit));
    assert!(owner.project(id(5), &grant(&CAPS)).is_none());
}
#[test]
fn epoch_changes_fence_old_grants_and_require_explicit_policy_reconsent() {
    let owner = registry(Disclosure::Transcript, false, true, true);
    let old = grant(&CAPS);
    owner.advance_epoch_local(3, 4, &CAPS).unwrap();
    let new =
        AuthenticatedPrincipal::new(Scope::new(id(1), id(2), 4).unwrap(), id(7), &CAPS).unwrap();
    for principal in [&old, &new] {
        assert!(!owner.authorize(id(4), principal, Action::ViewHistory, None));
    }
    owner
        .update_local(
            id(4),
            1,
            SharingSettings::new(Disclosure::Transcript, false),
        )
        .unwrap();
    assert!(owner.authorize(id(4), &new, Action::ViewHistory, None));
    assert!(!owner.authorize(id(4), &old, Action::ViewHistory, None));
    assert_eq!(
        owner.advance_epoch_local(3, 5, &CAPS).unwrap_err(),
        SharingError::Conflict
    );
    assert_eq!(
        owner.advance_epoch_local(4, 3, &CAPS).unwrap_err(),
        SharingError::Invalid
    );
    assert_eq!(
        owner
            .advance_epoch_local(4, MAX_REVISION, &CAPS)
            .unwrap_err(),
        SharingError::Invalid
    );
    assert!(owner.authorize(id(4), &new, Action::ViewHistory, None));
}
#[test]
fn concurrent_updates_have_one_winner_and_stale_snapshots_never_restore_consent() {
    let owner = Arc::new(registry(Disclosure::Transcript, false, false, false));
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = [Disclosure::None, Disclosure::Metadata]
        .into_iter()
        .map(|mode| {
            let owner = owner.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                owner.update_local(id(4), 1, SharingSettings::new(mode, false))
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(SharingError::Conflict)))
            .count(),
        1
    );
    assert!(!allowed(&owner, Action::ViewHistory));
    assert!(
        owner
            .update_local(
                id(4),
                1,
                SharingSettings::new(Disclosure::Transcript, false)
            )
            .is_err()
    );
}
#[test]
fn poisoned_authority_fails_closed() {
    let owner = Arc::new(registry(Disclosure::Transcript, false, false, false));
    let thread_owner = owner.clone();
    let _ = std::thread::spawn(move || {
        let _guard = thread_owner.state.write().unwrap();
        panic!("fixture poisoned registry");
    })
    .join();
    assert!(!allowed(&owner, Action::ViewHistory));
    assert!(owner.project(id(4), &grant(&CAPS)).is_none());
    assert_eq!(
        owner
            .update_local(id(4), 1, SharingSettings::new(Disclosure::None, false))
            .unwrap_err(),
        SharingError::Unavailable
    );
}
#[test]
fn capacity_conflicts_and_revision_overflow_preserve_current_state() {
    let owner = registry(Disclosure::Transcript, false, false, false);
    assert_eq!(
        owner
            .register_local(id(4), SharingSettings::new(Disclosure::None, false))
            .unwrap_err(),
        SharingError::Conflict
    );
    assert!(allowed(&owner, Action::ViewHistory));
    for n in 0..MAX_POLICIES - 1 {
        owner
            .register_local(
                id(100 + n as u128),
                SharingSettings::new(Disclosure::None, false),
            )
            .unwrap();
    }
    assert_eq!(
        owner
            .register_local(id(10000), SharingSettings::new(Disclosure::Live, false))
            .unwrap_err(),
        SharingError::Capacity
    );
    assert_eq!(
        owner
            .branch(id(4), 1, &grant(&CAPS), id(10000), Disclosure::Live)
            .unwrap_err(),
        SharingError::Capacity
    );
    assert!(allowed(&owner, Action::ViewHistory));
    {
        let mut state = owner.state.write().unwrap();
        state.sessions.get_mut(&id(4)).unwrap().snapshot.revision = MAX_REVISION;
    }
    assert_eq!(
        owner
            .update_local(
                id(4),
                MAX_REVISION,
                SharingSettings::new(Disclosure::None, false)
            )
            .unwrap_err(),
        SharingError::Invalid
    );
    assert!(allowed(&owner, Action::ViewHistory));
}

#[test]
fn each_single_capability_is_narrow_on_both_sides_of_the_intersection() {
    let cases = [
        (Capability::ViewMetadata, Some(Action::ViewMetadata)),
        (Capability::ViewLive, Some(Action::ViewLive)),
        (Capability::ViewHistory, Some(Action::ViewHistory)),
        (Capability::CreateSession, None),
        (Capability::SubmitTurn, Some(Action::Submit)),
        (Capability::CancelOwn, Some(Action::Cancel)),
        (Capability::CancelAny, Some(Action::Cancel)),
        (Capability::RenameSession, Some(Action::Rename)),
        (Capability::ChangeModel, Some(Action::ChangeModel)),
        // Branch requires history permission in addition to branch capability.
        (Capability::BranchSession, None),
        (Capability::ArchiveSession, Some(Action::Archive)),
        (Capability::DeleteSession, Some(Action::Delete)),
        (Capability::ChangeSharing, Some(Action::ChangeSharing)),
        (Capability::ApproveWrite, Some(Action::ApproveWrite)),
        (Capability::ApproveCommand, Some(Action::ApproveCommand)),
        (Capability::AdministerEnrollment, None),
    ];
    let run = RunAttribution::new(id(4), id(8), id(7)).unwrap();
    for (capability, expected) in cases {
        for installation_limited in [true, false] {
            let limited = [capability];
            let owner = SharingRegistry::new(
                scope(),
                if installation_limited {
                    &limited
                } else {
                    &CAPS
                },
            );
            owner
                .register_local(
                    id(4),
                    SharingSettings::new(Disclosure::Transcript, false).with_approvals(true, true),
                )
                .unwrap();
            let principal = grant(if installation_limited {
                &CAPS
            } else {
                &limited
            });
            for action in ACTIONS {
                assert_eq!(
                    owner.authorize(id(4), &principal, action, Some(&run)),
                    expected == Some(action),
                    "{capability:?} {action:?} installation_limited={installation_limited}"
                );
            }
        }
    }
    let owner = SharingRegistry::new(scope(), &[Capability::CancelOwn]);
    owner
        .register_local(id(4), SharingSettings::new(Disclosure::Live, false))
        .unwrap();
    assert!(!owner.authorize(
        id(4),
        &grant(&[Capability::CancelAny]),
        Action::Cancel,
        Some(&run)
    ));
}

#[test]
fn subscription_and_branch_recheck_after_another_thread_revokes() {
    let owner = Arc::new(registry(Disclosure::Transcript, false, true, true));
    let observer = grant(&CAPS);
    assert!(owner.project(id(4), &observer).is_some());
    let changing = owner.clone();
    std::thread::spawn(move || {
        changing.update_local(id(4), 1, SharingSettings::new(Disclosure::None, false))
    })
    .join()
    .unwrap()
    .unwrap();
    // The old identity remains authenticated, but current consent has changed.
    assert!(owner.project(id(4), &observer).is_none());
    for action in ACTIONS {
        assert!(!owner.authorize(id(4), &observer, action, None));
    }
    assert_eq!(
        owner
            .branch(id(4), 1, &observer, id(5), Disclosure::Transcript)
            .unwrap_err(),
        SharingError::Denied
    );
    // Explicit local re-consent creates a new revision, never restores old CAS.
    owner
        .update_local(
            id(4),
            2,
            SharingSettings::new(Disclosure::Transcript, false),
        )
        .unwrap();
    assert!(owner.authorize(id(4), &observer, Action::ViewHistory, None));
    assert_eq!(
        owner
            .branch(id(4), 1, &observer, id(5), Disclosure::Transcript)
            .unwrap_err(),
        SharingError::Conflict
    );
}
