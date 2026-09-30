//! A private child-process fixture owns its host ledger and one harmless process.
//! It qualifies broker admission/observation mechanics, not executable isolation.
use super::*;
use sdk::Host;
use std::sync::atomic::AtomicUsize;
const ROOT: &str = "VOYAGE_EXTENSION_BOUNDARY_ROOT";
#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        ensure!(
            !self.0.load(Ordering::SeqCst),
            "fixture authority withdrawn"
        );
        Ok(())
    }
}
#[test]
fn owned_broker_child() {
    let Some(root) = std::env::var_os(ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(crate::config::default_data_dir().starts_with(root.join("data")));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        crate::host_resources::set_process_scope(session, incarnation).unwrap();
        let invocation = Uuid::new_v4();
        let mut context = crate::tools::reliability_tests::context(&workspace);
        context.redactor = Arc::new(crate::tools::Redactor::new(["fixture-credential".into()]));
        let authority = Arc::new(Authority(AtomicBool::new(false)));
        context.policy = Arc::new(
            context
                .policy
                .as_ref()
                .clone()
                .with_execution_authority(authority.clone()),
        );
        let binding = "a".repeat(64);
        let digest = "b".repeat(64);
        let reservation = crate::host_resources::extensions::ExtensionReservation::acquire(
            &binding,
            &digest,
            invocation,
            context.execution_id,
            "tool.fixture",
        )
        .unwrap();
        let mut command = std::process::Command::new("/usr/bin/sleep");
        command
            .arg("30")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut command = tokio::process::Command::from(command);
        command.kill_on_drop(true);
        let process = command.spawn().unwrap();
        let identity = Arc::new(
            crate::tools::process::SessionIdentity::capture(process.id().unwrap()).unwrap(),
        );
        let owned = Arc::new(OwnedChild {
            invocation,
            reads: Mutex::new(vec![]),
            child: tokio::sync::Mutex::new(Some(process)),
            identity: Some(identity),
            reservation,
            observed: AtomicBool::new(false),
            process_observed: AtomicBool::new(false),
            observation: tokio::sync::Mutex::new(None),
        });
        let manager = Arc::new(Manager::default());
        manager.read_allowed.store(true, Ordering::Release);
        manager.state.lock().unwrap().children.push(owned.clone());
        let broker = Broker {
            context: context.clone(),
            manager: manager.clone(),
            progress: Arc::new(Mutex::new(vec![])),
        };
        let call = sdk::Identity {
            session,
            incarnation,
            run: context.execution_id,
            invocation,
            package: "fixture".into(),
            digest,
        };
        std::fs::write(workspace.join("safe"), b"safe fixture-credential").unwrap();
        assert_eq!(
            broker
                .read(
                    &call,
                    "safe",
                    0,
                    128,
                    Instant::now() + Duration::from_secs(2)
                )
                .await
                .unwrap(),
            "safe [REDACTED]"
        );
        std::fs::write(workspace.join("binary"), [0xff, 0xfe]).unwrap();
        std::fs::write(workspace.join("too-big"), vec![b'x'; sdk::MAX_READ + 1]).unwrap();
        std::fs::write(workspace.join("limited"), b"abcdef").unwrap();
        std::fs::write(workspace.join("linked"), b"private").unwrap();
        std::fs::hard_link(workspace.join("linked"), workspace.join("alias")).unwrap();
        std::fs::create_dir(workspace.join("directory")).unwrap();
        for (path, bound) in [
            ("binary", 128),
            ("too-big", 128),
            ("limited", 2),
            ("linked", 128),
            ("directory", 128),
        ] {
            assert!(
                broker
                    .read(
                        &call,
                        path,
                        0,
                        bound,
                        Instant::now() + Duration::from_secs(2)
                    )
                    .await
                    .is_err()
            );
        }
        let config = workspace.join("private-config");
        std::fs::write(&config, b"private configuration").unwrap();
        manager.private_files.lock().unwrap().push(
            super::super::PrivateFile::capture(&config, &std::fs::metadata(&config).unwrap())
                .unwrap(),
        );
        let renamed = workspace.join("renamed-config");
        std::fs::rename(&config, &renamed).unwrap();
        assert!(
            broker
                .read(
                    &call,
                    "renamed-config",
                    0,
                    128,
                    Instant::now() + Duration::from_secs(2)
                )
                .await
                .is_err()
        );
        authority.0.store(true, Ordering::SeqCst);
        let count = owned.reads.lock().unwrap().len();
        assert!(
            broker
                .read(
                    &call,
                    "safe",
                    0,
                    128,
                    Instant::now() + Duration::from_secs(2)
                )
                .await
                .is_err()
        );
        assert_eq!(owned.reads.lock().unwrap().len(), count);
        authority.0.store(false, Ordering::SeqCst);
        for _ in 0..256 {
            broker.progress(&call, "").await.unwrap();
        }
        assert!(broker.progress(&call, "overflow").await.is_err());
        let (send, receive) = tokio::sync::oneshot::channel();
        owned.reads.lock().unwrap().push(Arc::new(OwnedRead {
            task: tokio::sync::Mutex::new(Some(tokio::spawn(async move {
                receive.await.unwrap();
                Ok("discarded result".into())
            }))),
        }));
        let observer = owned.clone();
        let waiter = tokio::spawn(async move { observer.observe().await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !owned.process_observed.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!owned.observed.load(Ordering::Acquire));
        assert_eq!(
            crate::host_resources::extensions::pending_at(
                &binding,
                &crate::config::default_data_dir()
            )
            .unwrap(),
            1
        );
        waiter.abort();
        let _ = waiter.await;
        assert!(!owned.observed.load(Ordering::Acquire));
        send.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), owned.observe())
                .await
                .unwrap()
        );
        assert_eq!(
            crate::host_resources::extensions::pending_at(
                &binding,
                &crate::config::default_data_dir()
            )
            .unwrap(),
            0
        );
        manager.shutdown().await.unwrap();
        let ran = Arc::new(AtomicUsize::new(0));
        let flag = ran.clone();
        assert!(
            manager
                .start_read(invocation, move || {
                    flag.fetch_add(1, Ordering::SeqCst);
                    Ok("late".into())
                })
                .is_err()
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        std::fs::write(root.join("completed"), b"broker-and-observer-complete").unwrap();
    });
}
#[test]
fn isolated_broker_retains_read_and_process_obligations_until_actual_observation() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "extensions::runtime::boundary_tests::owned_broker_child",
        ])
        .env(ROOT, &root)
        .env("HOME", &root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "isolated broker fixture failed");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated broker fixture exceeded bounded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(root.join("completed")).unwrap(),
        b"broker-and-observer-complete"
    );
}
