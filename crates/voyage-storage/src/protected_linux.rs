//! Descriptor-relative Linux control and runtime-directory boundaries. These
//! protect against ordinary host identities, not an unrestricted administrator.
//! Runtime journals must never be parsed by a root control reader.
use anyhow::{Context, Result, ensure};
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    ffi::{CString, OsStr},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

/// An already checked control directory. No path resolution through an untrusted
/// ancestor occurs after opening it. Callers must serialize logical transactions
/// with `lock`; atomic file publication is not a multi-record transaction.
pub struct RootDirectory {
    directory: File,
    owner: u32,
}

/// A root-controlled, execute-only parent for per-voyage runtime directories.
/// Children own their own journals and IPC; the parent contains no root secrets.
/// A child may change contents of its directory but cannot replace its entry in
/// this parent. Never use child-owned data as supervisor authority.
pub struct RuntimeRoot {
    directory: File,
    owner: u32,
}

fn name(value: &OsStr) -> Result<CString> {
    let path = Path::new(value);
    ensure!(
        matches!(path.components().next(), Some(Component::Normal(_)))
            && path.components().count() == 1
            && !value.as_bytes().contains(&b'/'),
        "control name must be one path component"
    );
    Ok(CString::new(value.as_bytes())?)
}
fn session_name(value: &OsStr) -> Result<CString> {
    let text = value
        .to_str()
        .context("runtime session name must be UTF-8")?;
    let bytes = text.as_bytes();
    ensure!(
        bytes.len() == 36
            && bytes
                .iter()
                .any(|byte| byte.is_ascii_hexdigit() && *byte != b'0')
            && bytes.iter().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    *byte == b'-'
                } else {
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
                }
            }),
        "runtime session name must be a lowercase UUID"
    );
    name(value)
}
fn file_at(parent: &File, name: &CString, flags: i32, mode: libc::mode_t) -> Result<File> {
    // SAFETY: parent and CString remain live; ownership of a successful descriptor
    // is transferred exactly once. CLOEXEC prevents control handles leaking to tools.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn check_directory(directory: &File, owner: u32, private: bool) -> Result<()> {
    let m = directory.metadata()?;
    ensure!(
        m.is_dir() && (m.uid() == 0 || m.uid() == owner) && m.mode() & 0o022 == 0,
        "control directory has an unsafe owner or writable permissions"
    );
    if private {
        ensure!(
            m.uid() == owner && m.mode() & 0o077 == 0,
            "control directory must be private and owned by its authority"
        );
    }
    Ok(())
}
fn check_file(file: &File, owner: u32, limit: u64) -> Result<()> {
    let m = file.metadata()?;
    ensure!(
        m.is_file()
            && m.uid() == owner
            && m.mode() & 0o077 == 0
            && m.nlink() == 1
            && m.len() <= limit,
        "unsafe or oversized control record"
    );
    Ok(())
}

impl RuntimeRoot {
    /// Opens an administrator-provisioned root-owned parent with mode 0711.
    /// Every ancestor is opened without following symlinks and must be owned by
    /// root and unwritable by other identities.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_owned(path, 0)
    }
    fn open_owned(path: &Path, owner: u32) -> Result<Self> {
        ensure!(path.is_absolute(), "runtime root must be absolute");
        let mut directory = File::open("/")?;
        check_directory(&directory, owner, false)?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(value) => {
                    directory = file_at(
                        &directory,
                        &name(value)?,
                        libc::O_RDONLY | libc::O_DIRECTORY,
                        0,
                    )?;
                    check_directory(&directory, owner, false)?;
                }
                _ => anyhow::bail!("runtime root path must not contain relative components"),
            }
        }
        let metadata = directory.metadata()?;
        ensure!(
            metadata.uid() == owner && metadata.mode() & 0o777 == 0o711,
            "runtime parent must be authority-owned with mode 0711"
        );
        Ok(Self { directory, owner })
    }
    fn current(&self) -> Result<()> {
        let metadata = self.directory.metadata()?;
        ensure!(
            metadata.is_dir() && metadata.uid() == self.owner && metadata.mode() & 0o777 == 0o711,
            "runtime parent authority or mode changed"
        );
        Ok(())
    }
    /// Open a named runtime directory under the pinned trusted parent. Its
    /// contents remain untrusted even when their owner matches the binding.
    pub fn session(&self, entry: &OsStr, uid: u32, gid: u32) -> Result<File> {
        self.current()?;
        let directory = file_at(
            &self.directory,
            &session_name(entry)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == uid
                && metadata.gid() == gid
                && metadata.mode() & 0o777 == 0o700,
            "runtime session directory identity or mode changed"
        );
        self.current()?;
        Ok(directory)
    }
    /// Reviewed identity handoff only after protected old-process retirement.
    /// Pin and validate the whole bounded tree before changing any ownership.
    /// No content/SQLite parsing occurs here; symlinks, hardlinks and devices refuse.
    pub fn transfer_session(&self,entry:&OsStr,source_uid:u32,source_gid:u32,target_uid:u32,target_gid:u32)->Result<()> {
        ensure!(unsafe{libc::geteuid()}==self.owner,"runtime handoff requires its root owner");
        let directory=self.session(entry,source_uid,source_gid)?;
        let mut objects=Vec::new();
        pin_tree(&directory,source_uid,source_gid,0,&mut objects)?;
        // The directory fence removes source pathname access before descendants
        // change. A failure remains fenced for explicit, receipt-based recovery.
        ensure!(unsafe{libc::fchown(directory.as_raw_fd(),self.owner,self.owner)}==0,"runtime source fence failed");
        let mut current=Vec::new();pin_tree(&directory,source_uid,source_gid,0,&mut current)?;
        let before=objects.iter().map(|file|file.metadata().map(|metadata|(metadata.dev(),metadata.ino()))).collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
        let after=current.iter().map(|file|file.metadata().map(|metadata|(metadata.dev(),metadata.ino()))).collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
        ensure!(before==after,"runtime handoff tree changed before source fence");
        drop(current);
        for object in &objects {
            ensure!(unsafe{libc::fchownat(object.as_raw_fd(),c"".as_ptr(),target_uid,target_gid,libc::AT_EMPTY_PATH|libc::AT_SYMLINK_NOFOLLOW)}==0,"runtime descendant ownership handoff failed");
        }
        ensure!(unsafe{libc::fchown(directory.as_raw_fd(),target_uid,target_gid)}==0,"runtime target ownership handoff failed");
        directory.sync_all()?;self.directory.sync_all()?;
        self.session(entry,target_uid,target_gid)?;Ok(())
    }

    /// Provision a new private runtime directory. Existing entries are never
    /// chowned or repaired. A failed ownership change leaves an inaccessible
    /// root-owned directory for explicit operator reconciliation.
    pub fn create_session(&self, entry: &OsStr, uid: u32, gid: u32) -> Result<File> {
        self.current()?;
        ensure!(
            unsafe { libc::geteuid() } == self.owner,
            "runtime directory creation requires its owning OS identity"
        );
        let entry_name = session_name(entry)?;
        let result =
            unsafe { libc::mkdirat(self.directory.as_raw_fd(), entry_name.as_ptr(), 0o700) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            ensure!(
                error.raw_os_error() == Some(libc::EEXIST),
                "runtime session directory creation failed: {error}"
            );
            return self.session(entry, uid, gid);
        }
        let directory = file_at(
            &self.directory,
            &entry_name,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.is_dir() && metadata.uid() == self.owner,
            "new runtime session directory changed before ownership assignment"
        );
        ensure!(
            unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } == 0,
            "runtime session permission assignment failed: {}",
            std::io::Error::last_os_error()
        );
        if metadata.uid() != uid || metadata.gid() != gid {
            ensure!(
                unsafe { libc::fchown(directory.as_raw_fd(), uid, gid) } == 0,
                "runtime session ownership assignment failed: {}",
                std::io::Error::last_os_error()
            );
        }
        directory.sync_all()?;
        self.directory.sync_all()?;
        self.session(entry, uid, gid)
    }
}

impl RootDirectory {
    /// Existing root-owned private directory under root-owned non-writable
    /// ancestors. No credentials or runtime data are loaded by this operation.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_owned(path, 0)
    }
    fn open_owned(path: &Path, owner: u32) -> Result<Self> {
        ensure!(path.is_absolute(), "control directory must be absolute");
        let mut directory = File::open("/")?;
        check_directory(&directory, owner, false)?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(value) => {
                    directory = file_at(
                        &directory,
                        &name(value)?,
                        libc::O_RDONLY | libc::O_DIRECTORY,
                        0,
                    )?;
                    check_directory(&directory, owner, false)?;
                }
                _ => anyhow::bail!("control path must not contain relative components"),
            }
        }
        check_directory(&directory, owner, true)?;
        Ok(Self { directory, owner })
    }
    fn current(&self) -> Result<()> {
        check_directory(&self.directory, self.owner, true)
    }
    pub fn child(&self, entry: &OsStr) -> Result<Self> {
        self.current()?;
        let directory = file_at(
            &self.directory,
            &name(entry)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
            0,
        )?;
        check_directory(&directory, self.owner, true)?;
        Ok(Self {
            directory,
            owner: self.owner,
        })
    }
    /// Creates one private descendant; never repairs permissions/ownership on an
    /// existing object. System setup must provision the root anchor separately.
    pub fn create_child(&self, entry: &OsStr) -> Result<Self> {
        self.current()?;
        ensure!(
            unsafe { libc::geteuid() } == self.owner,
            "control creation requires its owning OS identity"
        );
        let entry_name = name(entry)?;
        let result =
            unsafe { libc::mkdirat(self.directory.as_raw_fd(), entry_name.as_ptr(), 0o700) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            ensure!(
                error.raw_os_error() == Some(libc::EEXIST),
                "control directory creation failed: {error}"
            );
        }
        let child = self.child(entry)?;
        self.directory.sync_all()?;
        Ok(child)
    }
    /// Reads a bounded regular private file. Never follows symbolic links, opens
    /// a pipe/device, or accepts a hardlinked record as supervisor authority.
    pub fn read(&self, entry: &OsStr, limit: u64) -> Result<Vec<u8>> {
        self.current()?;
        let pinned = file_at(&self.directory, &name(entry)?, libc::O_PATH, 0)?;
        check_file(&pinned, self.owner, limit)?;
        // Reopen the checked inode, not its potentially replaced directory entry.
        // The first O_PATH open cannot trigger FIFO/device open behavior.
        let mut file = File::open(format!("/proc/self/fd/{}", pinned.as_raw_fd()))?;
        let expected = pinned.metadata()?;
        let actual = file.metadata()?;
        ensure!(
            (expected.dev(), expected.ino()) == (actual.dev(), actual.ino()),
            "control descriptor identity changed"
        );
        check_file(&file, self.owner, limit)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(
                limit
                    .checked_add(1)
                    .context("control read limit overflow")?,
            )
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= limit,
            "control record grew beyond limit"
        );
        check_file(&file, self.owner, limit)?;
        self.current()?;
        Ok(bytes)
    }
    /// Publishes a new record without replacing an existing name. The durable
    /// content is complete before rename. Uncertain sync/publication failures must
    /// be resolved by reading the exact operation record, never blind replay.
    pub fn publish_new(&self, entry: &OsStr, bytes: &[u8], limit: usize) -> Result<()> {
        self.current()?;
        ensure!(bytes.len() <= limit, "control record exceeds limit");
        ensure!(
            unsafe { libc::geteuid() } == self.owner,
            "control publication requires its owning OS identity"
        );
        let destination = name(entry)?;
        let mut nonce = [0u8; 16];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("control nonce unavailable"))?;
        let temporary = CString::new(format!(
            ".pending-{}",
            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))?;
        let mut file = file_at(
            &self.directory,
            &temporary,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        let result = (|| -> Result<()> {
            check_file(&file, self.owner, limit as u64)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            check_file(&file, self.owner, limit as u64)?;
            self.current()?;
            // RENAME_NOREPLACE is a kernel-enforced admission, including dangling
            // symlinks. No exists()/rename() race and no hardlink publication.
            let result = unsafe {
                libc::renameat2(
                    self.directory.as_raw_fd(),
                    temporary.as_ptr(),
                    self.directory.as_raw_fd(),
                    destination.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            ensure!(
                result == 0,
                "control publication failed: {}",
                std::io::Error::last_os_error()
            );
            self.directory.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            // Only this operation's random temporary name. If publication already
            // happened, unlinkat cannot remove the destination or someone else's data.
            unsafe {
                libc::unlinkat(self.directory.as_raw_fd(), temporary.as_ptr(), 0);
            }
        }
        result
    }
    /// Returns an exclusive nonblocking transaction lock. Retain the handle for
    /// the whole logical operation; contention is a bounded refusal.
    pub fn lock(&self, entry: &OsStr) -> Result<File> {
        self.current()?;
        let entry = name(entry)?;
        let file = match file_at(
            &self.directory,
            &entry,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        ) {
            Ok(file) => {
                self.directory.sync_all()?;
                file
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
            {
                let pinned = file_at(&self.directory, &entry, libc::O_PATH, 0)?;
                check_file(&pinned, self.owner, 0)?;
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(format!("/proc/self/fd/{}", pinned.as_raw_fd()))?;
                let expected = pinned.metadata()?;
                let actual = file.metadata()?;
                ensure!(
                    (expected.dev(), expected.ino()) == (actual.dev(), actual.ino()),
                    "control lock identity changed"
                );
                file
            }
            Err(error) => return Err(error),
        };
        check_file(&file, self.owner, 0)?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "control transaction already active"
        );
        Ok(file)
    }
}

#[cfg(test)]
#[path = "protected_linux_tests.rs"]
mod tests;


fn pin_tree(directory:&File,uid:u32,gid:u32,depth:usize,objects:&mut Vec<File>)->Result<()> {
    ensure!(depth<=32&&objects.len()<512,"runtime handoff tree exceeds bounded review limits");
    let raw=unsafe{libc::openat(directory.as_raw_fd(),c".".as_ptr(),libc::O_RDONLY|libc::O_DIRECTORY|libc::O_CLOEXEC|libc::O_NOFOLLOW)};
    ensure!(raw>=0,"runtime handoff listing unavailable");
    let stream=unsafe{libc::fdopendir(raw)};
    if stream.is_null(){unsafe{libc::close(raw)};anyhow::bail!("runtime handoff listing unavailable");}
    struct Listing(*mut libc::DIR);
    impl Drop for Listing{fn drop(&mut self){unsafe{libc::closedir(self.0)};}}
    let _listing=Listing(stream);
    loop {
        unsafe{*libc::__errno_location()=0};
        let item=unsafe{libc::readdir(stream)};
        if item.is_null(){ensure!(std::io::Error::last_os_error().raw_os_error()==Some(0),"runtime handoff listing failed");break;}
        let value=unsafe{std::ffi::CStr::from_ptr((*item).d_name.as_ptr())};
        if value.to_bytes()==b"."||value.to_bytes()==b".."{continue;}
        ensure!(objects.len()<512,"runtime handoff file limit exceeded");
        let name=CString::new(value.to_bytes())?;
        let object=file_at(directory,&name,libc::O_PATH,0)?;
        let metadata=object.metadata()?;
        use std::os::unix::fs::FileTypeExt;
        ensure!(metadata.uid()==uid&&metadata.gid()==gid&&metadata.mode()&0o077==0&&!metadata.file_type().is_symlink()&&(metadata.is_dir()||metadata.is_file()||metadata.file_type().is_socket()),"unsafe runtime handoff entry");
        ensure!(!metadata.is_file()||metadata.nlink()==1,"runtime handoff hardlink refused");
        if metadata.is_dir(){
            let child=file_at(directory,&name,libc::O_RDONLY|libc::O_DIRECTORY,0)?;
            let current=child.metadata()?;
            ensure!(current.dev()==metadata.dev()&&current.ino()==metadata.ino(),"runtime handoff directory replaced");
            pin_tree(&child,uid,gid,depth+1,objects)?;
        }
        objects.push(object);
    }
    Ok(())
}
