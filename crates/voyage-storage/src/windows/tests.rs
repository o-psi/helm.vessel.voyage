use super::*;
use std::io::{ErrorKind, Read};

#[test]
fn private_roundtrip_and_replacement_have_verified_permissions() {
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    directory.publish("client.json", b"first").unwrap();
    directory.publish("client.json", b"second").unwrap();
    let mut file = directory.open_file("client.json", false).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"second");
    drop(directory);
    PrivateDirectory::open(&root.path().join("private")).unwrap();
}

#[test]
fn independent_directories_are_allowed_but_client_lock_is_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    let a = PrivateDirectory::open(&path).unwrap();
    let b = PrivateDirectory::open(&path).unwrap();
    let lock = a.lock("client.lock").unwrap();
    assert_eq!(
        b.lock("client.lock").unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    drop(lock);
    b.lock("client.lock").unwrap();
}

#[test]
fn rejects_linked_files_alternate_streams_and_replaced_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    let directory = PrivateDirectory::open(&path).unwrap();
    directory.publish("client.json", b"protected").unwrap();
    std::fs::hard_link(path.join("client.json"), path.join("alias")).unwrap();
    assert!(directory.open_file("client.json", false).is_err());
    assert!(directory.open_file("client.json:stream", true).is_err());
    assert!(directory.open_file("../escape", true).is_err());
    assert!(std::fs::rename(&path, root.path().join("moved")).is_err());
}

#[test]
fn failed_publication_keeps_original_state() {
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    directory.publish("client.json", b"original").unwrap();
    let held = directory.open_file("client.json", false).unwrap();
    assert!(
        directory
            .publish("client.json", b"must not replace")
            .is_err()
    );
    drop(held);
    assert_eq!(
        std::fs::read(directory.path().join("client.json")).unwrap(),
        b"original"
    );
}

fn change_dacl(path: &Path, broad: bool, inherit: bool) {
    let file = open_handle(path, WRITE_DAC, OPEN_EXISTING, path.is_dir(), None).unwrap();
    let sid = current_sid().unwrap();
    let sd = descriptor(&sid).unwrap();
    unsafe {
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        assert_ne!(
            GetSecurityDescriptorDacl(sd.0, &mut present, &mut acl, &mut defaulted),
            0
        );
        let mut extra_sd = null_mut();
        if broad {
            // A null DACL grants everyone access, unlike an empty DACL.
            acl = null_mut();
        } else if !inherit {
            let mut text = null_mut();
            assert_ne!(
                ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut text),
                0
            );
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let sid_text = String::from_utf16(std::slice::from_raw_parts(text, length)).unwrap();
            LocalFree(text.cast());
            let sddl = wide(Path::new(&format!("D:P(A;;FA;;;{sid_text})"))).unwrap();
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut extra_sd,
                    null_mut()
                ),
                0
            );
            assert_ne!(
                GetSecurityDescriptorDacl(extra_sd, &mut present, &mut acl, &mut defaulted),
                0
            );
        }
        assert_eq!(
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null_mut()
            ),
            0
        );
        if !extra_sd.is_null() {
            LocalFree(extra_sd);
        }
    }
}

#[test]
fn rejects_null_dacl_and_noninheriting_directory_without_repairing_it() {
    for broad in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        drop(PrivateDirectory::open(&path).unwrap());
        change_dacl(&path, broad, false);
        assert!(PrivateDirectory::open(&path).is_err());
        // No automatic chmod/ACL repair: it remains invalid on a second open.
        assert!(PrivateDirectory::open(&path).is_err());
        change_dacl(&path, false, true);
        PrivateDirectory::open(&path).unwrap();
    }
}

#[test]
fn rejects_broad_existing_file_before_reading_or_replacing_secrets() {
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    directory.publish("client.json", b"original").unwrap();
    change_dacl(&directory.path().join("client.json"), true, false);
    assert!(directory.open_file("client.json", false).is_err());
    assert!(directory.publish("client.json", b"replacement").is_err());
    assert_eq!(
        std::fs::read(directory.path().join("client.json")).unwrap(),
        b"original"
    );
    change_dacl(&directory.path().join("client.json"), false, true);
}

#[test]
fn junction_ancestor_is_rejected_before_creating_children() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("actual");
    drop(PrivateDirectory::open(&actual).unwrap());
    let alias = root.path().join("alias");
    let result = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&actual)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "could not create NTFS junction fixture"
    );
    assert!(PrivateDirectory::open(&alias.join("new-directory")).is_err());
    assert!(!actual.join("new-directory").exists());
    std::fs::remove_dir(&alias).unwrap();
}

#[test]
fn child_process_lock_fixture() {
    let Some(path) = std::env::var_os("VOYAGE_STORAGE_TEST_LOCK_DIRECTORY") else {
        return;
    };
    let directory = PrivateDirectory::open(Path::new(&path)).unwrap();
    if std::env::var("VOYAGE_STORAGE_TEST_LOCK_ROLE").unwrap() == "busy" {
        assert_eq!(
            directory.lock("client.lock").unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
        return;
    }
    let _lock = directory.lock("client.lock").unwrap();
    directory.publish("ready", b"locked").unwrap();
    loop {
        std::thread::park_timeout(std::time::Duration::from_secs(60));
    }
}

#[test]
fn real_process_lock_exclusion_and_abrupt_exit_release() {
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    struct KillOnDrop(Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    let command = |role: &str| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "windows::tests::child_process_lock_fixture",
                "--nocapture",
            ])
            .env("VOYAGE_STORAGE_TEST_LOCK_DIRECTORY", directory.path())
            .env("VOYAGE_STORAGE_TEST_LOCK_ROLE", role)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    let lock = directory.lock("client.lock").unwrap();
    let mut flags = 0;
    assert_ne!(
        unsafe {
            windows_sys::Win32::Foundation::GetHandleInformation(lock.as_raw_handle(), &mut flags)
        },
        0
    );
    assert_eq!(
        flags & windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT,
        0
    );
    assert!(command("busy").status().unwrap().success());
    drop(lock);
    let mut child = KillOnDrop(command("hold").spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.path().join("ready").is_file() {
        assert!(Instant::now() < deadline, "child did not acquire lock");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before acquiring lock"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        directory.lock("client.lock").unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    directory.lock("client.lock").unwrap();
}
