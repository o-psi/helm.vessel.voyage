use super::*;
use std::{ffi::CStr, num::NonZeroU64, path::PathBuf};
use uuid::Uuid;
use voyage_protocol::execution_identity::{AccountContextRef, IdentityRef};
use voyage_protocol::process::{PROCESS_PROTOCOL, ProcessPeerUids, ProcessState};

#[test]
fn bound_registration_cannot_use_the_legacy_same_identity_launcher() {
    let registration = ProcessRegistration {
        executable: None,
        protocol: PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "fixture-secret".into(),
        peer_uids: Some(ProcessPeerUids {
            supervisor: unsafe { libc::geteuid() },
            runtime: 0,
        }),
        workspace: PathBuf::from("/tmp"),
        state: ProcessState::Starting,
        name: None,
    };
    assert!(
        launch(
            Path::new("/does-not-exist"),
            Path::new("/tmp"),
            &registration
        )
        .unwrap_err()
        .to_string()
        .contains("cross-identity launch is unavailable")
    );
}

fn host_identity(name: &str) -> ConfiguredExecutionIdentity {
    let name = CString::new(name).unwrap();
    let mut password = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 16384];
    assert_eq!(
        unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                password.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        },
        0
    );
    assert!(!found.is_null());
    let password = unsafe { password.assume_init() };
    let home = PathBuf::from(std::ffi::OsStr::from_bytes(unsafe {
        CStr::from_ptr(password.pw_dir).to_bytes()
    }));
    let mut groups = [0 as libc::gid_t; 128];
    let mut count = groups.len() as libc::c_int;
    assert!(
        unsafe {
            libc::getgrouplist(
                name.as_ptr(),
                password.pw_gid,
                groups.as_mut_ptr(),
                &mut count,
            )
        } >= 0
    );
    ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Fixture identity".into(),
        user_name: name.to_str().unwrap().into(),
        uid: password.pw_uid,
        gid: password.pw_gid,
        supplementary_groups: groups[..count as usize]
            .iter()
            .copied()
            .filter(|group| *group != password.pw_gid)
            .collect(),
        home,
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: if password.pw_uid == 0 {
            AuthorityClass::Administrator
        } else {
            AuthorityClass::Ordinary
        },
        enabled: true,
    }
}

#[test]
fn host_account_changes_refuse_a_saved_execution_identity() {
    let user = std::env::var("USER").unwrap();
    let mut identity = host_identity(&user);
    validate_identity(&identity).unwrap();
    identity.uid = identity.uid.wrapping_add(1);
    assert!(validate_identity(&identity).is_err());
    identity.uid = unsafe { libc::geteuid() };
    identity.supplementary_groups.push(u32::MAX);
    assert!(validate_identity(&identity).is_err());
}

#[test]
#[ignore = "requires an explicitly designated disposable native-root fixture"]
fn native_root_launch_drops_to_ordinary_user_without_regain() {
    assert_eq!(
        std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
        Ok("1")
    );
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let identity = host_identity("voyageordinary");
    let mut command = Command::new("/usr/bin/python3");
    command.arg("-c").arg(
        "import json,os,pathlib; s=pathlib.Path('/proc/self/status').read_text();\ntry: os.setuid(0); regain=True\nexcept OSError: regain=False\nprint(json.dumps({'uid':os.geteuid(),'gid':os.getegid(),'groups':os.getgroups(),'regain':regain,'status':{k:v for k,v in (line.split(':',1) for line in s.splitlines() if line.startswith(('Uid:','Gid:','CapEff:','CapPrm:','CapAmb:','NoNewPrivs:')))}}))",
    );
    configure_identity(&mut command, &identity).unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    let observed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(observed["uid"], identity.uid);
    assert_eq!(observed["gid"], identity.gid);
    assert_eq!(
        observed["groups"],
        serde_json::json!(identity.supplementary_groups)
    );
    assert_eq!(observed["regain"], false);
    for name in ["Uid", "Gid"] {
        let value = if name == "Uid" {
            identity.uid
        } else {
            identity.gid
        };
        assert_eq!(
            observed["status"][name]
                .as_str()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            vec![value.to_string(); 4]
        );
    }
    assert_eq!(
        observed["status"]["NoNewPrivs"].as_str().unwrap().trim(),
        "1"
    );
    for name in ["CapEff", "CapPrm", "CapAmb"] {
        assert_eq!(
            observed["status"][name].as_str().unwrap().trim(),
            "0000000000000000"
        );
    }
}
