//! User launchd lifecycle. No administrator enrollment or voyage process ownership.
use super::files;
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const LABEL: &str = "org.voyage.vessel";
struct Layout {
    uid: u32,
    home: PathBuf,
    state: PathBuf,
    unit: PathBuf,
}
impl Layout {
    fn discover() -> Result<Self> {
        let uid = unsafe { libc::geteuid() };
        ensure!(
            uid != 0 && uid == unsafe { libc::getuid() },
            "Run as ordinary user without sudo"
        );
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME required")?);
        files::check_path(&home, uid)?;
        let state = home.join(".local/share/voyage/vessel");
        let unit = home.join("Library/LaunchAgents/org.voyage.vessel.plist");
        files::check_path(&state, uid)?;
        files::check_path(&unit, uid)?;
        Ok(Self {
            uid,
            home,
            state,
            unit,
        })
    }
    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }
    fn target(&self) -> String {
        format!("{}/{LABEL}", self.domain())
    }
}
fn xml(path: &Path) -> Result<String> {
    ensure!(path.is_absolute(), "Service paths must be absolute");
    let s = path.to_str().context("Service paths require UTF-8")?;
    ensure!(
        !s.chars().any(char::is_control),
        "Control characters in service path"
    );
    Ok(s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
fn render(bin: &Path, l: &Layout) -> Result<String> {
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>local-serve</string><string>--directory</string><string>{}</string><string>--voyage-binary</string><string>{}</string></array><key>WorkingDirectory</key><string>{}</string><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>Umask</key><integer>63</integer><key>AbandonProcessGroup</key><true/></dict></plist>\n",
        xml(&bin.join("vessel"))?,
        xml(&l.state)?,
        xml(&bin.join("voyage"))?,
        xml(&l.home)?
    ))
}
// Output is discarded: command completion is bounded and status is the observation.
fn command(args: &[&str]) -> Result<bool> {
    let mut child = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("launchctl timed out; effect uncertain, inspect before retry");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn loaded(l: &Layout) -> Result<bool> {
    command(&["print", &l.target()])
}
pub fn preview(bin: &Path, start: bool) -> Result<String> {
    let l = Layout::discover()?;
    let content = render(bin, &l)?;
    Ok(format!(
        "User launchd definition {} (start={start}):\n{content}",
        l.unit.display()
    ))
}
pub fn configure(bin: &Path, start: bool, dry_run: bool) -> Result<()> {
    let l = Layout::discover()?;
    let content = render(bin, &l)?;
    if dry_run {
        println!("{}", preview(bin, start)?);
        return Ok(());
    }
    // Current Vessel gates its process supervisor to Linux. A generated plist
    // cannot make a non-Linux runtime supported. Refuse before filesystem or
    // launchctl effects until the native supervisor contract exists.
    ensure!(
        !start,
        "Native Vessel process supervision is not implemented; refusing launchd activation"
    );
    files::executable(&bin.join("vessel"), l.uid)?;
    files::executable(&bin.join("voyage"), l.uid)?;
    // Never replace an active/unrecognized definition as an implicit upgrade.
    if l.unit.try_exists()? {
        ensure!(
            crate::install::files::read(&l.unit.with_extension("receipt"), 64 * 1024)?
                == content.as_bytes()
                && read_unit(&l)? == content,
            "launchd definition changed; stop/uninstall reviewed prior service before switching binaries"
        );
    } else {
        ensure!(!loaded(&l)?, "Unrecognized loaded launchd service");
        files::directory(
            l.unit.parent().context("No LaunchAgents parent")?,
            l.uid,
            false,
        )?;
        files::replace(&l.unit, &content, None)?;
        files::replace(&l.unit.with_extension("receipt"), &content, None)?;
    }
    files::directory(&l.state, l.uid, true)?;
    if start && !loaded(&l)? {
        ensure!(
            command(&[
                "bootstrap",
                &l.domain(),
                l.unit.to_str().context("UTF-8 required")?
            ])?,
            "launchd bootstrap refused; inspect service"
        );
    }
    if start {
        // Authenticated catalogue verifies the candidate protocol and state, not
        // merely launchd registration. No observation starts another supervisor.
        let mut child = Command::new(bin.join("helm"))
            .args(["connect", "--no-start", "--directory"])
            .arg(&l.state)
            .arg("list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(
                    status.success(),
                    "Native authenticated readiness refused; candidate may have accessed state, inspect before rollback"
                );
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "Native readiness timed out; candidate state access uncertain, inspect before rollback"
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    Ok(())
}
pub fn manage(operation: &str) -> Result<()> {
    let l = Layout::discover()?;
    if operation != "service-status" {
        let expected = crate::install::files::read(&l.unit.with_extension("receipt"), 64 * 1024)?;
        ensure!(
            read_unit(&l)?.as_bytes() == expected,
            "Native service definition differs from exact installation receipt"
        );
    }
    match operation {
        "service-status" => {
            println!(
                "launchd registration loaded: {}. Individual voyage liveness/readiness requires authenticated runtime inspection.",
                loaded(&l)?
            );
        }
        "service-stop" => {
            ensure!(l.unit.try_exists()?, "No installed launchd definition");
            ensure!(
                command(&["bootout", &l.target()])?,
                "launchd stop refused; inspect outcome"
            );
            ensure!(!loaded(&l)?, "launchd registration remains loaded");
        }
        "service-uninstall" => {
            ensure!(!loaded(&l)?, "Stop supervisor before uninstalling service");
            let previous = read_unit(&l)?;

            files::remove_reviewed(&l.unit, &previous)?;
            println!("Service removed; binaries, credentials and voyage state retained.");
        }
        _ => bail!("Unknown launchd service operation"),
    }
    Ok(())
}

fn read_unit(l: &Layout) -> Result<String> {
    files::check_path(&l.unit, l.uid)?;
    let bytes = crate::install::files::read(&l.unit, 64 * 1024)?;
    String::from_utf8(bytes).context("launchd definition is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_is_exact_argument_vector_and_preserves_voyage_process_group() {
        let l = Layout {
            uid: 1000,
            home: PathBuf::from("/Users/test"),
            state: PathBuf::from("/Users/test/state"),
            unit: PathBuf::from("/Users/test/LaunchAgents/unit"),
        };
        let text = render(Path::new("/Users/test/release & one/bin"), &l).unwrap();
        assert!(text.contains("release &amp; one/bin/vessel"));
        assert!(text.contains("<key>AbandonProcessGroup</key><true/>"));
        assert!(text.contains("<key>Umask</key><integer>63</integer>"));
        assert!(text.contains("<string>--voyage-binary</string>"));
        assert!(!text.contains("sh -c"));
    }
    #[test]
    fn paths_refuse_relative_and_controls() {
        assert!(xml(Path::new("relative")).is_err());
        assert!(xml(Path::new("/unsafe\npath")).is_err());
        assert_eq!(
            xml(Path::new(r#"/safe<&"'"#)).unwrap(),
            "/safe&lt;&amp;&quot;&apos;"
        );
    }
}
