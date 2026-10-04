//! Additive macOS ordinary-account process adapter; not supervisor admission.
//!
//! The caller must durably save launch intent/SpawnUncertain before calling
//! `spawn`, and persist subsequent observations. This module never retries a
//! launch, infers readiness from liveness, or recovers a child by PID alone.
//! It does not attest account group authority or runtime-owned resource cleanup.
use super::{Owner, Phase, Registration};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    mem::{MaybeUninit, size_of},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Kernel creation time plus exact process/account identity, not a PID probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub uid: u32,
    pub started_seconds: u64,
    pub started_microseconds: u64,
}
impl ProcessIdentity {
    pub fn registration_value(&self) -> String {
        format!(
            "macos:proc-bsdinfo:{}:{}:{}:{}",
            self.pid, self.uid, self.started_seconds, self.started_microseconds
        )
    }
}

/// Failure (including ESRCH) is unknown, never proof of exit or permission to
/// signal a saved PID. Only a retained Child's wait status proves exit here.
pub fn process_identity(pid: u32, expected_uid: u32) -> Result<ProcessIdentity> {
    ensure!(pid != 0 && pid <= i32::MAX as u32, "invalid macOS PID");
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>();
    // SAFETY: writable buffer with the exact SDK structure size. No fields are
    // read unless libproc reports that it filled the complete structure.
    let count = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    ensure!(
        count == size as i32,
        "macOS creation identity unavailable (proc_pidinfo returned {count})"
    );
    let info = unsafe { info.assume_init() };
    ensure!(
        expected_uid != 0
            && info.pbi_pid == pid
            && info.pbi_uid == expected_uid
            && info.pbi_ruid == expected_uid
            && info.pbi_svuid == expected_uid
            && info.pbi_start_tvsec != 0
            && info.pbi_start_tvusec < 1_000_000,
        "macOS process creation/account identity unprovable"
    );
    Ok(ProcessIdentity {
        pid,
        uid: expected_uid,
        started_seconds: info.pbi_start_tvsec,
        started_microseconds: info.pbi_start_tvusec,
    })
}

/// Owns the unreaped direct child. Dropping this handle does NOT stop the voyage
/// or observe cleanup: the caller must retain it until exit has been collected.
/// No PID-only adoption or automatic kill-on-drop is provided.
#[must_use = "retain the child handle and collect an observed exit"]
pub struct OwnedProcess {
    child: Child,
    registration: Registration,
    identity: Option<ProcessIdentity>,
    exit: Option<ExitStatus>,
}
impl OwnedProcess {
    pub fn registration(&self) -> &Registration {
        &self.registration
    }

    /// Observe creation identity while retaining ownership even when it cannot
    /// be proved. On failure registration remains SpawnUncertain, never Running.
    pub fn observe_running(&mut self) -> Result<&ProcessIdentity> {
        ensure!(self.exit.is_none(), "child already exited");
        let Owner::MacOs { uid, .. } = &self.registration.owner else {
            anyhow::bail!("macOS owner required");
        };
        let identity = process_identity(self.child.id(), *uid)?;
        if let Some(expected) = &self.identity {
            ensure!(&identity == expected, "macOS creation identity changed");
        } else {
            ensure!(
                self.registration.phase == Phase::SpawnUncertain,
                "launch phase changed"
            );
            // Registration::observe_running admits only Intent. Use a clone so
            // no transient Intent state can be exposed as permission to replay.
            let mut observed = self.registration.clone();
            observed.phase = Phase::Intent;
            observed.observe_running(identity.pid, identity.registration_value())?;
            self.registration = observed;
            self.identity = Some(identity);
        }
        Ok(self.identity.as_ref().expect("identity observed"))
    }

    /// Nonblocking waitpid through the retained child. No kill(0) or /proc probe.
    /// An uncertain launch may acquire an exit status but is not promoted into
    /// a proved lifecycle registration without a prior creation observation.
    pub fn observe_exit(&mut self) -> Result<Option<ExitStatus>> {
        if self.exit.is_none() {
            if let Some(status) = self.child.try_wait().context("observe macOS child exit")? {
                self.exit = Some(status);
                if let Some(identity) = &self.identity {
                    self.registration
                        .observe_exit(&identity.registration_value())?;
                }
            }
        }
        Ok(self.exit)
    }

    /// At most 30 seconds per call. Timeout retains the handle and obligation.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Result<Option<ExitStatus>> {
        ensure!(
            timeout <= Duration::from_secs(30),
            "macOS wait exceeds 30-second bound"
        );
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.observe_exit()? {
                return Ok(Some(status));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            thread::sleep(remaining.min(Duration::from_millis(20)));
        }
    }

    /// Explicit force stop of this direct child only, never of a process group.
    /// Identity failure refuses signaling. The unreaped child pins PID ownership;
    /// exited/reaped children are checked before any signal. Runtime descendants
    /// are NOT covered by this operation, and exit is still collected separately.
    pub fn force_stop(&mut self) -> Result<()> {
        if self.observe_exit()?.is_some() {
            return Ok(());
        }
        ensure!(
            self.identity.is_some(),
            "cannot signal unproved macOS launch"
        );
        self.observe_running()?;
        self.child.kill().context("stop owned macOS child")
    }

    /// The boolean is a caller attestation about separately observed runtime
    /// obligations, NOT evidence produced by this process adapter. Child exit
    /// alone cannot establish cleanup of tools, browser, PTYs or descendants.
    pub fn cleanup_observed(&mut self, resource_obligations_empty: bool) -> Result<()> {
        ensure!(
            self.exit.is_some() && self.identity.is_some(),
            "proved child exit required"
        );
        self.registration
            .cleanup_observed(resource_obligations_empty)
    }
}

/// Spawn exactly once under the current non-root account into a new session.
/// `registration` must already be durably recorded as SpawnUncertain, with no
/// PID/identity. The runtime's existing registration/config must be prepared by
/// the caller; this adapter does not create the runtime journal or credentials.
/// A spawn error remains uncertain and must not be automatically retried.
///
/// `directory` must already be an owner-only real directory. The new logfile is
/// exclusive, no-follow and 0600; existing logs are never truncated or reused.
/// The binary digest in Registration is a caller-reviewed value, not an executable
/// pin: protected executable review and ancestors remain caller prerequisites.
pub fn spawn(
    binary: &Path,
    directory: &Path,
    logfile: &Path,
    config: Option<&Path>,
    registration: Registration,
) -> Result<OwnedProcess> {
    registration.validate()?;
    ensure!(
        registration.phase == Phase::SpawnUncertain
            && registration.pid.is_none()
            && registration.process_identity.is_none(),
        "persist fresh uncertain intent before macOS spawn"
    );
    let Owner::MacOs { uid, .. } = &registration.owner else {
        anyhow::bail!("macOS owner required");
    };
    // Same-user ordinary execution only: no setuid or administrator launch.
    ensure!(
        unsafe { libc::getuid() } == *uid && unsafe { libc::geteuid() } == *uid,
        "macOS launch requires the exact current non-root account"
    );
    ensure!(
        binary.is_absolute() && registration.workspace.is_absolute(),
        "absolute binary/workspace required"
    );
    ensure!(
        directory.is_absolute() && logfile.parent() == Some(directory),
        "log must be directly inside the private runtime directory"
    );
    if let Some(config) = config {
        ensure!(
            config.is_absolute() && config.is_file(),
            "absolute host config file required"
        );
    }
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == *uid && metadata.mode() & 0o077 == 0,
        "private owner-only runtime directory required"
    );
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(logfile)
        .context("create private macOS voyage logfile")?;
    let log_metadata = log.metadata()?;
    ensure!(
        log_metadata.uid() == *uid && log_metadata.mode() & 0o077 == 0,
        "log is not private"
    );
    let mut command = Command::new(binary);
    command
        .arg("serve")
        .arg("--directory")
        .arg(directory)
        .arg("--session")
        .arg(registration.session_id.to_string())
        .arg("--incarnation")
        .arg(registration.incarnation.to_string())
        .arg("--workspace")
        .arg(&registration.workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    if let Some(config) = config {
        command.arg("--config").arg(config);
    }
    // SAFETY: only async-signal-safe system calls after fork; no allocations or
    // locks. No terminal/parent lifetime coupling and private child-created files.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::umask(0o077);
            Ok(())
        });
    }
    let child = command.spawn().context("spawn independent macOS voyage")?;
    Ok(OwnedProcess {
        child,
        registration,
        identity: None,
        exit: None,
    })
}
