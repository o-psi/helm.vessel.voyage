//! Ordinary identity storage/guard seam. Positive root-created transport and
//! native ownership transition remain separate; no root pipe is manufactured.
use super::*;
use crate::attachment::{journal::Journal, local_actor::storage::Directory};
use crate::identity_helper::ordinary_boundary_tests::{invoke, root};
use crate::tools::process::owned_lifetime_tests::owned;
use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
};
use uuid::Uuid;
macro_rules! case {
    ($name:ident,$body:block) => { #[test] fn $name() {
        owned::run(concat!(module_path!(),"::",stringify!($name)), || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async $body)
        });
    }};
}
struct Fixture {
    directory: std::path::PathBuf,
    session: Uuid,
    incarnation: Uuid,
    config: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let directory = root().join("retired-runtime");
        Directory::open(&directory).unwrap();
        let workspace = root().join("workspace");
        Directory::open(&workspace).unwrap();
        let config = serde_json::to_vec(
            &crate::launch_config::LaunchConfig::capture(&crate::Config::default(), &workspace)
                .unwrap(),
        )
        .unwrap();
        let mut journal = Journal::open(directory.join("journal")).unwrap();
        let session = crate::session::Session::new(workspace, "ordinary helper fixture".into());
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal
            .retain_initial_configuration(&guard, String::from_utf8(config.clone()).unwrap())
            .unwrap();
        drop(guard);
        drop(journal);
        Self {
            directory,
            session: session.id,
            incarnation: Uuid::new_v4(),
            config,
        }
    }
    fn request(&self, operation: TransitionOperation) -> TransitionRequest {
        TransitionRequest {
            schema: SCHEMA,
            session_id: self.session,
            source_incarnation: self.incarnation,
            operation,
        }
    }
    fn observe(&self) -> TransitionRequest {
        self.request(TransitionOperation::Observe {
            directory: self.directory.clone(),
        })
    }
    fn facts(&self) -> RetiredJournalFacts {
        let TransitionResponse::Facts { facts } = apply(self.observe()).unwrap() else {
            panic!("retired metadata expected")
        };
        facts
    }
    fn prepared(&self) -> PreparedTransitionReceipt {
        let request = self.request(TransitionOperation::SourceFreeze {
            directory: self.directory.clone(),
            command_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            target_incarnation: Uuid::new_v4(),
            expected: self.facts(),
            target_uid: unsafe { libc::geteuid() },
            target_gid: unsafe { libc::getegid() },
            target_config_digest: crate::identity_helper::config_digest(&self.config),
            review_digest: "a".repeat(64),
        });
        let TransitionResponse::Prepared { receipt } = apply(request).unwrap() else {
            panic!("prepared private receipt expected")
        };
        receipt
    }
    fn target(&self, expected: PreparedTransitionReceipt) -> TransitionRequest {
        let storage = Directory::open(&root().join("target-config")).unwrap();
        storage.publish_new("launch.json", &self.config).unwrap();
        self.request(TransitionOperation::TargetCommit {
            directory: self.directory.clone(),
            command_id: Uuid::new_v4(),
            expected,
            target_config_path: root().join("target-config/launch.json"),
        })
    }
    fn abort(&self, expected: PreparedTransitionReceipt) -> TransitionResponse {
        apply(self.request(TransitionOperation::AbortSource {
            directory: self.directory.clone(),
            command_id: Uuid::new_v4(),
            expected,
        }))
        .unwrap()
    }
    fn guards_released(&self) {
        let held = hold(&self.observe()).unwrap();
        drop(held);
    }
}
case!(
    ordinary_stdio_cannot_enter_root_private_transition_transport,
    {
        let (result, output) = invoke(
            b"synthetic-private-root-pipe-canary",
            crate::transition_helper::run(),
        )
        .await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "retired journal helper unavailable"
        );
        assert!(output.is_empty());
        assert!(!root().join("startup.lock").exists());
    }
);
case!(
    ordinary_anonymous_pipe_is_refused_and_owned_descriptors_close,
    {
        let mut descriptors = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let read = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        assert!(pipe(read.as_raw_fd()).is_err());
        drop(read);
        drop(write);
        for fd in descriptors {
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EBADF)
            );
        }
    }
);
case!(
    private_retired_observation_preserves_history_and_releases_both_guards,
    {
        let f = Fixture::new();
        let before = f.facts();
        let after = f.facts();
        assert_eq!(before, after);
        assert_eq!(before.session_id, f.session);
        assert_eq!(before.source_incarnation, f.incarnation);
        assert_eq!(
            before.frozen_config_digest,
            crate::identity_helper::config_digest(&f.config)
        );
        assert!(
            !serde_json::to_string(&before)
                .unwrap()
                .contains("ordinary helper fixture")
        );
        f.guards_released();
    }
);
case!(
    retained_private_configuration_publishes_once_and_reuses_exact_bytes,
    {
        let f = Fixture::new();
        let before = f.facts();
        let request = f.request(TransitionOperation::RetainConfiguration {
            directory: f.directory.clone(),
        });
        let first = apply(request.clone()).unwrap();
        assert_eq!(apply(request).unwrap(), first);
        let TransitionResponse::Configuration { path, digest } = first else {
            panic!("configuration identity expected")
        };
        assert_eq!(path, f.directory.join("migration-config.json"));
        assert_eq!(target_bytes(&path).unwrap(), f.config);
        assert_eq!(digest, crate::identity_helper::config_digest(&f.config));
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(
    conflicting_retained_configuration_is_preserved_and_refused,
    {
        let f = Fixture::new();
        let storage = Directory::open_existing(&f.directory).unwrap();
        storage
            .publish_new("migration-config.json", b"unmatched private bytes")
            .unwrap();
        let before = f.facts();
        assert!(
            apply(f.request(TransitionOperation::RetainConfiguration {
                directory: f.directory.clone()
            }))
            .is_err()
        );
        assert_eq!(
            storage
                .read_bounded("migration-config.json", MAX_TARGET_CONFIG)
                .unwrap()
                .unwrap(),
            b"unmatched private bytes"
        );
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(
    unsupported_schema_and_missing_private_journal_do_not_allocate_guards,
    {
        let f = Fixture::new();
        let mut invalid = f.observe();
        invalid.schema = SCHEMA + 1;
        assert!(apply(invalid).is_err());
        assert!(!f.directory.join("startup.lock").exists());
        let missing = root().join("without-journal");
        Directory::open(&missing).unwrap();
        assert!(
            apply(f.request(TransitionOperation::Observe {
                directory: missing.clone()
            }))
            .is_err()
        );
        assert!(!missing.join("journal").exists() && !missing.join("startup.lock").exists());
        f.guards_released();
    }
);
case!(
    startup_contention_refuses_without_stealing_the_existing_guard,
    {
        let f = Fixture::new();
        let before = f.facts();
        let lock = crate::attachment::journal::open_private_file(&f.directory.join("startup.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(apply(f.observe()).is_err());
        assert!(lock.try_lock().is_ok());
        drop(lock);
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(execution_contention_releases_temporary_startup_guard, {
    let f = Fixture::new();
    let before = f.facts();
    let journal = Journal::open(f.directory.join("journal")).unwrap();
    let guard = journal.acquire_execution(f.session).unwrap();
    assert!(apply(f.observe()).is_err());
    let startup =
        crate::attachment::journal::open_private_file(&f.directory.join("startup.lock")).unwrap();
    startup.try_lock().unwrap();
    drop(startup);
    drop(guard);
    drop(journal);
    assert_eq!(f.facts(), before);
    f.guards_released();
});
case!(
    held_readonly_observation_keeps_exact_lifetime_fences_until_drop,
    {
        let f = Fixture::new();
        let before = f.facts();
        let request = f.observe();
        let mut held = hold(&request).unwrap();
        assert_eq!(
            apply_held(&mut held, request.clone()).unwrap(),
            TransitionResponse::Facts {
                facts: before.clone()
            }
        );
        assert!(apply(request).is_err());
        drop(held);
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(
    unknown_lookup_is_exact_absence_without_history_or_configuration_effect,
    {
        let f = Fixture::new();
        let before = f.facts();
        let command = Uuid::new_v4();
        assert_eq!(
            apply(f.request(TransitionOperation::Lookup {
                directory: f.directory.clone(),
                command_id: command
            }))
            .unwrap(),
            TransitionResponse::Absent {
                command_id: command
            }
        );
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(
    same_ordinary_identity_exact_configuration_commits_without_provider_execution,
    {
        let f = Fixture::new();
        let before = f.facts();
        let receipt = f.prepared();
        let request = f.target(receipt.clone());
        let response = apply(request.clone()).unwrap();
        let TransitionResponse::Committed { receipt: committed } = response.clone() else {
            panic!("private committed receipt expected")
        };
        assert_eq!(committed.prepared, receipt);
        assert_eq!(
            committed.config_digest,
            crate::identity_helper::config_digest(&f.config)
        );
        assert_eq!(committed.history_digest, before.history_digest);
        assert_eq!(apply(request).unwrap(), response);
        f.guards_released();
    }
);
case!(
    changed_target_config_is_refused_then_exact_source_abort_releases_fences,
    {
        let f = Fixture::new();
        let before = f.facts();
        let receipt = f.prepared();
        let request = f.target(receipt.clone());
        let path = root().join("target-config/launch.json");
        Directory::open_existing(path.parent().unwrap())
            .unwrap()
            .publish("launch.json", b"changed synthetic target")
            .unwrap();
        assert!(apply(request).is_err());
        let TransitionResponse::Aborted { receipt: aborted } = f.abort(receipt.clone()) else {
            panic!("exact abort expected")
        };
        assert_eq!(aborted.prepared, receipt);
        assert_eq!(aborted.history_digest, before.history_digest);
        f.guards_released();
    }
);
case!(
    wrong_target_identity_cannot_read_config_or_commit_prepared_journal,
    {
        let f = Fixture::new();
        let original = f.prepared();
        let mut changed = original.clone();
        changed.target_uid = changed.target_uid.checked_add(1).unwrap();
        let missing = root().join("never-opened-target/launch.json");
        let request = f.request(TransitionOperation::TargetCommit {
            directory: f.directory.clone(),
            command_id: Uuid::new_v4(),
            expected: changed,
            target_config_path: missing.clone(),
        });
        assert!(apply(request).is_err());
        assert!(!missing.parent().unwrap().exists());
        assert!(matches!(
            f.abort(original),
            TransitionResponse::Aborted { .. }
        ));
        f.guards_released();
    }
);
case!(
    target_config_refuses_relative_parent_traversal_symlink_and_oversize,
    {
        let directory = root().join("configuration");
        let storage = Directory::open(&directory).unwrap();
        storage
            .publish_new("launch.json", b"owned exact bytes")
            .unwrap();
        assert_eq!(
            target_bytes(&directory.join("launch.json")).unwrap(),
            b"owned exact bytes"
        );
        assert!(target_bytes(Path::new("configuration/launch.json")).is_err());
        assert!(target_bytes(&directory.join("../configuration/launch.json")).is_err());
        std::os::unix::fs::symlink(directory.join("launch.json"), directory.join("alias.json"))
            .unwrap();
        assert!(target_bytes(&directory.join("alias.json")).is_err());
        // Intentionally malformed fixture bypasses the publisher's size bound;
        // the real target reader must independently refuse oversized content.
        std::fs::write(
            directory.join("launch.json"),
            vec![b'x'; MAX_TARGET_CONFIG + 1],
        )
        .unwrap();
        assert!(target_bytes(&directory.join("launch.json")).is_err());
    }
);
case!(
    runtime_directory_alias_is_refused_without_creating_alias_owned_state,
    {
        let f = Fixture::new();
        let before = f.facts();
        let alias = root().join("runtime-alias");
        std::os::unix::fs::symlink(&f.directory, &alias).unwrap();
        assert!(apply(f.request(TransitionOperation::Observe { directory: alias })).is_err());
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);

/// Variables are changed only in an exact env-cleared child around synchronous
/// helper admission; no provider or concurrently executing helper reads them.
struct InheritedEnvironment;
impl InheritedEnvironment {
    fn install(startup: Option<i32>, execution: Option<i32>) -> Self {
        for (key, fd) in [
            ("VOYAGE_TRANSITION_STARTUP_FD", startup),
            ("VOYAGE_TRANSITION_EXECUTION_FD", execution),
        ] {
            assert!(std::env::var_os(key).is_none());
            if let Some(fd) = fd {
                unsafe { std::env::set_var(key, fd.to_string()) };
            }
        }
        Self
    }
}
impl Drop for InheritedEnvironment {
    fn drop(&mut self) {
        for key in [
            "VOYAGE_TRANSITION_STARTUP_FD",
            "VOYAGE_TRANSITION_EXECUTION_FD",
        ] {
            unsafe { std::env::remove_var(key) };
        }
    }
}
fn inherited_file(path: &std::path::Path) -> i32 {
    use std::os::fd::IntoRawFd;
    let file = crate::attachment::journal::open_private_file(path).unwrap();
    file.try_lock().unwrap();
    file.into_raw_fd()
}
case!(
    retained_ordinary_lookup_accepts_both_exact_owned_guard_descriptions,
    {
        let f = Fixture::new();
        let command = Uuid::new_v4();
        let lookup = f.request(TransitionOperation::Lookup {
            directory: f.directory.clone(),
            command_id: command,
        });
        let lease = f.request(TransitionOperation::SourceLease {
            directory: f.directory.clone(),
            freeze: Box::new(lookup.clone()),
        });
        let startup = inherited_file(&f.directory.join("startup.lock"));
        let execution = inherited_file(
            &f.directory
                .join("journal")
                .join(format!("{}.execution.lock", f.session)),
        );
        let environment = InheritedEnvironment::install(Some(startup), Some(execution));
        let mut held = hold(&lease).unwrap();
        assert_ne!(
            unsafe { libc::fcntl(startup, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        assert_ne!(
            unsafe { libc::fcntl(execution, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        assert_eq!(
            apply_held(&mut held, lookup).unwrap(),
            TransitionResponse::Absent {
                command_id: command
            }
        );
        drop(held);
        drop(environment);
        for fd in [startup, execution] {
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
        }
        f.guards_released();
    }
);
case!(
    one_retained_guard_cannot_lease_and_its_owned_descriptor_is_closed,
    {
        let f = Fixture::new();
        let before = f.facts();
        let startup = inherited_file(&f.directory.join("startup.lock"));
        let environment = InheritedEnvironment::install(Some(startup), None);
        assert!(hold(&f.observe()).is_err());
        drop(environment);
        assert_eq!(unsafe { libc::fcntl(startup, libc::F_GETFD) }, -1);
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
case!(
    owned_guard_descriptor_for_another_file_cannot_pin_original_lifetime,
    {
        let f = Fixture::new();
        let before = f.facts();
        let startup = inherited_file(&f.directory.join("unrelated-private.lock"));
        let environment = InheritedEnvironment::install(Some(startup), None);
        assert!(hold(&f.observe()).is_err());
        drop(environment);
        assert_eq!(unsafe { libc::fcntl(startup, libc::F_GETFD) }, -1);
        assert_eq!(f.facts(), before);
        f.guards_released();
    }
);
