//! A session-scoped credential cannot acquire a new OS/account/admin identity.
//! Ordinary process replacement retains authority only under the same epoch.
use anyhow::{Result, ensure};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::{execution_identity::ExecutionBinding, process::ProcessGrant};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochPin {
    schema: u32,
    vessel: Uuid,
    session: Uuid,
    grant: Uuid,
    principal: Uuid,
    revision: u64,
    epoch: String,
}

pub(in crate::process) fn binding_digest(
    vessel: Uuid,
    binding: &ExecutionBinding,
) -> Result<String> {
    // Incarnation is deliberately not authority. Live input still carries its
    // own exact incarnation; ordinary clean restart must not promote/revoke the
    // stable identity's session grant merely because its PID changed.
    let mut hash = Sha256::new();
    hash.update(b"voyage/scoped-execution-epoch/v1\0");
    hash.update(serde_json::to_vec(&(
        vessel,
        binding.session_id,
        &binding.identity,
        &binding.account_context,
        &binding.peer_uids,
        binding.administrator_grant_id,
        &binding.host_identity_digest,
        &binding.policy_digest,
    ))?);
    Ok(format!("{:x}", hash.finalize()))
}

pub(in crate::process) fn current(root: &Path, session: Uuid) -> Result<Option<String>> {
    if !super::super::runtime_storage::has_bound_layout(root) {
        return Ok(None);
    }
    #[cfg(target_os = "linux")]
    {
        ensure!(
            unsafe { libc::getuid() } == 0 && unsafe { libc::geteuid() } == 0,
            "protected grant epoch requires root supervisor"
        );
        voyage_storage::protected_linux::RootDirectory::open(root)?;
        let vessel = super::super::identity::public(root)?.vessel_id;
        let db = super::super::database::open(root)?;
        let record: Option<String> = db
            .query_row(
                "SELECT record FROM execution_bindings WHERE session_id=?1",
                [session.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let binding = if let Some(record) = record {
            let binding: ExecutionBinding = serde_json::from_str(&record)?;
            ensure!(
                binding.session_id == session,
                "execution grant epoch session mismatch"
            );
            binding
        } else {
            // Before ordinary creation, pin the explicitly provisioned default.
            // An initial administrator admission produces a different epoch and
            // requires a newly derived human scope after its approved binding.
            let identity = super::super::default_execution::protected_default(root)?;
            super::super::default_execution::binding(&identity, session)?
        };
        Ok(Some(binding_digest(vessel, &binding)?))
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("system execution grant epoch unavailable on this host")
}

fn expected(root: &Path, grant: &ProcessGrant) -> Result<Option<EpochPin>> {
    let Some(epoch) = current(root, grant.session_id)? else {
        return Ok(None);
    };
    Ok(Some(EpochPin {
        schema: 1,
        vessel: super::super::identity::public(root)?.vessel_id,
        session: grant.session_id,
        grant: grant.grant_id,
        principal: grant.principal_id,
        revision: grant.revision,
        epoch,
    }))
}

pub(in crate::process) fn pin(root: &Path, grant: &ProcessGrant) -> Result<()> {
    let Some(pin) = expected(root, grant)? else {
        return Ok(());
    };
    ensure!(
        !pin.vessel.is_nil() && !pin.grant.is_nil() && pin.revision > 0,
        "invalid execution epoch identity"
    );
    #[cfg(target_os = "linux")]
    {
        let directory = voyage_storage::protected_linux::RootDirectory::open(root)?
            .create_child("access".as_ref())?
            .create_child("execution-epochs".as_ref())?;
        let name = format!("{}.json", grant.grant_id);
        let bytes = serde_json::to_vec(&pin)?;
        match directory.read(name.as_ref(), 4096) {
            Ok(prior) => ensure!(
                prior == bytes,
                "scoped credential cannot adopt a changed execution identity"
            ),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                directory.publish_new(name.as_ref(), &bytes, 4096)?
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(in crate::process) fn check(root: &Path, grant: &ProcessGrant) -> Result<()> {
    let latest: ProcessGrant = super::store::load(&super::store::grant_path(root, grant.grant_id))?;
    ensure!(
        super::store::process_authority_fingerprint(&latest)?
            == super::store::process_authority_fingerprint(grant)?,
        "saved session authority changed during admission"
    );
    let Some(expected) = expected(root, grant)? else {
        return Ok(());
    };
    #[cfg(target_os = "linux")]
    {
        let directory = voyage_storage::protected_linux::RootDirectory::open(root)?
            .child("access".as_ref())?
            .child("execution-epochs".as_ref())?;
        let saved: EpochPin = serde_json::from_slice(
            &directory.read(format!("{}.json", grant.grant_id).as_ref(), 4096)?,
        )?;
        ensure!(
            saved == expected,
            "session execution identity changed; renew scoped authority"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::{
        execution_identity::{AccountContextRef, IdentityRef},
        process::ProcessPeerUids,
    };

    #[test]
    fn scoped_authority_follows_identity_not_ordinary_process_replacement() {
        let vessel = Uuid::new_v4();
        let binding = ExecutionBinding {
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            identity: IdentityRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            account_context: AccountContextRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            peer_uids: ProcessPeerUids {
                supervisor: 0,
                runtime: 1000,
            },
            administrator_grant_id: None,
            host_identity_digest: "host".into(),
            policy_digest: "policy".into(),
        };
        let original = binding_digest(vessel, &binding).unwrap();
        let mut restarted = binding.clone();
        restarted.incarnation = Uuid::new_v4();
        assert_eq!(original, binding_digest(vessel, &restarted).unwrap());
        for field in 0..6 {
            let mut changed = restarted.clone();
            match field {
                0 => changed.identity.revision = 2.try_into().unwrap(),
                1 => changed.account_context.id = Uuid::new_v4(),
                2 => {
                    changed.administrator_grant_id = Some(Uuid::new_v4());
                    changed.peer_uids.runtime = 0;
                }
                3 => changed.peer_uids.runtime = 1001,
                4 => changed.policy_digest = "changed policy".into(),
                _ => changed.host_identity_digest = "changed host".into(),
            }
            assert_ne!(original, binding_digest(vessel, &changed).unwrap());
        }
        assert_ne!(original, binding_digest(Uuid::new_v4(), &binding).unwrap());
    }
}
