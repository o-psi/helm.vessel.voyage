use super::*;
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixListener,
    },
    path::PathBuf,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        // /tmp has a writable ancestor by design and must not be an accepted
        // privileged anchor. Keep this ordinary-UID fixture under the checkout.
        let mut nonce = [0u8; 16];
        SystemRandom::new().fill(&mut nonce).unwrap();
        let path = std::env::current_dir().unwrap().join(format!(
            ".control-test-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> RootDirectory {
        RootDirectory::open_owned(&self.0, unsafe { libc::geteuid() }).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

#[test]
fn protected_control_admission_is_durable_exclusive_and_bounded() {
    let fixture = Fixture::new();
    let directory = fixture.open();
    directory
        .publish_new(OsStr::new("review.json"), b"first", 32)
        .unwrap();
    assert_eq!(
        directory.read(OsStr::new("review.json"), 32).unwrap(),
        b"first"
    );
    assert!(
        directory
            .publish_new(OsStr::new("review.json"), b"second", 32)
            .is_err()
    );
    assert_eq!(
        directory.read(OsStr::new("review.json"), 32).unwrap(),
        b"first"
    );
    assert!(directory.read(OsStr::new("review.json"), 4).is_err());
    assert!(
        directory
            .publish_new(OsStr::new("oversize"), b"abc", 2)
            .is_err()
    );
    assert!(!fixture.0.join("oversize").exists());
    let lock = directory.lock(OsStr::new("lock")).unwrap();
    assert!(fixture.open().lock(OsStr::new("lock")).is_err());
    drop(lock);
    directory.lock(OsStr::new("lock")).unwrap();
    let child = directory.create_child(OsStr::new("catalogue")).unwrap();
    child
        .publish_new(OsStr::new("entry"), b"bound", 16)
        .unwrap();
    assert_eq!(
        directory
            .child(OsStr::new("catalogue"))
            .unwrap()
            .read(OsStr::new("entry"), 16)
            .unwrap(),
        b"bound"
    );
    assert!(!fs::read_dir(&fixture.0).unwrap().any(|f| {
        f.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".pending-")
    }));
}

#[test]
fn protected_control_rejects_names_links_special_files_and_permission_changes() {
    let fixture = Fixture::new();
    let directory = fixture.open();
    for bad in ["..", ".", "/absolute", "sub/name", "", "nul\0name"] {
        assert!(directory.read(OsStr::new(bad), 100).is_err());
        assert!(
            directory
                .publish_new(OsStr::new(bad), b"value", 100)
                .is_err()
        );
        assert!(directory.create_child(OsStr::new(bad)).is_err());
    }
    directory
        .publish_new(OsStr::new("record"), b"secret", 100)
        .unwrap();
    std::os::unix::fs::symlink("record", fixture.0.join("alias")).unwrap();
    std::os::unix::fs::symlink("missing", fixture.0.join("dangling")).unwrap();
    assert!(directory.read(OsStr::new("alias"), 100).is_err());
    assert!(
        directory
            .publish_new(OsStr::new("dangling"), b"no", 100)
            .is_err()
    );
    fs::hard_link(fixture.0.join("record"), fixture.0.join("hardlink")).unwrap();
    assert!(directory.read(OsStr::new("record"), 100).is_err());
    fs::remove_file(fixture.0.join("hardlink")).unwrap();
    mode(&fixture.0.join("record"), 0o644);
    assert!(directory.read(OsStr::new("record"), 100).is_err());
    mode(&fixture.0.join("record"), 0o600);
    let fifo = CString::new(fixture.0.join("fifo").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(directory.read(OsStr::new("fifo"), 100).is_err());
    let _socket = UnixListener::bind(format!(
        "/proc/self/fd/{}/socket",
        directory.directory.as_raw_fd()
    ))
    .unwrap();
    assert!(directory.read(OsStr::new("socket"), 100).is_err());
    mode(&fixture.0, 0o750);
    assert!(directory.read(OsStr::new("record"), 100).is_err());
    assert!(
        directory
            .publish_new(OsStr::new("after-change"), b"no", 100)
            .is_err()
    );
    mode(&fixture.0, 0o700);
}

#[test]
fn protected_control_rejects_unsafe_ancestors_and_retains_pinned_directory() {
    let fixture = Fixture::new();
    let directory = fixture.open();
    directory.create_child(OsStr::new("private")).unwrap();
    let path = fixture.0.join("private");
    mode(&fixture.0, 0o702);
    assert!(RootDirectory::open_owned(&path, unsafe { libc::geteuid() }).is_err());
    mode(&fixture.0, 0o700);
    std::os::unix::fs::symlink("private", fixture.0.join("alias")).unwrap();
    assert!(
        RootDirectory::open_owned(&fixture.0.join("alias"), unsafe { libc::geteuid() }).is_err()
    );
    assert!(RootDirectory::open_owned(Path::new("relative"), unsafe { libc::geteuid() }).is_err());
    if unsafe { libc::geteuid() } != 0 {
        assert!(RootDirectory::open(&fixture.0).is_err());
    }
    let pinned = directory.child(OsStr::new("private")).unwrap();
    fs::rename(&path, fixture.0.join("original")).unwrap();
    directory.create_child(OsStr::new("private")).unwrap();
    pinned
        .publish_new(OsStr::new("receipt"), b"exact", 100)
        .unwrap();
    assert_eq!(
        directory
            .child(OsStr::new("original"))
            .unwrap()
            .read(OsStr::new("receipt"), 100)
            .unwrap(),
        b"exact"
    );
    assert!(!path.join("receipt").exists());
}

#[test]
fn runtime_parent_fences_names_ownership_and_child_control() {
    let fixture = Fixture::new();
    mode(&fixture.0, 0o711);
    let owner = unsafe { libc::geteuid() };
    let group = unsafe { libc::getegid() };
    let runtime = RuntimeRoot::open_owned(&fixture.0, owner).unwrap();
    let session = OsStr::new("641458d0-9562-46e6-b617-a34d1c0e53b7");
    let directory = runtime.create_session(session, owner, group).unwrap();
    assert_eq!(directory.metadata().unwrap().mode() & 0o777, 0o700);
    assert!(runtime.create_session(session, owner, group).is_ok());
    assert!(
        runtime
            .session(session, owner.wrapping_add(1), group)
            .is_err()
    );
    for name in [
        "..",
        "other",
        "00000000-0000-0000-0000-000000000000",
        "641458D0-9562-46e6-b617-a34d1c0e53b7",
    ] {
        assert!(
            runtime
                .create_session(OsStr::new(name), owner, group)
                .is_err()
        );
    }
    std::os::unix::fs::symlink(
        session,
        fixture.0.join("641458d0-9562-46e6-b617-a34d1c0e53b8"),
    )
    .unwrap();
    assert!(
        runtime
            .session(
                OsStr::new("641458d0-9562-46e6-b617-a34d1c0e53b8"),
                owner,
                group
            )
            .is_err()
    );
    mode(&fixture.0, 0o713);
    assert!(runtime.session(session, owner, group).is_err());
    assert!(RuntimeRoot::open_owned(&fixture.0, owner).is_err());
    mode(&fixture.0, 0o711);
    mode(&fixture.0.join(session), 0o777);
    assert!(runtime.session(session, owner, group).is_err());
    assert!(runtime.create_session(session, owner, group).is_err());
    if owner != 0 {
        assert!(RuntimeRoot::open(&fixture.0).is_err());
    }
}

#[test]
#[ignore = "requires an explicitly disposable native Linux root fixture"]
fn native_root_records_exclude_ordinary_identity() {
    assert_eq!(
        std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
        Ok("1")
    );
    if let Some(path) = std::env::var_os("VOYAGE_CONTROL_CHILD_PROBE") {
        assert_ne!(unsafe { libc::geteuid() }, 0);
        let path = Path::new(&path);
        assert!(RootDirectory::open(path).is_err());
        assert!(fs::read(path.join("authority")).is_err());
        assert!(fs::write(path.join("forged"), b"forged").is_err());
        return;
    }
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "native fixture needs real root authority"
    );
    let fixture = Fixture::new();
    let directory = RootDirectory::open(&fixture.0).unwrap();
    directory
        .publish_new(OsStr::new("authority"), b"root-owned", 100)
        .unwrap();
    let lock = directory.lock(OsStr::new("lock")).unwrap();
    assert_ne!(
        unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    let uid = std::process::Command::new("id")
        .args(["-u", "voyageordinary"])
        .output()
        .unwrap();
    assert!(uid.status.success());
    let uid: u32 = String::from_utf8(uid.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_ne!(uid, 0);
    let group = std::process::Command::new("id")
        .args(["-g", "voyageordinary"])
        .output()
        .unwrap();
    assert!(group.status.success());
    let gid: u32 = String::from_utf8(group.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut command = std::process::Command::new("setpriv");
    command.args([
        "--reuid",
        &uid.to_string(),
        "--regid",
        &gid.to_string(),
        "--clear-groups",
        "--no-new-privs",
    ]);
    // Local development binaries may need their matching loader when copied into
    // a disposable distribution fixture. Shipping binaries use that distro ABI.
    if let Some(loader) = std::env::var_os("VOYAGE_TEST_LINUX_LOADER") {
        command
            .arg(loader)
            .arg("--library-path")
            .arg(std::env::var_os("VOYAGE_TEST_LINUX_LIBS").unwrap());
    }
    let output = command
        .arg(
            std::env::var_os("VOYAGE_TEST_EXECUTABLE")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_exe().unwrap()),
        )
        .args([
            "--ignored",
            "--exact",
            "protected_linux::tests::native_root_records_exclude_ordinary_identity",
            "--nocapture",
        ])
        .env("VOYAGE_CONTROL_CHILD_PROBE", &fixture.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    assert_eq!(
        directory.read(OsStr::new("authority"), 100).unwrap(),
        b"root-owned"
    );
    let record = fs::File::open(fixture.0.join("authority")).unwrap();
    assert_eq!(unsafe { libc::fchown(record.as_raw_fd(), uid, gid) }, 0);
    assert!(directory.read(OsStr::new("authority"), 100).is_err());
}
