//! Read-only assessment for a future explicit system-scope installation.
//! No result here authorizes changing an existing service or adopting its state.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    ffi::{CStr, CString},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
struct Check {
    name: &'static str,
    observed: String,
    required: &'static str,
    passed: bool,
}

#[derive(Serialize)]
struct Assessment {
    schema_version: u32,
    scope: &'static str,
    execution_user: String,
    gateway_user: String,
    checks: Vec<Check>,
    blockers: Vec<String>,
    existing_user_installation: bool,
    existing_system_installation: bool,
    ready_to_install: bool,
}

pub(super) struct Account {
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) home: PathBuf,
    pub(super) groups: Vec<u32>,
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let names = parse(args)?;
    let assessment = assess(names.execution_user, names.gateway_user)?;
    println!("{}", serde_json::to_string_pretty(&assessment)?);
    Ok(())
}

struct AccountNames<'a> {
    execution_user: &'a str,
    gateway_user: &'a str,
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn parse(args: &[String]) -> Result<AccountNames<'_>> {
    let [execution_flag, execution_user, gateway_flag, gateway_user] = args else {
        bail!(
            "system-assess requires --execution-user USER --gateway-user USER; no login or SUDO_USER identity is inferred"
        )
    };
    ensure!(
        execution_flag == "--execution-user" && gateway_flag == "--gateway-user",
        "expected --execution-user USER --gateway-user USER"
    );
    ensure!(
        valid_name(execution_user) && valid_name(gateway_user),
        "execution and gateway users must be explicit local account names"
    );
    ensure!(
        execution_user != gateway_user,
        "gateway and execution users must differ"
    );
    Ok(AccountNames {
        execution_user,
        gateway_user,
    })
}

pub(super) fn account(name: &str) -> Result<Account> {
    let name = CString::new(name)?;
    let mut entry = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0u8; 65536];
    let status = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    ensure!(status == 0, "account lookup failed: errno {status}");
    ensure!(!result.is_null(), "ordinary account does not exist");
    ensure!(
        entry.pw_uid != 0 && entry.pw_gid != 0,
        "account must be ordinary"
    );
    ensure!(!entry.pw_dir.is_null(), "account has no home");
    let home = unsafe { CStr::from_ptr(entry.pw_dir) };
    ensure!(
        home.to_bytes().len() <= 4096,
        "account home path is too long"
    );
    let home = PathBuf::from(home.to_str().context("account home is not UTF-8")?);
    ensure!(
        home.is_absolute()
            && !home.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            }),
        "account home must be an absolute normalized path"
    );
    let mut groups = vec![0 as libc::gid_t; 256];
    let mut count = groups.len() as libc::c_int;
    let found =
        unsafe { libc::getgrouplist(name.as_ptr(), entry.pw_gid, groups.as_mut_ptr(), &mut count) };
    ensure!(
        found >= 0 && count > 0 && count <= 256,
        "account group list is unavailable or too large"
    );
    groups.truncate(count as usize);
    groups.sort_unstable();
    groups.dedup();
    Ok(Account {
        uid: entry.pw_uid,
        gid: entry.pw_gid,
        home,
        groups,
    })
}

fn checked_path(path: &Path, expected_uid: u32, directory: bool) -> Result<String> {
    ensure!(path.is_absolute(), "system path must be absolute");
    let mut exists = false;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if ancestor == path {
                    exists = true;
                    ensure!(
                        if directory {
                            metadata.is_dir()
                        } else {
                            metadata.is_file()
                        },
                        "unexpected system path type at {}",
                        path.display()
                    );
                }
                ensure!(
                    !metadata.file_type().is_symlink()
                        && (metadata.uid() == expected_uid || metadata.uid() == 0)
                        && metadata.mode() & 0o022 == 0
                        && (metadata.is_dir() || (metadata.is_file() && metadata.nlink() == 1)),
                    "unsafe ownership, mode, link or type at {}",
                    ancestor.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(if exists { "present" } else { "absent" }.into())
}

pub(super) fn proc_value(path: &str, limit: usize) -> Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .with_context(|| format!("cannot inspect {path}"))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "host probe exceeded its bound");
    Ok(String::from_utf8(bytes)?.trim().to_owned())
}

pub(super) fn capability_bits() -> Result<u64> {
    let status = proc_value("/proc/self/status", 65536)?;
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:\t"))
        .context("effective capabilities unavailable")?;
    Ok(u64::from_str_radix(value.trim(), 16)?)
}

pub(super) fn full_identity_map(value: &str) -> bool {
    let mut lines = value.lines();
    let Some(line) = lines.next() else {
        return false;
    };
    let mut columns = line.split_ascii_whitespace();
    columns.next() == Some("0")
        && columns.next() == Some("0")
        && columns.next() == Some("4294967295")
        && columns.next().is_none()
        && lines.next().is_none()
}

fn mount_writable(path: &Path) -> Result<bool> {
    let existing = path
        .ancestors()
        .find(|candidate| candidate.exists())
        .context("no existing mount ancestor")?;
    let path = CString::new(existing.as_os_str().as_encoded_bytes())?;
    let mut info = unsafe { std::mem::zeroed::<libc::statvfs>() };
    ensure!(
        unsafe { libc::statvfs(path.as_ptr(), &mut info) } == 0,
        "mount probe failed"
    );
    Ok(info.f_flag & libc::ST_RDONLY == 0)
}

fn service_state(unit: &str) -> Result<String> {
    let output = crate::service::command::run(
        Path::new("/usr/bin/systemctl"),
        &[
            "--system",
            "--no-pager",
            "show",
            unit,
            "--property=LoadState,ActiveState,UnitFileState,FragmentPath,DropInPaths,MainPID",
        ],
        None,
    )?;
    let output = String::from_utf8(output)?;
    ensure!(
        output.len() <= 8192,
        "system service state exceeded its bound"
    );
    Ok(output.trim().replace('\n', "; "))
}

fn record(
    checks: &mut Vec<Check>,
    blockers: &mut Vec<String>,
    name: &'static str,
    required: &'static str,
    result: Result<String>,
    accepted: impl FnOnce(&str) -> bool,
) {
    let (observed, passed) = match result {
        Ok(observed) => {
            let passed = accepted(&observed);
            (observed, passed)
        }
        Err(error) => (format!("unavailable: {error:#}"), false),
    };
    if !passed {
        blockers.push(format!("{name}: {observed}; required: {required}"));
    }
    checks.push(Check {
        name,
        observed,
        required,
        passed,
    });
}

fn assess(execution_user: &str, gateway_user: &str) -> Result<Assessment> {
    let mut checks = Vec::new();
    let mut blockers = Vec::new();
    let execution_account = account(execution_user);
    let gateway_account = account(gateway_user);
    record(
        &mut checks,
        &mut blockers,
        "platform",
        "Linux x86-64",
        Ok(format!(
            "{} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )),
        |value| value == "linux x86_64",
    );
    record(
        &mut checks,
        &mut blockers,
        "effective identity",
        "effective root with setuid and setgid capabilities; namespace authority requires separate review",
        capability_bits().map(|bits| {
            format!(
                "euid={} uid={} CapEff={bits:016x}",
                unsafe { libc::geteuid() },
                unsafe { libc::getuid() }
            )
        }),
        |_| {
            (unsafe { libc::geteuid() == 0 && libc::getuid() == 0 })
                && capability_bits()
                    .is_ok_and(|bits| bits & ((1 << 6) | (1 << 7)) == ((1 << 6) | (1 << 7)))
        },
    );
    record(
        &mut checks,
        &mut blockers,
        "user namespace",
        "initial host UID/GID maps; container namespace requires separate review",
        (|| {
            let uid = proc_value("/proc/self/uid_map", 4096)?;
            let gid = proc_value("/proc/self/gid_map", 4096)?;
            ensure!(
                full_identity_map(&uid) && full_identity_map(&gid),
                "UID/GID map is not the full initial mapping"
            );
            Ok(format!("uid_map={uid}; gid_map={gid}"))
        })(),
        |_| true,
    );
    let root_service = if Path::new("/run/systemd/system").is_dir() {
        service_state(super::system_service::ROOT_UNIT)
    } else {
        bail_result("/run/systemd/system is absent")
    };
    let gateway_service = if Path::new("/run/systemd/system").is_dir() {
        service_state(super::system_service::GATEWAY_UNIT)
    } else {
        bail_result("/run/systemd/system is absent")
    };
    let existing_system_service = root_service
        .as_ref()
        .is_ok_and(|value| value.contains("LoadState=loaded"))
        || gateway_service
            .as_ref()
            .is_ok_and(|value| value.contains("LoadState=loaded"));
    record(
        &mut checks,
        &mut blockers,
        "root service manager",
        "running systemd system manager with inspectable root unit",
        root_service,
        |value| value.contains("LoadState=not-found") || value.contains("LoadState=loaded"),
    );
    record(
        &mut checks,
        &mut blockers,
        "gateway service manager",
        "running systemd system manager with inspectable gateway unit",
        gateway_service,
        |value| value.contains("LoadState=not-found") || value.contains("LoadState=loaded"),
    );
    match &execution_account {
        Ok(account) => {
            record(
                &mut checks,
                &mut blockers,
                "execution account",
                "explicit ordinary UID/GID and resolved groups",
                Ok(format!(
                    "uid={} gid={} groups={:?} home={}",
                    account.uid,
                    account.gid,
                    account.groups,
                    account.home.display()
                )),
                |_| true,
            );
            record(
                &mut checks,
                &mut blockers,
                "execution home",
                "existing directory; actual identity access and ancestry must be verified before adoption",
                checked_home(&account.home),
                |value| value == "present",
            );
        }
        Err(error) => record(
            &mut checks,
            &mut blockers,
            "execution account",
            "explicit ordinary UID/GID and resolved groups",
            Err(anyhow::anyhow!("{error:#}")),
            |_| false,
        ),
    }
    match &gateway_account {
        Ok(account) => {
            record(
                &mut checks,
                &mut blockers,
                "gateway account",
                "explicit ordinary UID/GID and resolved groups, distinct from execution",
                Ok(format!(
                    "uid={} gid={} groups={:?} home={}",
                    account.uid,
                    account.gid,
                    account.groups,
                    account.home.display()
                )),
                |_| {
                    execution_account
                        .as_ref()
                        .is_ok_and(|execution| execution.uid != account.uid)
                },
            );
            record(
                &mut checks,
                &mut blockers,
                "gateway home",
                "existing ordinary directory; gateway state must be separate from supervisor control",
                checked_home(&account.home),
                |value| value == "present",
            );
        }
        Err(error) => record(
            &mut checks,
            &mut blockers,
            "gateway account",
            "explicit ordinary UID/GID and resolved groups, distinct from execution",
            Err(anyhow::anyhow!("{error:#}")),
            |_| false,
        ),
    }
    for (name, path, directory) in [
        ("release root", "/opt/voyage", true),
        ("configuration root", "/etc/voyage", true),
        ("control root", "/var/lib/voyage", true),
        ("gateway state parent", "/var/lib/voyage-gateway", true),
        ("runtime endpoint root", "/run/voyage", true),
        (
            "root system unit",
            "/etc/systemd/system/voyage-vessel.service",
            false,
        ),
        (
            "gateway system unit",
            "/etc/systemd/system/voyage-gateway.service",
            false,
        ),
    ] {
        record(
            &mut checks,
            &mut blockers,
            name,
            "root-owned, non-writable ancestry without symlinks or file hardlinks on a writable mount",
            checked_path(Path::new(path), 0, directory).and_then(|present| {
                ensure!(mount_writable(Path::new(path))?, "read-only mount");
                Ok(present)
            }),
            |_| true,
        );
    }
    let existing_user_installation = execution_account.as_ref().is_ok_and(|account| {
        fs::symlink_metadata(account.home.join(".local/share/voyage/install")).is_ok()
    });
    let existing_system_installation = existing_system_service
        || fs::symlink_metadata("/opt/voyage/current").is_ok()
        || fs::symlink_metadata("/var/lib/voyage/vessel").is_ok()
        || fs::symlink_metadata("/var/lib/voyage-gateway").is_ok();
    if existing_system_installation || existing_user_installation {
        blockers.push("existing installation requires a separate exact state, process, endpoint and rollback inventory before adoption".into());
    }
    blockers.push("fresh system installation is an isolated-fixture increment; scope-aware update/rollback, public bound creation and production qualification remain incomplete".into());
    blockers.push("execution home/workspace access, isolation and live voyages need an actual identity and service review before mutation".into());
    Ok(Assessment {
        schema_version: 1,
        scope: "system",
        execution_user: execution_user.into(),
        gateway_user: gateway_user.into(),
        checks,
        blockers,
        existing_user_installation,
        existing_system_installation,
        ready_to_install: false,
    })
}

fn checked_home(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "execution home is not an ordinary directory"
    );
    Ok("present".into())
}

fn bail_result<T>(message: &str) -> Result<T> {
    bail!("{message}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn explicit_account_is_required() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--execution-user".into(), "root".into(), "extra".into()]).is_err());
        assert!(
            parse(&[
                "--execution-user".into(),
                "../root".into(),
                "--gateway-user".into(),
                "voyagegateway".into()
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--execution-user".into(),
                "voyageordinary".into(),
                "--gateway-user".into(),
                "voyageordinary".into()
            ])
            .is_err()
        );
        let args = [
            "--execution-user".into(),
            "voyageordinary".into(),
            "--gateway-user".into(),
            "voyagegateway".into(),
        ];
        let names = parse(&args).unwrap();
        assert_eq!(names.execution_user, "voyageordinary");
        assert_eq!(names.gateway_user, "voyagegateway");
    }

    #[test]
    fn path_assessment_refuses_writable_and_linked_ancestors() {
        let fixture = crate::fixture_tests::Fixture::new();
        let uid = unsafe { libc::geteuid() };
        let root = fixture.root.join("system");
        fs::create_dir(&root).unwrap();
        let file = root.join("unit");
        fs::write(&file, "unit").unwrap();
        assert_eq!(checked_path(&file, uid, false).unwrap(), "present");
        assert!(checked_path(&file, uid, true).is_err());
        fs::hard_link(&file, root.join("other")).unwrap();
        assert!(checked_path(&file, uid, false).is_err());
        fs::remove_file(root.join("other")).unwrap();
        let link = root.join("link");
        symlink(&file, &link).unwrap();
        assert!(checked_path(&link, uid, false).is_err());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o770)).unwrap();
        assert!(checked_path(&file, uid, false).is_err());
    }

    #[test]
    fn namespace_map_requires_one_full_mapping() {
        assert!(full_identity_map("         0          0 4294967295\n"));
        assert!(!full_identity_map("0 100000 65536\n"));
        assert!(!full_identity_map("0 0 4294967295\n1 1 1\n"));
    }
}
