use super::*;
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Seek, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt, process::CommandExt},
    },
};
#[derive(Debug)]
pub(super) struct Root {
    path: PathBuf,
    file: File,
    writable: bool,
}
fn pin(path: &Path) -> Result<File, Error> {
    let mut fd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open("/")
        .map_err(|_| Error::Setup)?;
    let components = path.components().collect::<Vec<_>>();
    if !path.is_absolute() {
        return Err(Error::Invalid("roots must be absolute"));
    }
    for (i, part) in components.iter().enumerate().skip(1) {
        let std::path::Component::Normal(name) = part else {
            return Err(Error::Invalid("roots must be normalized"));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| Error::Invalid("invalid root"))?;
        let flags = libc::O_PATH
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if i + 1 < components.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: owned parent descriptor and NUL-terminated component; no following links.
        let next = unsafe { libc::openat(fd.as_raw_fd(), name.as_ptr(), flags) };
        if next < 0 {
            return Err(Error::Setup);
        }
        fd = unsafe { File::from_raw_fd(next) };
        if fd
            .metadata()
            .map_err(|_| Error::Setup)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Invalid("symbolic link root"));
        }
    }
    let metadata = fd.metadata().map_err(|_| Error::Setup)?;
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(Error::Invalid("roots must be regular files or directories"));
    }
    Ok(fd)
}
fn reserved(path: &Path) -> bool {
    path == Path::new("/")
        || [
            "/usr",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/proc",
            "/dev",
            "/sys",
            "/run",
            "/etc/ssl/certs",
        ]
        .iter()
        .any(|x| path.starts_with(x) || Path::new(x).starts_with(path))
        || path == Path::new("/tmp")
        || path == Path::new("/etc")
        || path == Path::new("/home")
}
pub(super) fn roots(read: &[PathBuf], write: &[PathBuf]) -> Result<Vec<Root>, Error> {
    if read.len() + write.len() > 128 {
        return Err(Error::Invalid("too many roots"));
    }
    let mut roots = std::collections::BTreeMap::new();
    for (paths, writable) in [(read, false), (write, true)] {
        for path in paths {
            if reserved(path) {
                return Err(Error::Invalid("root overlaps reserved runtime interface"));
            }
            roots.insert(path.clone(), writable);
        }
    }
    // Mount parents before children, retaining exact explicit read/write overlays.
    let mut roots = roots
        .into_iter()
        .map(|(path, writable)| {
            Ok(Root {
                file: pin(&path)?,
                path,
                writable,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    roots.sort_by_key(|r| r.path.components().count());
    Ok(roots)
}
#[allow(deprecated)] // Deny historical syscall numbers as well as current interfaces.
fn filter(network: Network) -> Vec<libc::sock_filter> {
    let mut p = vec![];
    let mut stmt = |code, k| {
        p.push(libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        })
    };
    stmt(0x20, 4); // seccomp_data.arch
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 1,
        jf: 0,
        k: 0xc000003e,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x80000000,
    }); // kill foreign ABI
    p.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 0,
    });
    p.push(libc::sock_filter {
        code: 0x45,
        jt: 0,
        jf: 1,
        k: 0x40000000,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x80000000,
    }); // x32 forbidden
    // Deny isolation escapes, global kernel interfaces and alternative file/socket interfaces.
    for nr in [
        libc::SYS_ptrace,
        libc::SYS_pivot_root,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_reboot,
        libc::SYS_iopl,
        libc::SYS_ioperm,
        libc::SYS_create_module,
        libc::SYS_init_module,
        libc::SYS_delete_module,
        libc::SYS_kexec_load,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_keyctl,
        libc::SYS_unshare,
        libc::SYS_perf_event_open,
        libc::SYS_fanotify_init,
        libc::SYS_name_to_handle_at,
        libc::SYS_open_by_handle_at,
        libc::SYS_setns,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_finit_module,
        libc::SYS_bpf,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
        libc::SYS_pidfd_getfd,
        libc::SYS_process_madvise,
    ] {
        p.push(libc::sock_filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            k: nr as u32,
        });
        p.push(libc::sock_filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            k: 0x50000 | libc::EPERM as u32,
        });
    }
    // clone3 opaque argument block: ENOSYS lets libc fall back to inspectable clone.
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: 1,
        k: 435,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x50000 | libc::ENOSYS as u32,
    });
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: 3,
        k: 56,
    });
    p.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 16,
    });
    p.push(libc::sock_filter {
        code: 0x45,
        jt: 0,
        jf: 1,
        k: 0x7e020000,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x50000 | libc::EPERM as u32,
    });
    p.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 0,
    });
    // TIOCSTI can inject into an outer terminal; deny regardless of namespace.
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: 3,
        k: 16,
    });
    p.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 24,
    });
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: 1,
        k: libc::TIOCSTI as u32,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x50000 | libc::EPERM as u32,
    });
    p.push(libc::sock_filter {
        code: 0x20,
        jt: 0,
        jf: 0,
        k: 0,
    });
    // No filesystem or abstract Unix sockets, netlink, packet sockets, etc.
    p.push(libc::sock_filter {
        code: 0x15,
        jt: 0,
        jf: if network == Network::Host { 4 } else { 1 },
        k: 41,
    });
    if network == Network::Host {
        p.push(libc::sock_filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            k: 16,
        });
        p.push(libc::sock_filter {
            code: 0x15,
            jt: 2,
            jf: 0,
            k: libc::AF_INET as u32,
        });
        p.push(libc::sock_filter {
            code: 0x15,
            jt: 1,
            jf: 0,
            k: libc::AF_INET6 as u32,
        });
    }
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x50000 | libc::EPERM as u32,
    });
    p.push(libc::sock_filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        k: 0x7fff0000,
    });
    p
}
pub(super) fn apply(
    sandbox: &Sandbox,
    payload: &mut Command,
    cwd: &Path,
    read_only: bool,
) -> Result<(), Error> {
    let mut cmd = Command::new("/usr/bin/bwrap");
    cmd.args([
        "--unshare-user",
        "--unshare-ipc",
        "--unshare-pid",
        "--unshare-uts",
        "--disable-userns",
        "--die-with-parent",
        "--cap-drop",
        "ALL",
    ]);
    if sandbox.settings.network == Network::Denied {
        cmd.arg("--unshare-net");
    }
    let mut files = vec![];
    // Fixed system runtime, never the operator's home, host root or /etc tree.
    for path in [
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/usr/lib64",
        "/usr/share/terminfo",
        "/usr/share/zoneinfo",
        "/bin",
        "/sbin",
        "/lib",
        "/lib64",
    ] {
        let path = Path::new(path);
        if !path.exists() {
            continue;
        }
        if let Ok(target) = std::fs::read_link(path) {
            cmd.arg("--symlink").arg(target).arg(path);
        } else {
            let file = pin(path)?;
            cmd.arg("--ro-bind-fd")
                .arg(file.as_raw_fd().to_string())
                .arg(path);
            files.push(file);
        }
    }
    cmd.args([
        "--proc", "/proc", "--dev", "/dev", "--dir", "/run", "--dir", "/etc",
    ]);
    for path in [
        "/etc/ld.so.cache",
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/ssl/certs",
    ] {
        let path = Path::new(path);
        if path.exists() {
            let resolved = path.canonicalize().map_err(|_| Error::Setup)?;
            let file = pin(&resolved)?;
            cmd.arg("--ro-bind-fd")
                .arg(file.as_raw_fd().to_string())
                .arg(path);
            files.push(file);
        }
    }
    cmd.arg("--size")
        .arg(sandbox.settings.temporary_bytes.to_string())
        .args(["--tmpfs", "/tmp"]);
    for root in sandbox.roots.iter() {
        let file = root.file.try_clone().map_err(|_| Error::Setup)?;
        cmd.arg(if root.writable && !read_only {
            "--bind-fd"
        } else {
            "--ro-bind-fd"
        })
        .arg(file.as_raw_fd().to_string())
        .arg(&root.path);
        files.push(file);
    }
    // Only explicit write mounts and sized /tmp may accept ordinary files.
    // Root and /dev tmpfs must not provide an unbounded temporary-storage bypass.
    cmd.args(["--remount-ro", "/", "--remount-ro", "/dev"]);
    let mut seccomp = tempfile::tempfile().map_err(|_| Error::Setup)?;
    for instruction in filter(sandbox.settings.network) {
        seccomp
            .write_all(&instruction.code.to_ne_bytes())
            .and_then(|_| seccomp.write_all(&[instruction.jt, instruction.jf]))
            .and_then(|_| seccomp.write_all(&instruction.k.to_ne_bytes()))
            .map_err(|_| Error::Setup)?;
    }
    seccomp.rewind().map_err(|_| Error::Setup)?;
    cmd.arg("--seccomp").arg(seccomp.as_raw_fd().to_string());
    let seccomp_fd = seccomp.as_raw_fd();
    files.push(seccomp);
    for (name, value) in payload.get_envs() {
        if let Some(value) = value {
            cmd.arg("--setenv").arg(name).arg(value);
        }
    }
    cmd.arg("--chdir").arg(cwd);
    let mut arguments = tempfile::tempfile().map_err(|_| Error::Setup)?;
    let mut size = 0usize;
    for arg in cmd.get_args() {
        let bytes = arg.as_bytes();
        size = size.saturating_add(bytes.len() + 1);
        if bytes.contains(&0) || size > 1024 * 1024 {
            return Err(Error::Invalid(
                "sandbox launch arguments exceed 1 MiB or contain NUL",
            ));
        }
        arguments
            .write_all(bytes)
            .and_then(|_| arguments.write_all(&[0]))
            .map_err(|_| Error::Setup)?;
    }
    arguments.rewind().map_err(|_| Error::Setup)?;
    let argument_file_fd = arguments.as_raw_fd();
    let arguments_fd = CString::new(argument_file_fd.to_string()).map_err(|_| Error::Setup)?;
    let mut argv_strings = vec![
        CString::new("bwrap").unwrap(),
        CString::new("--args").unwrap(),
        arguments_fd,
        CString::new("--").unwrap(),
    ];
    for arg in std::iter::once(payload.get_program()).chain(payload.get_args()) {
        argv_strings.push(
            CString::new(arg.as_bytes()).map_err(|_| Error::Invalid("invalid payload argument"))?,
        );
    }
    let mut argv_pointers = argv_strings
        .iter()
        .map(|value| value.as_ptr() as usize)
        .collect::<Vec<_>>();
    argv_pointers.push(0);
    files.push(arguments);
    // No loader hooks or payload credentials may enter the adapter environment.
    payload.env_clear();
    let settings = sandbox.settings.clone();
    // SAFETY: this child-only closure uses only async-signal-safe syscalls, no allocation.
    // CLOEXEC everything first, then expose only immutable setup descriptors to bwrap.
    // bwrap closes them before executing the payload; the exec-error pipe stays CLOEXEC.
    unsafe {
        payload.pre_exec(move || {
            // A Command may be spawned repeatedly; bwrap consumes these offsets.
            for fd in [seccomp_fd, argument_file_fd] {
                if libc::lseek(fd, 0, libc::SEEK_SET) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for file in &files {
                if libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            for (resource, value) in [
                (libc::RLIMIT_AS, settings.address_space_bytes),
                (libc::RLIMIT_CPU, settings.cpu_seconds),
                (libc::RLIMIT_FSIZE, settings.file_size_bytes),
                (libc::RLIMIT_NOFILE, settings.open_files),
                (libc::RLIMIT_NPROC, settings.uid_processes),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let _keep_arguments_alive = &argv_strings;
            let environment = [std::ptr::null::<libc::c_char>()];
            libc::execve(
                c"/usr/bin/bwrap".as_ptr(),
                argv_pointers.as_ptr().cast(),
                environment.as_ptr(),
            );
            Err(std::io::Error::last_os_error())
        });
    }
    Ok(())
}
