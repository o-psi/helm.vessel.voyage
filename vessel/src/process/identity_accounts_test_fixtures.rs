//! Typed metadata only: these fixtures are not OS identities or administrator authority.
//! Native dependencies remain: a protected system anchor, real configured users,
//! UID/group/capability drop, Root peer callbacks, namespace ownership transfer,
//! revocation during helper work, actual process admission and observed cleanup.
//! None can be replaced by an accepted metadata predicate in these source cases.
use std::{num::NonZeroU64, path::PathBuf};
use uuid::Uuid;
use voyage_protocol::{
    accounts::*, execution_identity::*, execution_review_control::*, identity_helper::*, process::*,
};

pub(in crate::process) fn identity(authority: AuthorityClass) -> ConfiguredExecutionIdentity {
    let uid = if authority == AuthorityClass::Administrator {
        0
    } else {
        1000
    };
    ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Metadata fixture".into(),
        user_name: "fixture-not-a-host-account".into(),
        uid,
        gid: uid,
        supplementary_groups: vec![10, 20],
        home: PathBuf::from("/fixture/home"),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority,
        enabled: true,
    }
}
pub(in crate::process) fn registration() -> ProcessRegistration {
    ProcessRegistration {
        executable: Some("/fixture/voyage".into()),
        protocol: PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: Some("/fixture/config.json".into()),
        token: "private-runtime-sentinel-not-a-credential".into(),
        peer_uids: Some(ProcessPeerUids {
            supervisor: 0,
            runtime: 1000,
        }),
        workspace: "/fixture/work".into(),
        state: ProcessState::Stopped,
        name: Some("Metadata fixture".into()),
    }
}
pub(in crate::process) fn connection() -> ConnectionGrant {
    ConnectionGrant {
        full_access: true,
        schema_version: 1,
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        vessel_id: Uuid::new_v4(),
        revision: 3,
        rights: ProcessRight::all(),
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: u64::MAX,
        revoked: false,
        token_hash: "unused-fixture-hash".into(),
        workspaces: vec![],
    }
}
pub(in crate::process) fn account() -> AccountBinding {
    AccountBinding {
        account_id: Uuid::new_v4(),
        connection_id: Uuid::new_v4(),
        identity_generation: 1,
        connection_revision: 1,
        transport: Transport::ChatgptOauth,
    }
}
pub(in crate::process) fn config_facts(
    identity: &ConfiguredExecutionIdentity,
) -> IdentityConfigFacts {
    IdentityConfigFacts {
        account: account(),
        capability_revision: 3,
        policy_digest: "a".repeat(64),
        config_digest: "b".repeat(64),
        account_root_digest: "c".repeat(64),
        uid: identity.uid,
        gid: identity.gid,
        supplementary_groups: identity.supplementary_groups.clone(),
    }
}
pub(in crate::process) fn facts(
    identity: &ConfiguredExecutionIdentity,
    registration: &ProcessRegistration,
    grant: &ConnectionGrant,
) -> ReviewFacts {
    ReviewFacts {
        vessel_id: grant.vessel_id,
        session_id: registration.session_id,
        run_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        requester_id: grant.principal_id,
        connection_id: grant.grant_id,
        connection_revision: NonZeroU64::new(grant.revision).unwrap(),
        administrative_owner_id: grant.principal_id,
        authority_revision: NonZeroU64::new(2).unwrap(),
        expected_session_revision: 17,
        change: ExecutionChange::Transition,
        previous_incarnation: Some(registration.incarnation),
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        account: account(),
        account_capability_revision: 3,
        workspace: registration.workspace.clone(),
        host_identity_digest: "d".repeat(64),
        policy_digest: "e".repeat(64),
        release_digest: "f".repeat(64),
        pending_work_digest: "a".repeat(64),
    }
}
pub(in crate::process) fn saved(
    facts: ReviewFacts,
    outcome: ExecutionOutcome,
) -> SavedExecutionReview {
    let mut review = ExecutionReview {
        schema: EXECUTION_SCHEMA,
        review_id: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        created_at_ms: 1000,
        expires_at_ms: 2000,
        facts,
        digest: String::new(),
    };
    review.digest = review.calculated_digest().unwrap();
    let receipt = ExecutionReceipt {
        schema: EXECUTION_SCHEMA,
        vessel_id: review.facts.vessel_id,
        session_id: review.facts.session_id,
        run_id: review.facts.run_id,
        incarnation: review.facts.incarnation,
        command_id: review.command_id,
        review_id: review.review_id,
        review_digest: review.digest.clone(),
        outcome,
    };
    SavedExecutionReview {
        review,
        receipt,
        administrator_grant_id: Some(Uuid::new_v4()),
    }
}
pub(in crate::process) fn observed(
    identity: &ConfiguredExecutionIdentity,
    incarnation: Uuid,
) -> ObservedExecution {
    ObservedExecution {
        identity: identity.identity.clone(),
        incarnation,
        uid: identity.uid,
        gid: identity.gid,
        supplementary_groups: identity.supplementary_groups.clone(),
        effective_capabilities: 0,
        permitted_capabilities: 0,
        inheritable_capabilities: 0,
        ambient_capabilities: 0,
        no_new_privileges: true,
        user_namespace: "fixture-user-namespace".into(),
        process_start_ticks: 7,
        release_digest: "f".repeat(64),
    }
}
