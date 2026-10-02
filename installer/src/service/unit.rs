//! Exact managed unit templates and user-manager path validation.
use super::{command, files};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub(super) const NAME: &str = "voyage-vessel.service";
pub(super) struct Layout {
    pub uid: u32,
    pub state: PathBuf,
    pub units: PathBuf,
    pub unit: PathBuf,
}
impl Layout {
    pub fn discover() -> Result<Self> {
        #[cfg(test)]
        if let Some(root) = crate::fixture_tests::required_root()? {
            let units = root.join("units");
            return Ok(Self {
                uid: unsafe { libc::geteuid() },
                state: root.join("state"),
                unit: units.join(NAME),
                units,
            });
        }
        let uid = unsafe { libc::geteuid() };
        ensure!(
            uid != 0 && uid == unsafe { libc::getuid() },
            "Run service installation as the ordinary user, without sudo"
        );
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is required")?);
        files::check_path(&home, uid)?;
        let state = base("XDG_STATE_HOME", home.join(".local/state"))?.join("voyage/vessel");
        let units = base("XDG_CONFIG_HOME", home.join(".config"))?.join("systemd/user");
        let unit = units.join(NAME);
        for path in [&state, &units, &unit] {
            files::check_path(path, uid)?;
        }
        ensure!(
            state
                .join("sessions/00000000-0000-0000-0000-000000000000/runtime.sock")
                .as_os_str()
                .as_encoded_bytes()
                .len()
                < 108,
            "Vessel state path exceeds Linux Unix socket limit"
        );
        Ok(Self {
            uid,
            state,
            units,
            unit,
        })
    }
}
fn base(key: &str, fallback: PathBuf) -> Result<PathBuf> {
    let path = std::env::var_os(key).map(PathBuf::from).unwrap_or(fallback);
    ensure!(path.is_absolute(), "{key} must be absolute");
    Ok(path)
}
fn quote(path: &Path) -> Result<String> {
    let value = path.to_str().context("service paths require UTF-8")?;
    ensure!(
        !value.chars().any(char::is_control),
        "service path contains control characters"
    );
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}
pub(super) fn render(bin: &Path, state: &Path) -> Result<String> {
    Ok(format!(
        "[Unit]\nDescription=Voyage Vessel session supervisor\n\n[Service]\nType=simple\nExecStart=:{} local-serve --directory {} --voyage-binary {}\nWorkingDirectory=%h\nUMask=0077\nRestart=on-failure\nRestartSec=2\nKillMode=process\nTimeoutStopSec=30\nStandardInput=null\nStandardOutput=journal\nStandardError=journal\n\n[Install]\nWantedBy=default.target\n",
        quote(&bin.join("vessel"))?,
        quote(state)?,
        quote(&bin.join("voyage"))?
    ))
}
fn quoted(input: &str) -> Result<(PathBuf, &str)> {
    let mut chars = input
        .strip_prefix('"')
        .context("unrecognized service path")?
        .char_indices();
    let mut value = String::new();
    while let Some((offset, c)) = chars.next() {
        match c {
            '"' => {
                return Ok((
                    PathBuf::from(value.replace("%%", "%")),
                    &input[offset + 2..],
                ));
            }
            '\\' => {
                let (_, escaped) = chars.next().context("invalid service escape")?;
                ensure!(matches!(escaped, '\\' | '"'), "unrecognized service escape");
                value.push(escaped)
            }
            _ => value.push(c),
        }
    }
    anyhow::bail!("unterminated service path")
}
pub(super) fn recognized(content: &str, state: &Path) -> Result<PathBuf> {
    let line = content
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart=:"))
        .context("unrecognized service executable")?;
    let (vessel, rest) = quoted(line)?;
    let (saved_state, rest) = quoted(
        rest.strip_prefix(" local-serve --directory ")
            .context("unrecognized service arguments")?,
    )?;
    let (voyage, rest) = quoted(
        rest.strip_prefix(" --voyage-binary ")
            .context("unrecognized runtime arguments")?,
    )?;
    let bin = vessel.parent().context("service binary parent missing")?;
    ensure!(
        (rest.is_empty() || rest == " --capacity 16")
            && vessel.file_name().is_some_and(|n| n == "vessel")
            && voyage == bin.join("voyage")
            && saved_state == state
            && (render(bin, state)? == content
                || render(bin, state)?
                    .replace("\nWorkingDirectory=", " --capacity 16\nWorkingDirectory=")
                    == content),
        "Refusing unrecognized service edits or a different state directory"
    );
    Ok(bin.to_owned())
}
pub(super) fn existing(layout: &Layout) -> Result<Option<String>> {
    if !layout.unit.exists() {
        return Ok(None);
    }
    let meta = fs::symlink_metadata(&layout.unit)?;
    ensure!(
        meta.is_file() && meta.len() <= 16384,
        "invalid service unit"
    );
    let content = fs::read_to_string(&layout.unit)?;
    recognized(&content, &layout.state)?;
    Ok(Some(content))
}
pub(super) fn check_effective(layout: &Layout) -> Result<()> {
    let search = command::systemctl(&["show", "--value", "--property", "UnitPath"])?;
    ensure!(
        search
            .split_whitespace()
            .any(|path| Path::new(path) == layout.units),
        "XDG_CONFIG_HOME unit directory is not in the running user manager's search path; align the user manager environment explicitly"
    );
    let overrides = command::query("DropInPaths")?;
    for path in overrides.split_whitespace() {
        let path = Path::new(path);
        files::check_path(path, layout.uid)?;
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= 4096,
            "Invalid credential environment override"
        );
        ensure!(
            credential_environment(&fs::read_to_string(path)?),
            "Refusing service behavior overrides; only the credential key-file environment is supported"
        );
    }
    let fragment = command::query("FragmentPath")?;
    ensure!(
        fragment.is_empty() || Path::new(&fragment) == layout.unit,
        "Refusing a service loaded from another unit path"
    );
    if !fragment.is_empty() {
        ensure!(
            command::query("KillMode")? == "process",
            "Effective KillMode must be process to preserve voyages"
        );
        for property in ["ExecStop", "ExecStopPost"] {
            ensure!(
                command::query(property)?.is_empty(),
                "Refusing custom service stop commands"
            );
        }
    }
    Ok(())
}

// Credential location is deployment configuration, not a service behavior override.
// Preserve this exact narrow form; never allow PATH, loaders, Exec*, stop or kill
// directives through the installer compatibility exception.
fn credential_environment(content: &str) -> bool {
    credential_key_path(content).is_some()
}
pub(super) fn credential_key_path(content: &str) -> Option<&str> {
    let mut section = false;
    let mut environment = None;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if line == "[Service]" && !section {
            section = true;
            continue;
        }
        let value = line.strip_prefix("Environment=")?;
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        let path = value.strip_prefix("VOYAGE_CREDENTIAL_KEY_FILE=")?;
        if !section
            || environment.is_some()
            || !path.starts_with('/')
            || path.split('/').any(|part| matches!(part, "." | ".."))
            || !path
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/_-.".contains(&c))
        {
            return None;
        }
        environment = Some(path);
    }
    if section { environment } else { None }
}

#[cfg(test)]
#[path = "unit_tests.rs"]
mod tests;
