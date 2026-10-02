//! Actual lazy Catalog -> runtime register -> ToolRegistry boundaries. No ELF execution.
use super::*;
use crate::extensions::catalog::Scope;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
const ROOT: &str = "VOYAGE_OWNED_REGISTRY_ROOT";
const CASE: &str = "VOYAGE_OWNED_REGISTRY_CASE";

struct Fixture {
    workspace: std::path::PathBuf,
    catalog: Catalog,
    config: crate::Config,
    manager: Arc<Manager>,
}
impl Fixture {
    fn new(root: &std::path::Path) -> Self {
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let catalog = Catalog::new(&workspace, &crate::config::default_data_dir()).unwrap();
        let config = crate::Config {
            access: Some(crate::config::AccessMode::Unrestricted),
            sandbox: crate::sandbox::Settings {
                mode: crate::sandbox::Mode::Required,
                ..Default::default()
            },
            extension_private_files_complete: true,
            ..Default::default()
        };
        Self {
            workspace,
            catalog,
            config,
            manager: Arc::new(Manager::default()),
        }
    }
    fn package(&self, scope: Scope, id: &str, host_read: bool, many: bool) -> String {
        let mut package = crate::extensions::package_tests::package();
        package.manifest.id = id.into();
        if host_read {
            package.manifest.capabilities.push("host.file.read".into());
        }
        if many {
            package.manifest.definitions["commands"] = json!([{"name":"named","description":"owned command","input_schema":{"type":"object"},"output_schema":true}]);
            package.manifest.definitions["lifecycle"] = json!([
                {"name":"run_start","description":"owned start","input_schema":{"type":"object"},"output_schema":true},
                {"name":"run_finish","description":"owned finish","input_schema":{"type":"object"},"output_schema":true}
            ]);
        }
        let bytes = serde_json::to_vec(&package).unwrap();
        let digest = crate::extensions::digest(&bytes);
        self.catalog
            .mutate(scope, id, None, Some(&bytes), None)
            .unwrap();
        self.catalog
            .review_executable(scope, id, &digest, &package.manifest.capabilities)
            .unwrap();
        digest
    }
    fn register(&self, registry: &mut crate::tools::ToolRegistry) -> Result<()> {
        let policy = crate::policy::Policy::new(&self.config, self.workspace.clone()).unwrap();
        register(registry, self.manager.clone(), &policy, &self.config)
    }
    fn pending(&self) -> usize {
        self.manager.state.lock().unwrap().children.len()
    }
    fn names(registry: &crate::tools::ToolRegistry) -> Vec<String> {
        registry.definitions().into_iter().map(|d| d.name).collect()
    }
    fn context(&self) -> ToolContext {
        crate::tools::reliability_tests::context(&self.workspace)
    }
}
fn no_reservations() {
    let path = crate::config::default_data_dir().join("host-resources/reservations.sqlite3");
    if path.exists() {
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let reservations: i64 = db
            .query_row("SELECT count(*) FROM reservations", [], |row| row.get(0))
            .unwrap();
        let extensions: i64 = db
            .query_row("SELECT count(*) FROM extension_reservations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(reservations, 0);
        assert_eq!(extensions, 0);
    }
}
#[derive(Clone)]
struct Existing;
#[async_trait]
impl Tool for Existing {
    fn definition(&self) -> crate::model::ToolDefinition {
        crate::model::ToolDefinition {
            name: "ext_example_echo".into(),
            description: "existing independent tool".into(),
            input_schema: json!({"type":"object"}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
        Ok("existing-owner-result".into())
    }
}

#[test]
fn owned_registry_child() {
    let Some(root) = std::env::var_os(ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let metadata = std::fs::symlink_metadata(&root).unwrap();
    assert!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
    );
    assert!(crate::config::default_data_dir().starts_with(root.join("data")));
    let case: usize = std::env::var(CASE).unwrap().parse().unwrap();
    if case != 0 {
        crate::host_resources::set_process_scope(Uuid::new_v4(), Uuid::new_v4()).unwrap();
    }
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut f = Fixture::new(&root);
        let mut registry = crate::tools::ToolRegistry::default();
        match case {
            0 => {
                f.package(Scope::Project, "example", false, true);
                f.register(&mut registry).unwrap();
                assert!(Fixture::names(&registry).is_empty());
                assert!(f.manager.state.lock().unwrap().executors.is_empty());
            }
            1 => {
                f.package(Scope::Project, "example", false, true);
                f.register(&mut registry).unwrap();
                assert_eq!(
                    Fixture::names(&registry),
                    vec!["ext_example_echo", "extcmd_example_named"]
                );
                assert_eq!(f.manager.state.lock().unwrap().executors.len(), 1);
                // Lifecycle handlers are internal: not advertised as model tools.
                let mut context = f.context();
                context.policy = Arc::new(
                    crate::policy::Policy::new(
                        &crate::Config {
                            access: Some(crate::config::AccessMode::ReadOnly),
                            ..Default::default()
                        },
                        f.workspace.clone(),
                    )
                    .unwrap(),
                );
                registry
                    .run_extension_lifecycle("run_start", &context)
                    .await;
                registry
                    .run_extension_lifecycle("run_finish", &context)
                    .await;
                assert_eq!(f.pending(), 0);
            }
            2 => {
                f.package(Scope::User, "example", true, false);
                f.config.extension_private_files_complete = false;
                registry.register(crate::tools::ReadFile);
                f.register(&mut registry).unwrap();
                assert_eq!(Fixture::names(&registry), vec!["read_file"]);
                assert!(f.manager.state.lock().unwrap().executors.is_empty());
                f.config.extension_private_files_complete = true;
                let private = root.join("private-source");
                std::fs::write(&private, b"synthetic private source").unwrap();
                f.config.extension_private_files.push(
                    crate::extensions::PrivateFile::capture(
                        &private,
                        &std::fs::metadata(&private).unwrap(),
                    )
                    .unwrap(),
                );
                f.register(&mut registry).unwrap();
                assert_eq!(
                    Fixture::names(&registry),
                    vec!["ext_example_echo", "read_file"]
                );
                assert!(f.manager.read_allowed.load(Ordering::Acquire));
                assert_eq!(f.manager.private_files.lock().unwrap().len(), 1);
                f.manager.restrict_host_read(false);
                f.manager.restrict_host_read(true);
                assert!(!f.manager.read_allowed.load(Ordering::Acquire));
            }
            3 => {
                f.package(Scope::User, "example", false, true);
                registry.register(Existing);
                f.register(&mut registry).unwrap();
                assert_eq!(Fixture::names(&registry), vec!["ext_example_echo"]);
                assert_eq!(
                    registry
                        .execute("ext_example_echo", json!({}), &f.context())
                        .await
                        .unwrap(),
                    "existing-owner-result"
                );
                assert!(f.manager.state.lock().unwrap().executors.is_empty());
            }
            4 => {
                let digest = f.package(Scope::Project, "example", false, false);
                f.register(&mut registry).unwrap();
                let inspection = f.catalog.inspect(Scope::Project, "example").unwrap();
                f.catalog
                    .revoke_execution(&inspection.binding, &digest)
                    .unwrap();
                let manager = Arc::new(Manager::default());
                let mut next = crate::tools::ToolRegistry::default();
                let policy = crate::policy::Policy::new(&f.config, f.workspace.clone()).unwrap();
                register(&mut next, manager.clone(), &policy, &f.config).unwrap();
                assert!(Fixture::names(&next).is_empty());
                assert_eq!(Fixture::names(&registry), vec!["ext_example_echo"]);
                assert!(manager.state.lock().unwrap().executors.is_empty());
                manager.shutdown().await.unwrap();
            }
            5 => {
                f.package(Scope::User, "example", false, false);
                f.package(Scope::Project, "example", false, true);
                f.register(&mut registry).unwrap();
                let names = Fixture::names(&registry);
                assert_eq!(names, vec!["ext_example_echo", "extcmd_example_named"]);
                assert_eq!(f.manager.state.lock().unwrap().executors.len(), 1);
                assert_eq!(f.pending(), 0);
            }
            6 => {
                f.package(Scope::Project, "example", false, true);
                f.manager.shutdown().await.unwrap();
                assert!(f.register(&mut registry).is_err());
                assert!(Fixture::names(&registry).is_empty());
                assert!(f.manager.state.lock().unwrap().executors.is_empty());
            }
            7 => {
                f.package(Scope::Project, "example", false, false);
                let snapshot = Arc::new(f.catalog.executable_snapshots().unwrap().remove(0));
                let contexts = Arc::new(Mutex::new(BTreeMap::new()));
                let invocation = Uuid::new_v4();
                let context = f.context();
                contexts
                    .lock()
                    .unwrap()
                    .insert(invocation, (context.clone(), "tool.echo".into()));
                let adapter = Adapter {
                    catalog: Arc::new(f.catalog),
                    snapshot: snapshot.clone(),
                    manager: Arc::downgrade(&f.manager),
                    contexts: contexts.clone(),
                };
                let (session, incarnation) = crate::host_resources::process_scope().unwrap();
                let identity = sdk::Identity {
                    session,
                    incarnation,
                    run: context.execution_id,
                    invocation,
                    package: snapshot.archive.manifest.id.clone(),
                    digest: snapshot.digest.clone(),
                };
                for mismatch in 0..5 {
                    let mut bad = identity.clone();
                    match mismatch {
                        0 => bad.session = Uuid::new_v4(),
                        1 => bad.incarnation = Uuid::new_v4(),
                        2 => bad.run = Uuid::new_v4(),
                        3 => bad.package = "foreign".into(),
                        _ => bad.digest = "f".repeat(64),
                    };
                    assert!(
                        adapter
                            .launch(&bad, Instant::now() + Duration::from_secs(1))
                            .await
                            .is_err()
                    );
                }
                assert!(adapter.launch(&identity, Instant::now()).await.is_err());
                context.cancellation.cancel();
                assert!(
                    adapter
                        .launch(&identity, Instant::now() + Duration::from_secs(1))
                        .await
                        .is_err()
                );
                assert!(f.manager.state.lock().unwrap().children.is_empty());
                contexts.lock().unwrap().clear();
                f.manager.shutdown().await.unwrap();
                no_reservations();
                std::fs::write(root.join("completed"), b"qualified-local-boundaries").unwrap();
                return;
            }
            8 => {
                f.package(Scope::User, "example", false, false);
                f.register(&mut registry).unwrap();
                let definition = registry.definitions().pop().unwrap();
                assert_eq!(definition.name, "ext_example_echo");
                let mut context = f.context();
                context.cancellation.cancel();
                assert!(
                    registry
                        .execute("ext_example_echo", json!({}), &context)
                        .await
                        .is_err()
                );
                context = f.context();
                context.policy = Arc::new(
                    crate::policy::Policy::new(
                        &crate::Config {
                            access: Some(crate::config::AccessMode::ReadOnly),
                            ..Default::default()
                        },
                        f.workspace.clone(),
                    )
                    .unwrap(),
                );
                assert!(
                    registry
                        .execute("ext_example_echo", json!({}), &context)
                        .await
                        .is_err()
                );
                assert_eq!(f.pending(), 0);
                assert_eq!(f.manager.state.lock().unwrap().executors.len(), 1);
            }
            _ => panic!("unknown owned registry journey"),
        }
        assert_eq!(f.pending(), 0);
        no_reservations();
        f.manager.shutdown().await.unwrap();
        assert!(f.manager.state.lock().unwrap().closed);
        assert!(
            f.manager
                .state
                .lock()
                .unwrap()
                .executors
                .iter()
                .all(|e| e.tasks_drained())
        );
        std::fs::write(root.join("completed"), b"qualified-local-boundaries").unwrap();
    });
}
fn run(case: usize) {
    let root = tempfile::tempdir().unwrap();
    let log = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.path().join("child.log"))
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args([
            "--exact",
            "extensions::runtime::owned_registry_journeys::owned_registry_child",
            "--nocapture",
        ])
        .env(ROOT, root.path())
        .env(CASE, case.to_string())
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "owned registry case {case} failed: {}",
                std::fs::read_to_string(root.path().join("child.log")).unwrap()
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned registry case {case} exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(root.path().join("completed")).unwrap(),
        b"qualified-local-boundaries"
    );
}
macro_rules! journey {
    ($name:ident, $case:literal) => {
        #[test]
        fn $name() {
            run($case);
        }
    };
}
journey!(
    unsupervised_reviewed_packages_never_contribute_runtime_tools,
    0
);
journey!(
    whole_registration_namespaces_tools_commands_and_internal_lifecycle_without_launch,
    1
);
journey!(
    host_read_registration_requires_complete_private_provenance_and_current_ceiling,
    2
);
journey!(
    package_collision_preserves_existing_tool_and_skips_every_package_candidate,
    3
);
journey!(
    execution_revocation_prevents_fresh_registration_without_rewriting_old_snapshot,
    4
);
journey!(
    project_and_user_name_collision_never_installs_competing_executors,
    5
);
journey!(
    closed_resource_manager_refuses_complete_registration_before_tools_publish,
    6
);
journey!(
    actual_adapter_context_identity_deadline_and_cancellation_refuse_before_image_or_spawn,
    7
);
journey!(
    registered_tool_cancellation_and_read_only_refuse_before_child_or_receipt,
    8
);
