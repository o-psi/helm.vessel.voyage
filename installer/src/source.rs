//! Acquire one pinned upgrade source before review; publication stays transactional.
use anyhow::Result;
use std::{path::PathBuf, sync::atomic::AtomicBool};

#[derive(Clone, Copy)]
pub enum Source {
    Latest,
    Main,
}

pub struct Prepared {
    pub bin_dir: PathBuf,
    pub description: String,
    root: PathBuf,
    retain: bool,
}
static CLEANUP_FAILURES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn cleanup_result() -> Result<()> {
    let failures = CLEANUP_FAILURES.lock().unwrap_or_else(|e| e.into_inner());
    anyhow::ensure!(
        failures.is_empty(),
        "Temporary source cleanup failed: {}",
        failures.join("; ")
    );
    Ok(())
}

impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.retain {
            // Only our uniquely created staging directory, never the user's checkout.
            if let Err(error) = std::fs::remove_dir_all(&self.root)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                CLEANUP_FAILURES
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(format!("{}: {error}", self.root.display()));
            }
        }
    }
}

pub fn prepare(source: Source, cancelled: &AtomicBool) -> Result<Prepared> {
    #[cfg(target_os = "linux")]
    {
        linux::prepare(source, cancelled)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, cancelled);
        anyhow::bail!("Automatic upgrades currently support Linux only")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use std::{
        fs::{self, OpenOptions},
        os::unix::{fs::OpenOptionsExt, process::CommandExt},
        process::{Child, Command, Stdio},
        sync::atomic::Ordering,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn active(group: u32) -> Result<bool> {
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
            {
                continue;
            }
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((_, fields)) = stat.rsplit_once(") ") else {
                continue;
            };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if fields.len() > 2
                && fields[2].parse::<u32>() == Ok(group)
                && !matches!(fields[0], "Z" | "X")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn stop(child: &mut Child) -> Result<()> {
        for signal in [libc::SIGTERM, libc::SIGKILL] {
            unsafe {
                libc::kill(-(child.id() as i32), signal);
            }
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                child.try_wait()?;
                if !active(child.id())? {
                    child.wait()?;
                    return Ok(());
                }
                if Instant::now() >= until {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        bail!("Upgrade subprocess cleanup is unconfirmed")
    }

    pub(super) fn prepare(source: Source, cancelled: &AtomicBool) -> Result<Prepared> {
        ensure!(!cancelled.load(Ordering::Relaxed), "Upgrade cancelled");
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is required")?);
        let cache = home.join(".cache/voyage/upgrades");
        crate::install::files::private_directory(&cache)?;
        let root = cache.join(format!(
            "prepare-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let mut prepared = Prepared {
            bin_dir: PathBuf::new(),
            description: String::new(),
            root,
            retain: false,
        };
        let log_path = prepared.root.join("acquire.log");
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log_path)?;
        let mut command = Command::new("python3");
        command
            .env_clear()
            .env("HOME", &home)
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()),
            )
            .env("LANG", "C.UTF-8")
            .env(
                "RUSTUP_HOME",
                std::env::var_os("RUSTUP_HOME")
                    .unwrap_or_else(|| home.join(".rustup").into_os_string()),
            )
            .args(["-I", "-c", include_str!("source_acquire.py")])
            .arg(match source {
                Source::Latest => "latest",
                Source::Main => "main",
            })
            .arg(&prepared.root)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .current_dir(&prepared.root)
            .process_group(0);
        let mut child = command
            .spawn()
            .context("Automatic upgrades require Python 3.11+; could not start acquisition")?;
        let deadline = Instant::now()
            + Duration::from_secs(match source {
                Source::Latest => 1000,
                Source::Main => 4200,
            });
        let result = (|| -> Result<()> {
            loop {
                ensure!(!cancelled.load(Ordering::Relaxed), "Upgrade cancelled");
                ensure!(Instant::now() < deadline, "Upgrade acquisition timed out");
                ensure!(
                    fs::metadata(&log_path)?.len() <= 64 * 1024 * 1024,
                    "Upgrade diagnostics exceeded 64 MiB"
                );
                if let Some(status) = child.try_wait()? {
                    if !status.success() {
                        let detail = fs::read_to_string(prepared.root.join("error.txt")).unwrap_or_else(|_| "Acquisition stopped unexpectedly; check Python, curl, Git and Rust prerequisites.".into());
                        let detail: String = detail
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(2048)
                            .collect();
                        bail!("{detail}");
                    }
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })();
        if let Err(error) = stop(&mut child) {
            prepared.retain = true;
            CLEANUP_FAILURES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(format!(
                    "Subprocess cleanup unconfirmed; retained {}",
                    prepared.root.display()
                ));
            bail!(
                "{error:#}; retained staging at {}. Stop remaining work before removing it.",
                prepared.root.display()
            );
        }
        result?;
        #[derive(serde::Deserialize)]
        struct Metadata {
            bin_dir: PathBuf,
            description: String,
        }
        let bytes = crate::install::files::read(&prepared.root.join("prepared.json"), 65536)?;
        let metadata: Metadata = serde_json::from_slice(&bytes)?;
        ensure!(
            metadata.bin_dir.starts_with(&prepared.root),
            "Prepared source escaped staging"
        );
        crate::install::release::Manifest::inspect(&metadata.bin_dir)?;
        prepared.bin_dir = metadata.bin_dir;
        prepared.description = metadata.description;
        Ok(prepared)
    }
}

/// Own cancellation flags during preparation/review. Publication deliberately
/// finishes its bounded transaction rather than interrupting between effects.
/// Unregistering removes our callbacks, not signal-hook's process-wide handler.
pub struct Cancellation {
    pub flag: std::sync::Arc<AtomicBool>,
    #[cfg(unix)]
    signals: Vec<signal_hook::SigId>,
}
impl Cancellation {
    pub fn new() -> Result<Self> {
        #[allow(unused_mut)]
        let mut guard = Self {
            flag: std::sync::Arc::new(AtomicBool::new(false)),
            #[cfg(unix)]
            signals: Vec::new(),
        };
        #[cfg(unix)]
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            guard
                .signals
                .push(signal_hook::flag::register(signal, guard.flag.clone())?);
        }
        Ok(guard)
    }
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        #[cfg(unix)]
        for id in self.signals.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}
