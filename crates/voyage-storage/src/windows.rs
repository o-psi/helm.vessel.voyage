//! Native ACL validation and handles that pin the checked directory hierarchy.
use crate::policy::{Grant, private_acl, sid_length};
use std::{
    ffi::c_void,
    fs::File,
    io::{self, Write},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf, Prefix},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{
        SystemServices::FILE_PERSISTENT_ACLS,
        Threading::{GetCurrentProcess, OpenProcessToken},
        WindowsProgramming::DRIVE_FIXED,
    },
};

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "enrollment storage is not private",
    )
}
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "private storage requires a local NTFS volume",
    )
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(denied());
    }
    value.push(0);
    Ok(value)
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: descriptors returned by Windows are LocalAlloc allocations.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn copied_sid(sid: PSID) -> io::Result<Vec<u8>> {
    // SAFETY: callers pass SID pointers returned within live Windows buffers.
    unsafe {
        if sid.is_null() || IsValidSid(sid) == 0 {
            return Err(denied());
        }
        let length = GetLengthSid(sid) as usize;
        if !(8..=68).contains(&length) {
            return Err(denied());
        }
        Ok(std::slice::from_raw_parts(sid.cast::<u8>(), length).to_vec())
    }
}
fn current_sid() -> io::Result<Vec<u8>> {
    // SAFETY: all output pointers refer to initialized, correctly aligned storage;
    // the token remains owned until after its SID has been copied.
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut bytes = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes);
        if bytes == 0 || bytes > 65536 {
            return Err(denied());
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        copied_sid((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}
fn descriptor(sid: &[u8]) -> io::Result<Descriptor> {
    // SAFETY: the SID is a copied, validated Windows SID; allocated strings and
    // security descriptors are released through LocalFree exactly once.
    unsafe {
        let mut text = null_mut();
        if ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        let sid_text = String::from_utf16(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        let sid_text = sid_text.map_err(|_| denied())?;
        let sddl = wide(Path::new(&format!(
            "O:{sid_text}D:P(A;OICI;FA;;;{sid_text})"
        )))?;
        let mut sd = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Descriptor(sd))
    }
}
fn attributes(sd: &Descriptor) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    }
}
fn open_handle(
    path: &Path,
    access: u32,
    creation: u32,
    directory: bool,
    sd: Option<&Descriptor>,
) -> io::Result<File> {
    let path = wide(path)?;
    let sa = sd.map(attributes);
    // SAFETY: strings/optional security attributes stay live for this synchronous
    // call. Owned File closes a successful handle. Delete sharing is deliberately
    // absent: checked ancestors/lock files cannot be renamed under this owner.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            sa.as_ref().map_or(null(), |sa| sa),
            creation,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    FILE_ATTRIBUTE_NORMAL
                },
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_handle(handle) })
    }
}
fn information(file: &File, directory: bool) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    // SAFETY: file owns a live handle and info is a correctly sized output buffer.
    unsafe {
        let mut info = zeroed();
        if GetFileType(file.as_raw_handle()) != FILE_TYPE_DISK
            || GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0
        {
            return Err(denied());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
            || (!directory && info.nNumberOfLinks != 1)
        {
            return Err(denied());
        }
        Ok(info)
    }
}
fn verify_acl(file: &File, expected: &[u8], directory: bool) -> io::Result<()> {
    information(file, directory)?;
    // SAFETY: Windows owns descriptor layout; GetAce validates indexing. We check
    // ACE type/length before interpreting its SID and keep the descriptor alive.
    unsafe {
        let mut sd = null_mut();
        let mut owner = null_mut();
        let mut acl = null_mut();
        let result = GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut sd,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let sd = Descriptor(sd);
        if acl.is_null() || IsValidAcl(acl) == 0 {
            return Err(denied());
        }
        let mut control = 0;
        let mut revision = 0;
        if GetSecurityDescriptorControl(sd.0, &mut control, &mut revision) == 0 {
            return Err(denied());
        }
        let owner = copied_sid(owner)?;
        let mut grants = Vec::new();
        for index in 0..(*acl).AceCount {
            let mut pointer: *mut c_void = null_mut();
            if GetAce(acl, index as u32, &mut pointer) == 0 {
                return Err(denied());
            }
            let acl_start = acl as usize;
            let acl_end = acl_start
                .checked_add((*acl).AclSize as usize)
                .ok_or_else(denied)?;
            let ace_start = pointer as usize;
            if ace_start < acl_start + size_of::<ACL>()
                || ace_start
                    .checked_add(size_of::<ACE_HEADER>())
                    .is_none_or(|end| end > acl_end)
            {
                return Err(denied());
            }
            let header = &*pointer.cast::<ACE_HEADER>();
            if ace_start
                .checked_add(header.AceSize as usize)
                .is_none_or(|end| end > acl_end)
            {
                return Err(denied());
            }
            // ACCESS_ALLOWED_ACE_TYPE = 0. Callback/object/deny/unknown ACEs are
            // deliberately unsupported rather than approximating their meaning.
            if header.AceType != 0 || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(denied());
            }
            let ace = &*pointer.cast::<ACCESS_ALLOWED_ACE>();
            let sid_pointer = (&ace.SidStart as *const u32).cast::<u8>();
            let available = header.AceSize as usize - 8;
            let encoded = std::slice::from_raw_parts(sid_pointer, available);
            let length = sid_length(encoded).ok_or_else(denied)?;
            if IsValidSid(sid_pointer.cast_mut().cast()) == 0 {
                return Err(denied());
            }
            let sid = encoded[..length].to_vec();
            grants.push(Grant {
                sid,
                mask: ace.Mask,
                flags: header.AceFlags,
            });
        }
        if !private_acl(
            &owner,
            expected,
            control & SE_DACL_PROTECTED != 0,
            directory,
            &grants,
        ) {
            return Err(denied());
        }
    }
    Ok(())
}

/// Pins each checked ancestor and the dedicated private directory for its lifetime.
/// Same-account malicious processes and privileged administrators are not isolated.
pub struct PrivateDirectory {
    path: PathBuf,
    sid: Vec<u8>,
    _handles: Vec<File>,
}
impl PrivateDirectory {
    pub fn open(path: &Path) -> io::Result<Self> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let components: Vec<_> = path.components().collect();
        let Some(Component::Prefix(prefix)) = components.first() else {
            return Err(denied());
        };
        let drive = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err(unsupported()),
        };
        if components.len() < 3 || !matches!(components[1], Component::RootDir)
            || components[2..].iter().any(|c| !matches!(c, Component::Normal(name) if !name.encode_wide().any(|c| c == b':' as u16))) { return Err(denied()); }
        let drive_path = wide(Path::new(&format!("{}:\\", drive as char)))?;
        if unsafe { GetDriveTypeW(drive_path.as_ptr()) } != DRIVE_FIXED {
            return Err(unsupported());
        }
        let sid = current_sid()?;
        let sd = descriptor(&sid)?;
        let mut prefix_path = PathBuf::new();
        let mut handles = Vec::new();
        for (index, component) in components.iter().enumerate() {
            prefix_path.push(component.as_os_str());
            if index == 0 {
                continue;
            }
            if index == components.len() - 1 {
                let name = wide(&prefix_path)?;
                if unsafe { CreateDirectoryW(name.as_ptr(), &attributes(&sd)) } == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                        return Err(error);
                    }
                }
            }
            let handle = open_handle(
                &prefix_path,
                FILE_READ_ATTRIBUTES,
                OPEN_EXISTING,
                true,
                None,
            )?;
            information(&handle, true)?;
            if index == 1 {
                let mut flags = 0;
                let mut filesystem = [0u16; 32];
                if unsafe {
                    GetVolumeInformationByHandleW(
                        handle.as_raw_handle(),
                        null_mut(),
                        0,
                        null_mut(),
                        null_mut(),
                        &mut flags,
                        filesystem.as_mut_ptr(),
                        filesystem.len() as u32,
                    )
                } == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let length = filesystem
                    .iter()
                    .position(|c| *c == 0)
                    .ok_or_else(unsupported)?;
                if flags & FILE_PERSISTENT_ACLS == 0
                    || String::from_utf16_lossy(&filesystem[..length]) != "NTFS"
                {
                    return Err(unsupported());
                }
            }
            handles.push(handle);
        }
        verify_acl(handles.last().ok_or_else(denied)?, &sid, true)?;
        Ok(Self {
            path,
            sid,
            _handles: handles,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn child(&self, name: &str) -> io::Result<PathBuf> {
        if name.is_empty()
            || name.contains(['/', '\\', ':', '\0'])
            || name == "."
            || name == ".."
            || name.ends_with(['.', ' '])
        {
            return Err(denied());
        }
        Ok(self.path.join(name))
    }
    pub fn open_file(&self, name: &str, create: bool) -> io::Result<File> {
        let path = self.child(name)?;
        let sd = descriptor(&self.sid)?;
        let file = open_handle(
            &path,
            GENERIC_READ | if create { GENERIC_WRITE } else { 0 },
            if create { OPEN_ALWAYS } else { OPEN_EXISTING },
            false,
            Some(&sd),
        )?;
        verify_acl(&file, &self.sid, false)?;
        Ok(file)
    }
    pub fn lock(&self, name: &str) -> io::Result<File> {
        let file = self.open_file(name, true)?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
            std::fs::TryLockError::Error(error) => error,
        })?;
        Ok(file)
    }
    /// Atomically publish in this same private directory. Any error is uncertain
    /// to the caller, which must retain its original transaction and stop writes.
    pub fn publish(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let destination = self.child(name)?;
        match self.open_file(name, false) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let temporary = self.child(&format!(".tmp-{}", uuid::Uuid::new_v4()))?;
        let sd = descriptor(&self.sid)?;
        let mut file = open_handle(
            &temporary,
            GENERIC_READ | GENERIC_WRITE,
            CREATE_NEW,
            false,
            Some(&sd),
        )?;
        let result = (|| {
            verify_acl(&file, &self.sid, false)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            let source = wide(&temporary)?;
            let target = wide(&destination)?;
            if unsafe {
                MoveFileExW(
                    source.as_ptr(),
                    target.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            self.open_file(name, false)?;
            Ok(())
        })();
        // Only this freshly created, randomly named temporary file is removed.
        // Existing data and lock files are never unlinked on failure.
        let _ = std::fs::remove_file(&temporary);
        result
    }
}
#[cfg(test)]
mod tests;
