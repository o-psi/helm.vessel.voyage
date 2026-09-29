use super::*;

#[test]
fn ordinary_process_cannot_enter_privileged_guardian() {
    if unsafe { libc::geteuid() } != 0 {
        assert!(
            run(Args {
                directory: "/not-a-control-root".into(),
                session: Uuid::new_v4(),
                incarnation: Uuid::new_v4()
            })
            .unwrap_err()
            .to_string()
            .contains("requires root")
        );
    }
}

#[tokio::test]
#[ignore = "requires an explicitly disposable native-root fixture and Vessel/Voyage executables"]
async fn native_guardian_attests_pipe_launch_and_proves_descendant_cleanup() {
    use crate::process::{database, launch, registry, routing};
    use std::{
        fs,
        num::NonZeroU64,
        os::fd::{FromRawFd, OwnedFd},
        os::unix::fs::{MetadataExt, PermissionsExt},
    };
    use voyage_protocol::{execution_identity::*, process::*};
    assert_eq!(
        std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
        Ok("1")
    );
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let vessel = PathBuf::from(std::env::var_os("VOYAGE_TEST_VESSEL_EXECUTABLE").unwrap());
    let voyage = PathBuf::from(std::env::var_os("VOYAGE_TEST_CATALOGUE_EXECUTABLE").unwrap());
    launch::protected_binary(&vessel).unwrap();
    launch::protected_binary(&voyage).unwrap();
    let id = |flag| -> u32 {
        String::from_utf8(
            Command::new("/usr/bin/id")
                .args([flag, "voyageordinary"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .parse()
        .unwrap()
    };
    let uid = id("-u");
    let gid = id("-g");
    assert_ne!(uid, 0);
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(PathBuf::from(format!(
        "/opt/vg-{}",
        &Uuid::new_v4().simple().to_string()[..8]
    )));
    fs::create_dir(&fixture.0).unwrap();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).unwrap();
    let control = fixture.0.join("control");
    let runtime = fixture.0.join("runtime");
    let workspace = fixture.0.join("workspace");
    registry::private_directory(&control).unwrap();
    registry::private_directory(&control.join("sessions")).unwrap();
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o711)).unwrap();
    registry::private_directory(&workspace).unwrap();
    std::os::unix::fs::chown(&workspace, Some(uid), Some(gid)).unwrap();
    database::initialize(&control).await.unwrap();
    RootDirectory::open(&control)
        .unwrap()
        .publish_new(
            "runtime-layout.json".as_ref(),
            &serde_json::to_vec(&serde_json::json!({"version":1,"runtime_root":runtime})).unwrap(),
            4096,
        )
        .unwrap();
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Guardian ordinary fixture".into(),
        user_name: "voyageordinary".into(),
        uid,
        gid,
        supplementary_groups: vec![],
        home: "/home/voyageordinary".into(),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Ordinary,
        enabled: true,
    };
    database::store_identity(&control, &identity).await.unwrap();
    let ordinary = |args: &[&str]| -> Vec<u8> {
        let mut command = Command::new(&vessel);
        command.args(["auth", "accounts"]).args(args);
        launch::configure_identity(&mut command, &identity).unwrap();
        command.env("VOYAGE_GUARDIAN_FIXTURE_KEY", "synthetic-native-fixture");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let label = format!("guardian-{}", Uuid::new_v4());
    let connection: serde_json::Value = serde_json::from_slice(&ordinary(&[
        "connect",
        "--label",
        &label,
        "--endpoint",
        "http://127.0.0.1:9/v1",
        "--transports",
        "openai-responses",
    ]))
    .unwrap();
    let account: serde_json::Value = serde_json::from_slice(&ordinary(&[
        "add",
        "--connection",
        connection["id"].as_str().unwrap(),
        "--account",
        &label,
        "--env",
        "VOYAGE_GUARDIAN_FIXTURE_KEY",
    ]))
    .unwrap();
    let config = workspace.join("launch.json");
    fs::write(&config,serde_json::to_vec(&serde_json::json!({"version":1,"workspace":workspace,"config":{"provider":"openai-responses","model":"fixture-model","api_key_required":false,"base_url":"http://127.0.0.1:9/v1","account":{"account_id":account["id"],"connection_id":connection["id"],"identity_generation":account["identity_generation"],"connection_revision":connection["revision"],"transport":"openai_responses"},"access":"read-only","provider_retry_attempts":1,"context_window":0,"command_timeout_secs":2},"explicit":{"access":"read-only"},"selection":null,"confirmation":null})).unwrap()).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::chown(&config, Some(uid), Some(gid)).unwrap();
    let registration = |binary: PathBuf| ProcessRegistration {
        protocol: PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: Some(config.clone()),
        token: "f".repeat(64),
        peer_uids: Some(ProcessPeerUids {
            supervisor: 0,
            runtime: uid,
        }),
        workspace: workspace.clone(),
        state: ProcessState::Starting,
        name: None,
        executable: Some(binary),
    };
    let binding = |r: &ProcessRegistration| ExecutionBinding {
        session_id: r.session_id,
        incarnation: r.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: r.peer_uids.clone().unwrap(),
        administrator_grant_id: None,
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
    };
    let spawn = |r: &ProcessRegistration| {
        let mut command = Command::new(&vessel);
        command
            .arg("guard-bound")
            .arg("--directory")
            .arg(&control)
            .arg("--session")
            .arg(r.session_id.to_string())
            .arg("--incarnation")
            .arg(r.incarnation.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(control.join(format!("guardian-{}.stderr", r.incarnation)))
                    .unwrap(),
            ));
        // Seed a real inheritable root capability; the ordinary child must
        // clear it, rather than merely inherit the fixture's usual empty set.
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                #[repr(C)]
                struct Header {
                    version: u32,
                    pid: i32,
                }
                #[repr(C)]
                #[derive(Clone, Copy)]
                struct Data {
                    effective: u32,
                    permitted: u32,
                    inheritable: u32,
                }
                let header = Header {
                    version: 0x2008_0522,
                    pid: 0,
                };
                let mut data = [Data {
                    effective: 0,
                    permitted: 0,
                    inheritable: 0,
                }; 2];
                if libc::syscall(libc::SYS_capget, &header, data.as_mut_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                data[0].inheritable |= 1;
                if libc::syscall(libc::SYS_capset, &header, data.as_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn().unwrap()
    };
    async fn wait(child: &mut std::process::Child) -> std::process::ExitStatus {
        for _ in 0..200 {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("guardian did not finish");
    }
    let r = registration(voyage.clone());
    database::admit_with_binding(&control, &r, vec![1], Some(&binding(&r)))
        .await
        .unwrap();
    let directory = runtime.join(r.session_id.to_string());
    let mut guardian = spawn(&r);
    let mut healthy = false;
    for _ in 0..100 {
        if let Ok(response) = routing::forward(&directory, &r, RuntimeCommand::Health).await
            && response.error.is_none()
        {
            healthy = true;
            break;
        }
        if guardian.try_wait().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(
        healthy,
        "actual bound Voyage did not become healthy: guardian={}, runtime={}",
        fs::read_to_string(control.join(format!("guardian-{}.stderr", r.incarnation)))
            .unwrap_or_default(),
        fs::read_to_string(directory.join("fixture-runtime.stderr")).unwrap_or_default()
    );
    assert!(
        fs::read_to_string(format!("/proc/{}/status", guardian.id()))
            .unwrap()
            .lines()
            .any(|line| line.starts_with("CapInh:") && line.ends_with("0000000000000001"))
    );
    let observed: ObservedExecution = serde_json::from_slice(
        &RootDirectory::open(&control)
            .unwrap()
            .child("guardians".as_ref())
            .unwrap()
            .child(r.session_id.to_string().as_ref())
            .unwrap()
            .child(r.incarnation.to_string().as_ref())
            .unwrap()
            .read("observed.json".as_ref(), 8192)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed.uid, uid);
    assert_eq!(observed.gid, gid);
    assert!(observed.supplementary_groups.is_empty());
    assert_eq!(
        (
            observed.effective_capabilities,
            observed.permitted_capabilities,
            observed.inheritable_capabilities,
            observed.ambient_capabilities
        ),
        (0, 0, 0, 0)
    );
    assert!(observed.no_new_privileges);
    assert!(observed.process_start_ticks > 0);
    assert_eq!(
        fs::metadata(directory.join("registration.json"))
            .unwrap()
            .uid(),
        uid
    );
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).is_err());
    assert!(!wait(&mut spawn(&r)).await.success());
    // Tampering with the runtime projection and stop marker cannot change either
    // the in-memory launch authority or the protected cleanup decision.
    let mut attack = Command::new("/usr/bin/python3");
    attack.arg("-c").arg("import pathlib,sys; p=pathlib.Path(sys.argv[1]); (p/'registration.json').write_text('{}'); (p/'stopped.json').write_text('{\"cleanup_observed\":true}');\ntry: pathlib.Path(sys.argv[2]).read_bytes(); raise AssertionError('control leaked')\nexcept PermissionError: pass").arg(&directory).arg(control.join("catalogue.sqlite3"));
    launch::configure_identity(&mut attack, &identity).unwrap();
    assert!(attack.status().unwrap().success());
    assert!(
        routing::forward(&directory, &r, RuntimeCommand::Health)
            .await
            .unwrap()
            .error
            .is_none()
    );
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).is_err());
    assert!(
        routing::forward(&directory, &r, RuntimeCommand::Stop)
            .await
            .unwrap()
            .error
            .is_none()
    );
    assert!(wait(&mut guardian).await.success());
    assert!(cleanup_observed(&control, r.session_id, r.incarnation).unwrap());
    assert!(!wait(&mut spawn(&r)).await.success());
    // An ordinary pipe cannot impersonate the privileged launch handoff.
    let mut wrong = Command::new("/usr/bin/python3");
    wrong.arg("-c").arg("import os,sys,struct; r,w=os.pipe(); os.write(w,struct.pack('!I',0)); os.close(w); os.dup2(r,0); os.close(r); os.execv(sys.argv[1],sys.argv[1:])")
        .arg(&voyage).arg("serve-bound").arg("--directory").arg(&directory)
        .arg("--session").arg(r.session_id.to_string()).arg("--incarnation").arg(r.incarnation.to_string())
        .arg("--workspace").arg(&workspace);
    launch::configure_identity(&mut wrong, &identity).unwrap();
    let refused = wrong.output().unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("root-owned private pipe"));
    // A genuine root pipe still requires the exact execution identity. Wrong
    // identity is rejected before interpreting child-owned configuration.
    let mut wrong_identity = Command::new(&voyage);
    wrong_identity
        .arg("serve-bound")
        .arg("--directory")
        .arg(&directory)
        .arg("--session")
        .arg(r.session_id.to_string())
        .arg("--incarnation")
        .arg(r.incarnation.to_string())
        .arg("--workspace")
        .arg(&workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    launch::configure_identity(&mut wrong_identity, &identity).unwrap();
    let mut wrong_child = wrong_identity.spawn().unwrap();
    let mut invalid = r.clone();
    invalid.peer_uids.as_mut().unwrap().runtime += 1;
    send_registration(wrong_child.stdin.take().unwrap(), &invalid).unwrap();
    assert!(!wait(&mut wrong_child).await.success());
    let mut stale_incarnation = r.clone();
    stale_incarnation.incarnation = Uuid::new_v4();
    assert!(!wait(&mut spawn(&stale_incarnation)).await.success());
    // A controlled runtime fixture leaves a double-forked setsid descendant.
    // The guardian must kill and reap it through kernel child ownership.
    let fake = fixture.0.join("runtime-fixture");
    fs::write(
        &fake,
        r#"#!/usr/bin/python3
import json,os,pathlib,struct,sys,time
n=struct.unpack('!I',sys.stdin.buffer.read(4))[0]
r=json.loads(sys.stdin.buffer.read(n)); p=pathlib.Path(sys.argv[sys.argv.index('--directory')+1])
if '--hold' in pathlib.Path(__file__).read_text().splitlines()[1:2]:
 (p/'fixture-ready').write_text(str(os.getpid())); time.sleep(30); sys.exit(0)
pid=os.fork()
if pid==0:
 os.setsid()
 if os.fork(): os._exit(0)
 (p/'escaped-pid').write_text(str(os.getpid()))
 time.sleep(30); os._exit(0)
for _ in range(100):
 if (p/'escaped-pid').exists() and (p/'escaped-pid').read_text().isdigit(): break
 time.sleep(.01)
os._exit(0)
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let escaped = registration(fake.clone());
    database::admit_with_binding(&control, &escaped, vec![2], Some(&binding(&escaped)))
        .await
        .unwrap();
    assert!(wait(&mut spawn(&escaped)).await.success());
    let pid = fs::read_to_string(
        runtime
            .join(escaped.session_id.to_string())
            .join("escaped-pid"),
    )
    .unwrap();
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(cleanup_observed(&control, escaped.session_id, escaped.incarnation).unwrap());
    // Kill a live guardian. Pin its exact kernel child before the kill so test
    // teardown never targets a recycled numeric PID. Runtime markers cannot
    // manufacture a completion record after guardian death.
    let source = fs::read_to_string(&fake)
        .unwrap()
        .replacen("#!/usr/bin/python3\n", "#!/usr/bin/python3\n--hold\n", 1)
        .replace("--hold\nimport", "# --hold\nimport")
        .replace(
            "if '--hold' in pathlib.Path(__file__).read_text().splitlines()[1:2]:",
            "if True:",
        );
    fs::write(&fake, source).unwrap();
    let abandoned = registration(fake);
    database::admit_with_binding(&control, &abandoned, vec![3], Some(&binding(&abandoned)))
        .await
        .unwrap();
    let mut guard = spawn(&abandoned);
    let abandoned_directory = runtime.join(abandoned.session_id.to_string());
    for _ in 0..100 {
        if abandoned_directory.join("fixture-ready").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(abandoned_directory.join("fixture-ready").exists());
    let children =
        fs::read_to_string(format!("/proc/{}/task/{}/children", guard.id(), guard.id())).unwrap();
    let pid: u32 = children.trim().parse().unwrap();
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(raw >= 0);
    let child = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    guard.kill().unwrap();
    guard.wait().unwrap();
    fs::write(
        abandoned_directory.join("stopped.json"),
        b"{\"cleanup_observed\":true}",
    )
    .unwrap();
    assert!(cleanup_observed(&control, abandoned.session_id, abandoned.incarnation).is_err());
    assert!(!wait(&mut spawn(&abandoned)).await.success());
    assert_eq!(
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                child.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        },
        0
    );
    for _ in 0..100 {
        if !PathBuf::from(format!("/proc/{pid}")).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(cleanup_observed(&control, abandoned.session_id, abandoned.incarnation).is_err());
    // A newly admitted launch cannot survive a configured-identity revision
    // change, even when its saved numeric UID remains the same.
    let stale = registration(voyage.clone());
    database::admit_with_binding(&control, &stale, vec![4], Some(&binding(&stale)))
        .await
        .unwrap();
    let mut revised = identity.clone();
    revised.identity.revision = NonZeroU64::new(2).unwrap();
    database::store_identity(&control, &revised).await.unwrap();
    assert!(!wait(&mut spawn(&stale)).await.success());
    assert!(!runtime.join(stale.session_id.to_string()).exists());
    let _ = ordinary(&[
        "logout",
        "--account",
        account["id"].as_str().unwrap(),
        "--remove",
    ]);
}
