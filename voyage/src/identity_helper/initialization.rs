//! Initialization records and canonical configuration are read only after UID drop.
use super::*;
use crate::attachment::{journal::Journal, local_actor::storage::Directory};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use voyage_protocol::process::RuntimeInitialization;

fn provenance(workspace: &Path, initialize: &RuntimeInitialization) -> Result<()> {
    match initialize {
        RuntimeInitialization::Branch {
            source_directory,
            source_session_id,
            source_command_id,
            branch_id,
        } => {
            ensure!(
                !source_session_id.is_nil()
                    && !source_command_id.is_nil()
                    && !branch_id.is_nil()
                    && branch_id != source_session_id,
                "invalid branch provenance"
            );
            Directory::open_existing(source_directory)?;
        }
        RuntimeInitialization::ManagedImport {
            source_directory,
            transfer_id,
            ..
        } => {
            ensure!(!transfer_id.is_nil(), "invalid managed import provenance");
            Directory::open_existing(source_directory)?;
        }
        RuntimeInitialization::Import {
            source_directory,
            transfer_id,
            source_sha256,
            ..
        } => {
            ensure!(
                !transfer_id.is_nil()
                    && source_sha256.len() == 64
                    && source_sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "invalid JSON import provenance"
            );
            Directory::open_existing(source_directory)?;
        }
        RuntimeInitialization::Participant {
            assignment_id,
            parent_vessel_id,
            parent_session_id,
            parent_run_id,
            policy,
        } => {
            ensure!(
                [
                    assignment_id,
                    parent_vessel_id,
                    parent_session_id,
                    parent_run_id
                ]
                .iter()
                .all(|id| !id.is_nil()),
                "invalid participant provenance"
            );
            // The runtime applies the policy and exact grant separately; this is never authority.
            ensure!(
                serde_json::to_vec(policy)?.len() <= 65_536,
                "participant policy exceeds bounds"
            );
        }
        RuntimeInitialization::Transfer {
            artifact_path,
            sha256,
            prepare_digest,
            transfer_id,
            generation,
        } => {
            ensure!(
                !transfer_id.is_nil()
                    && *generation > 0
                    && sha256.len() == 64
                    && prepare_digest.len() == 64,
                "invalid transfer provenance"
            );
            let parent = artifact_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("transfer parent missing"))?;
            let name = artifact_path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| anyhow::anyhow!("transfer name missing"))?;
            Directory::open_existing(parent)?;
            let _ = name;
            let (bytes, _) = read_artifact(artifact_path)?;
            ensure!(
                hex::encode(Sha256::digest(&bytes)) == *sha256,
                "transfer artifact changed"
            );
        }
    }
    ensure!(
        workspace.is_absolute(),
        "absolute initialization workspace required"
    );
    Ok(())
}

pub(super) fn capture(
    scope: &IdentityAccountScope,
    workspace: &Path,
    directory: &Path,
    request: &str,
    base: Option<&Path>,
    initialize: &RuntimeInitialization,
) -> Result<serde_json::Value> {
    ensure!(
        request.len() == 64 && request.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid initialization request digest"
    );
    provenance(workspace, initialize)?;
    let storage = Directory::open_existing(directory)?;
    let lock = storage.lock()?;
    let retained = serde_json::to_vec(
        &serde_json::json!({"schema":1,"request_digest":request,"initialize":initialize}),
    )?;
    ensure!(
        retained.len() <= 65_536,
        "initialization provenance exceeds bounds"
    );
    if let Some(prior) = storage.read_bounded("initialization-intent.json", 65_536)? {
        ensure!(prior == retained, "initialization provenance changed");
        let bytes = frozen_config(&directory.join("launch.json"))?;
        let config = serde_json::from_slice::<crate::launch_config::LaunchConfig>(&bytes)?
            .resolve(workspace)?;
        let registry = crate::accounts::Registry::default_host()?;
        use_account(
            scope,
            &registry,
            config
                .account
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("explicit initialization account required"))?,
            workspace,
        )?;
        ensure!(
            storage.read_bounded("request.json", 256)?.as_deref() == Some(request.as_bytes()),
            "initialization capture not committed"
        );
        return Ok(
            serde_json::json!({"config_path":directory.join("launch.json"),"config_digest":config_digest(&bytes)}),
        );
    }
    ensure!(
        storage
            .read_bounded("launch.json", PRIVATE_CONFIG_BYTES)?
            .is_none()
            && storage.read_bounded("request.json", 256)?.is_none(),
        "unmatched initialization capture"
    );
    storage.publish_new("initialization-intent.json", &retained)?;
    // Branch configuration is an immutable owner-created snapshot. Root does not
    // read the source journal or config, and a branch cannot change namespaces.
    let branch = if matches!(initialize, RuntimeInitialization::Branch { .. }) {
        Journal::frozen_branch_configuration(Some(initialize))?
    } else {
        None
    };
    if let Some(branch) = branch {
        let bytes = branch.as_bytes();
        ensure!(
            bytes.len() <= PRIVATE_CONFIG_BYTES,
            "branch configuration exceeds private limit"
        );
        let config = serde_json::from_slice::<crate::launch_config::LaunchConfig>(bytes)?
            .resolve(workspace)?;
        let registry = crate::accounts::Registry::default_host()?;
        use_account(
            scope,
            &registry,
            config
                .account
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("explicit branch account required"))?,
            workspace,
        )?;
        crate::runtime_policy::RuntimePolicy::resolve(&config, workspace)?;
        storage.publish_new("request.json", request.as_bytes())?;
        storage.publish_new("launch.json", bytes)?;
        return Ok(
            serde_json::json!({"config_path":directory.join("launch.json"),"config_digest":config_digest(bytes)}),
        );
    }
    drop(lock);
    capture_launch(
        scope,
        workspace,
        directory,
        request,
        base,
        None,
        &Default::default(),
    )
}

#[cfg(unix)]
fn read_artifact(path: &Path) -> Result<(Vec<u8>, std::fs::Metadata)> {
    use std::{
        ffi::CString,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    ensure!(path.is_absolute(), "absolute artifact path required");
    let mut directory = std::fs::File::open("/")?;
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => {
                let name = CString::new(value.as_bytes())?;
                let last = index + 1 == components.len();
                let flags = libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | if last { 0 } else { libc::O_DIRECTORY };
                let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
                ensure!(fd >= 0, "private artifact open refused");
                let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
                if !last {
                    directory = file;
                    continue;
                }
                let metadata = file.metadata()?;
                ensure!(
                    metadata.is_file()
                        && metadata.uid() == unsafe { libc::geteuid() }
                        && metadata.mode() & 0o077 == 0
                        && metadata.nlink() == 1
                        && metadata.len() <= 16 * 1024 * 1024,
                    "unsafe private artifact"
                );
                let mut bytes = Vec::new();
                (&mut file)
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= 16 * 1024 * 1024 && bytes.len() as u64 == metadata.len(),
                    "artifact changed during bounded read"
                );
                let after = file.metadata()?;
                ensure!(
                    after.dev() == metadata.dev()
                        && after.ino() == metadata.ino()
                        && after.uid() == metadata.uid()
                        && after.mode() == metadata.mode()
                        && after.nlink() == 1
                        && after.len() == metadata.len(),
                    "private artifact metadata changed during read"
                );
                return Ok((bytes, metadata));
            }
            _ => anyhow::bail!("relative artifact components refused"),
        }
    }
    anyhow::bail!("artifact file missing")
}
#[cfg(not(unix))]
fn read_artifact(_path: &Path) -> Result<(Vec<u8>, std::fs::Metadata)> {
    anyhow::bail!("private artifact observation unsupported")
}

/// The supervisor sees no canonical history, provider state or configuration.
#[cfg(unix)]
pub(super) fn observe_transfer(path: &Path) -> Result<serde_json::Value> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("artifact parent missing"))?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| anyhow::anyhow!("artifact name missing"))?;
    Directory::open_existing(parent)?;
    let _ = name;
    let (bytes, metadata) = read_artifact(path)?;
    let checkpoint: voyage_protocol::process::PortableCheckpoint = serde_json::from_slice(&bytes)?;
    Ok(
        serde_json::json!({"session_id":checkpoint.session_id,"transfer_id":checkpoint.transfer_id,"destination_vessel_id":checkpoint.destination_vessel_id,"prepare_digest":checkpoint.prepare_digest,"generation":checkpoint.generation,"artifact_sha256":hex::encode(Sha256::digest(&bytes)),"artifact_bytes":bytes.len(),"artifact_device":metadata.dev(),"artifact_inode":metadata.ino(),"artifact_uid":metadata.uid()}),
    )
}

#[cfg(not(unix))]
pub(super) fn observe_transfer(_path: &Path) -> Result<serde_json::Value> {
    anyhow::bail!("private transfer observation unsupported")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use uuid::Uuid;
    fn participant() -> RuntimeInitialization {
        RuntimeInitialization::Participant {
            assignment_id: Uuid::new_v4(),
            parent_vessel_id: Uuid::new_v4(),
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            policy: voyage_protocol::process::ParticipantPolicy {
                access: "workspace-write".into(),
                legacy_deny_commands: vec![],
                inherit_env: vec![],
                github_enabled: false,
                timeout_secs: 10,
                max_output_bytes: 1024,
                max_subagents: 0,
            },
        }
    }
    fn scope(root: &Path) -> IdentityAccountScope {
        IdentityAccountScope {
            full_access: false,
            can_use: false,
            can_enroll: false,
            account_ids: vec![],
            enrollment_connections: vec![],
            actor: voyage_protocol::accounts::EnrollmentActor {
                principal: "synthetic ordinary identity".into(),
                workspace: root.to_string_lossy().into_owned(),
            },
        }
    }
    #[test]
    fn transfer_provenance_refuses_changed_private_bytes_and_inode_weakening() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("initialization-artifact.json");
        let bytes = b"opaque transport bytes; never privileged JSON parsing";
        std::fs::write(&file, bytes).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let init = RuntimeInitialization::Transfer {
            transfer_id: Uuid::new_v4(),
            artifact_path: file.clone(),
            sha256: hex::encode(Sha256::digest(bytes)),
            prepare_digest: "a".repeat(64),
            generation: 1,
        };
        provenance(root.path(), &init).unwrap();
        std::fs::write(&file, b"changed opaque bytes").unwrap();
        assert!(provenance(root.path(), &init).is_err());
        std::fs::write(&file, bytes).unwrap();
        let hardlink = root.path().join("duplicate");
        std::fs::hard_link(&file, &hardlink).unwrap();
        assert!(provenance(root.path(), &init).is_err());
        std::fs::remove_file(hardlink).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(provenance(root.path(), &init).is_err());
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), &file).unwrap();
        assert!(provenance(root.path(), &init).is_err());
    }
    #[test]
    fn initialization_sources_require_private_owned_directory_without_creating_it() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing-source");
        for init in [
            RuntimeInitialization::ManagedImport {
                transfer_id: Uuid::new_v4(),
                source_directory: missing.clone(),
                expected_revision: 7,
            },
            RuntimeInitialization::Import {
                transfer_id: Uuid::new_v4(),
                source_directory: missing.clone(),
                expected_revision: 7,
                source_sha256: "a".repeat(64),
            },
            RuntimeInitialization::Branch {
                source_directory: missing.clone(),
                source_session_id: Uuid::new_v4(),
                source_command_id: Uuid::new_v4(),
                branch_id: Uuid::new_v4(),
            },
        ] {
            assert!(provenance(root.path(), &init).is_err());
            assert!(!missing.exists());
        }
        std::fs::create_dir(&missing).unwrap();
        std::fs::set_permissions(&missing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let init = RuntimeInitialization::ManagedImport {
            transfer_id: Uuid::new_v4(),
            source_directory: missing.clone(),
            expected_revision: 7,
        };
        assert!(provenance(root.path(), &init).is_err());
        std::fs::set_permissions(&missing, std::fs::Permissions::from_mode(0o700)).unwrap();
        provenance(root.path(), &init).unwrap();
    }
    #[test]
    fn interrupted_initialization_capture_is_not_recaptured_from_changed_defaults() {
        let root = tempfile::tempdir().unwrap();
        let init = participant();
        let request = "a".repeat(64);
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema":1,"request_digest":request,"initialize":init}),
        )
        .unwrap();
        let storage = Directory::open_existing(root.path()).unwrap();
        storage
            .publish_new("initialization-intent.json", &bytes)
            .unwrap();
        assert!(
            capture(
                &scope(root.path()),
                root.path(),
                root.path(),
                &request,
                None,
                &init
            )
            .is_err()
        );
        assert!(!root.path().join("launch.json").exists());
        assert!(!root.path().join("request.json").exists());
        assert_eq!(
            storage
                .read_bounded("initialization-intent.json", 65_536)
                .unwrap()
                .unwrap(),
            bytes
        );
    }
    #[test]
    fn changed_initialization_provenance_cannot_reuse_a_retained_capture() {
        let root = tempfile::tempdir().unwrap();
        let init = participant();
        let request = "a".repeat(64);
        let storage = Directory::open_existing(root.path()).unwrap();
        let prior = serde_json::to_vec(
            &serde_json::json!({"schema":1,"request_digest":"b".repeat(64),"initialize":init}),
        )
        .unwrap();
        storage
            .publish_new("initialization-intent.json", &prior)
            .unwrap();
        let error = capture(
            &scope(root.path()),
            root.path(),
            root.path(),
            &request,
            None,
            &init,
        )
        .unwrap_err();
        assert!(error.to_string().contains("provenance changed"));
        assert!(!root.path().join("launch.json").exists());
        assert_eq!(
            storage
                .read_bounded("initialization-intent.json", 65_536)
                .unwrap()
                .unwrap(),
            prior
        );
    }
    #[test]
    fn invalid_participant_references_and_self_branch_never_capture_configuration() {
        let root = tempfile::tempdir().unwrap();
        let mut init = participant();
        if let RuntimeInitialization::Participant { parent_run_id, .. } = &mut init {
            *parent_run_id = Uuid::nil();
        }
        assert!(
            capture(
                &scope(root.path()),
                root.path(),
                root.path(),
                &"a".repeat(64),
                None,
                &init
            )
            .is_err()
        );
        assert!(!root.path().join("initialization-intent.json").exists());
        let source = Uuid::new_v4();
        let branch = RuntimeInitialization::Branch {
            source_directory: root.path().to_owned(),
            source_session_id: source,
            source_command_id: Uuid::new_v4(),
            branch_id: source,
        };
        assert!(provenance(root.path(), &branch).is_err());
    }
    #[test]
    fn checkpoint_observation_returns_only_frozen_metadata_not_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("checkpoint.json");
        let transfer = Uuid::new_v4();
        let session = Uuid::new_v4();
        let destination = Uuid::new_v4();
        let checkpoint = voyage_protocol::process::PortableCheckpoint {
            transfer_id: transfer,
            session_id: session,
            destination_vessel_id: destination,
            prepare_digest: "a".repeat(64),
            generation: 7,
            session: serde_json::json!({"private_history":"synthetic-private-history","provider_credentials":"synthetic-private-credential"}),
            commands: vec![],
        };
        let bytes = serde_json::to_vec(&checkpoint).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let facts = observe_transfer(&path).unwrap();
        assert_eq!(facts["transfer_id"], serde_json::json!(transfer));
        assert_eq!(facts["session_id"], serde_json::json!(session));
        assert_eq!(facts["artifact_bytes"], serde_json::json!(bytes.len()));
        assert_eq!(
            facts["artifact_sha256"],
            hex::encode(Sha256::digest(&bytes))
        );
        assert!(!facts.to_string().contains("synthetic-private"));
        assert!(facts.get("session").is_none());
        assert!(facts.get("commands").is_none());
        assert!(facts["artifact_inode"].as_u64().is_some());
    }
    #[test]
    fn opaque_transfer_artifacts_above_small_record_limit_remain_bounded() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("artifact.json");
        let bytes = vec![b'x'; 65_537];
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_artifact(&path).unwrap().0, bytes);
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(16 * 1024 * 1024 + 1).unwrap();
        assert!(read_artifact(&path).is_err());
    }
}
