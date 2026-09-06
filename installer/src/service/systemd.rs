use super::files;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const BINARIES: [&str; 3] = ["helm", "vessel", "voyage"];
const UNIT: &str = "voyage-vessel.service";

fn quoted(path: &Path) -> Result<String> {
    let value = path.to_str().context("Service paths must be valid UTF-8")?;
    if value.chars().any(char::is_control) {
        bail!("Service paths must not contain control characters");
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}

pub fn install(source: &Path, start: bool, dry_run: bool) -> Result<()> {
    // No privilege elevation: installation and execution have the caller's identity.
    let uid = unsafe { libc::geteuid() };
    if uid == 0 || uid != unsafe { libc::getuid() } {
        bail!("Run service installation as the ordinary executing user, without sudo");
    }
    let home = std::env::var_os("HOME").context("HOME is required")?;
    let home = Path::new(&home);
    files::check_path(home, uid)?;
    files::check_path(source, uid)?;
    for name in BINARIES {
        files::executable(&source.join(name), uid)?;
    }
    let state = home.join(".local/state/voyage/vessel");
    let units = home.join(".config/systemd/user");
    let unit_path = units.join(UNIT);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let release = home.join(format!(
        ".local/share/voyage/releases/{stamp}-{}",
        std::process::id()
    ));
    for path in [&state, &units, &release, &unit_path] {
        files::check_path(path, uid)?;
    }
    if unit_path.exists() {
        bail!(
            "{} already exists; preserve it and use an explicit reviewed upgrade",
            unit_path.display()
        );
    }
    let content = format!(
        "[Unit]\nDescription=Voyage Vessel session supervisor\n\n[Service]\nType=simple\nExecStart=:{} local-serve --directory {} --voyage-binary {} --capacity 16\nWorkingDirectory=%h\nUMask=0077\nRestart=on-failure\nRestartSec=2\nKillMode=process\nTimeoutStopSec=30\nStandardInput=null\nStandardOutput=journal\nStandardError=journal\n\n[Install]\nWantedBy=default.target\n",
        quoted(&release.join("vessel"))?,
        quoted(&state)?,
        quoted(&release.join("voyage"))?
    );
    if dry_run {
        println!("{content}");
        println!(
            "Unit: {}\nPrivate state: {}\nActivation requested: {start}",
            unit_path.display(),
            state.display()
        );
        return Ok(());
    }
    files::directory(&state, uid, true)?;
    files::directory(&units, uid, false)?;
    files::directory(&release, uid, true)?;
    let result = (|| -> Result<()> {
        for name in BINARIES {
            files::copy_executable(&source.join(name), &release.join(name))?;
        }
        files::write_new(&unit_path, &content)?;
        Ok(())
    })();
    if let Err(error) = result {
        // Retain the release whenever a published unit may reference it.
        if !unit_path.exists() {
            let _ = fs::remove_dir_all(&release);
        }
        return Err(error.context("Installation failed; existing data is preserved"));
    }
    println!(
        "Installed {}\nBinaries: {}\nState: {}",
        unit_path.display(),
        release.display(),
        state.display()
    );
    if start {
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "--now", UNIT])?;
        systemctl(&["is-active", "--quiet", UNIT])?;
        println!(
            "Vessel service is active. User-service lifetime follows the user manager; boot/logout persistence requires separately configured lingering."
        );
    } else {
        println!(
            "Activate with: systemctl --user daemon-reload && systemctl --user enable --now {UNIT}"
        );
    }
    println!(
        "Stop the supervisor with: systemctl --user stop {UNIT}\nVoyage processes survive supervisor stop; stop individual voyages through Vessel first when draining work."
    );
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl").arg("--user").args(args).status()
        .context("Service files are installed, but systemctl could not run; inspect the user manager and retry activation")?;
    if !status.success() {
        bail!(
            "Service files are installed, but systemctl {args:?} failed ({status}); inspect journalctl --user -u {UNIT} and retry activation"
        );
    }
    Ok(())
}
