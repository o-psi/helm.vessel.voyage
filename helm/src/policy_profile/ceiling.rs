#[cfg(target_os = "linux")]
use super::MAX_DOCUMENT;
use super::{CeilingDocument, Error, Result};
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        ffi::CString,
        fs::File,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
    };
    fn checked(file: &File, directory: bool, owner: u32) -> Result<()> {
        let m = file.metadata().map_err(|_| Error::Ceiling)?;
        if m.uid() != owner
            || m.mode() & 0o022 != 0
            || if directory {
                !m.is_dir()
            } else {
                !m.is_file() || m.nlink() != 1 || m.len() > MAX_DOCUMENT as u64
            }
        {
            return Err(Error::Ceiling);
        }
        Ok(())
    }
    fn open(parent: &File, name: &str, directory: bool) -> std::io::Result<File> {
        let name = CString::new(name)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    struct Pin {
        parent: File,
        name: String,
        file: File,
        directory: bool,
    }
    fn verify(root: &File, pins: &[Pin], owner: u32) -> Result<()> {
        checked(root, true, owner)?;
        for pin in pins {
            checked(&pin.file, pin.directory, owner)?;
            let current =
                open(&pin.parent, &pin.name, pin.directory).map_err(|_| Error::Ceiling)?;
            checked(&current, pin.directory, owner)?;
            let a = current.metadata().map_err(|_| Error::Ceiling)?;
            let b = pin.file.metadata().map_err(|_| Error::Ceiling)?;
            if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                return Err(Error::Ceiling);
            }
        }
        Ok(())
    }
    pub(super) fn read(
        root: File,
        parts: &[&str],
        owner: u32,
        after: impl FnOnce() -> Result<()>,
    ) -> Result<Option<CeilingDocument>> {
        checked(&root, true, owner)?;
        let mut parent = root.try_clone().map_err(|_| Error::Ceiling)?;
        let mut pins = Vec::new();
        for (index, name) in parts.iter().enumerate() {
            let directory = index + 1 < parts.len();
            let file = match open(&parent, name, directory) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    verify(&root, &pins, owner)?;
                    return Ok(None);
                }
                Err(_) => return Err(Error::Ceiling),
            };
            checked(&file, directory, owner)?;
            pins.push(Pin {
                parent: parent.try_clone().map_err(|_| Error::Ceiling)?,
                name: (*name).into(),
                file: file.try_clone().map_err(|_| Error::Ceiling)?,
                directory,
            });
            parent = file;
        }
        let before = parent.metadata().map_err(|_| Error::Ceiling)?;
        let mut bytes = Vec::new();
        parent
            .by_ref()
            .take(MAX_DOCUMENT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Ceiling)?;
        after()?;
        verify(&root, &pins, owner)?;
        let last = parent.metadata().map_err(|_| Error::Ceiling)?;
        if (
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            last.len(),
            last.mtime(),
            last.mtime_nsec(),
            last.ctime(),
            last.ctime_nsec(),
        ) {
            return Err(Error::Ceiling);
        }
        CeilingDocument::decode(&bytes).map(Some)
    }
    pub(super) fn load() -> Result<Option<CeilingDocument>> {
        use std::os::unix::fs::OpenOptionsExt;
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| Error::Ceiling)?;
        read(root, &["etc", "helm", "policy-ceiling.toml"], 0, || Ok(()))
    }
}
pub(super) fn load() -> Result<Option<CeilingDocument>> {
    #[cfg(target_os = "linux")]
    {
        linux::load()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::UnsupportedPlatform)
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("helm")).unwrap();
        fs::set_permissions(d.path().join("helm"), fs::Permissions::from_mode(0o755)).unwrap();
        d
    }
    fn read(d: &tempfile::TempDir) -> Result<Option<CeilingDocument>> {
        linux::read(
            fs::File::open(d.path()).unwrap(),
            &["helm", "policy-ceiling.toml"],
            unsafe { libc::geteuid() },
            || Ok(()),
        )
    }
    fn write(d: &tempfile::TempDir) {
        let doc = CeilingDocument {
            schema: 1,
            rules: crate::policy_profile::Builtin::Balanced.document().rules,
        };
        fs::write(
            d.path().join("helm/policy-ceiling.toml"),
            toml::to_string(&doc).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn missing_and_valid_source_are_distinct() {
        let d = fixture();
        assert!(read(&d).unwrap().is_none());
        write(&d);
        assert!(read(&d).unwrap().is_some());
    }
    #[test]
    fn absent_file_under_untrusted_parent_is_not_missing_ceiling() {
        let d = fixture();
        fs::set_permissions(d.path().join("helm"), fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(read(&d).unwrap_err(), Error::Ceiling);
        fs::remove_dir(d.path().join("helm")).unwrap();
        assert!(read(&d).unwrap().is_none());
    }
    #[test]
    fn modes_owner_links_and_nonregular_paths_fail_closed() {
        for kind in 0..8 {
            let d = fixture();
            write(&d);
            let p = d.path().join("helm/policy-ceiling.toml");
            match kind {
                0 => fs::set_permissions(&p, fs::Permissions::from_mode(0o666)).unwrap(),
                1 => fs::set_permissions(d.path().join("helm"), fs::Permissions::from_mode(0o777))
                    .unwrap(),
                2 => {
                    fs::rename(&p, d.path().join("actual")).unwrap();
                    symlink(d.path().join("actual"), &p).unwrap()
                }
                3 => fs::hard_link(&p, d.path().join("extra")).unwrap(),
                4 => {
                    fs::remove_file(&p).unwrap();
                    symlink(d.path().join("missing"), &p).unwrap()
                }
                5 => {
                    fs::remove_file(&p).unwrap();
                    fs::create_dir(&p).unwrap()
                }
                6 => {
                    fs::rename(d.path().join("helm"), d.path().join("real")).unwrap();
                    symlink(d.path().join("real"), d.path().join("helm")).unwrap()
                }
                _ => {
                    assert!(
                        linux::read(
                            fs::File::open(d.path()).unwrap(),
                            &["helm", "policy-ceiling.toml"],
                            unsafe { libc::geteuid() }.wrapping_add(1),
                            || Ok(())
                        )
                        .is_err()
                    );
                    continue;
                }
            }
            assert!(read(&d).is_err(), "case {kind}");
        }
    }
    #[test]
    fn invalid_oversized_and_fifo_fail_without_echo_or_hang() {
        for kind in 0..4 {
            let d = fixture();
            let p = d.path().join("helm/policy-ceiling.toml");
            match kind {
                0 => fs::write(&p, b"SECRET_BAD_DATA").unwrap(),
                1 => fs::write(&p, vec![b'x'; MAX_DOCUMENT + 1]).unwrap(),
                2 => {
                    let p = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0)
                }
                _ => {
                    write(&d);
                    let mut text = fs::read_to_string(&p).unwrap();
                    text.push_str("\nnetwork = 'SECRET_BAD_DATA'\n");
                    fs::write(&p, text).unwrap()
                }
            }
            let e = read(&d).unwrap_err();
            assert_eq!(e, Error::Ceiling);
            assert!(!e.to_string().contains("SECRET"));
        }
    }
    #[test]
    fn replacement_and_in_place_edits_during_read_fail_closed() {
        for kind in 0..3 {
            let d = fixture();
            write(&d);
            let result = linux::read(
                fs::File::open(d.path()).unwrap(),
                &["helm", "policy-ceiling.toml"],
                unsafe { libc::geteuid() },
                || {
                    match kind {
                        0 => {
                            fs::rename(d.path().join("helm"), d.path().join("old")).unwrap();
                            fs::create_dir(d.path().join("helm")).unwrap()
                        }
                        1 => {
                            let p = d.path().join("helm/policy-ceiling.toml");
                            fs::rename(&p, d.path().join("old")).unwrap();
                            write(&d)
                        }
                        _ => fs::write(d.path().join("helm/policy-ceiling.toml"), b"changed")
                            .unwrap(),
                    }
                    Ok(())
                },
            );
            assert_eq!(result.unwrap_err(), Error::Ceiling);
        }
    }
}
