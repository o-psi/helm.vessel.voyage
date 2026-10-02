//! Only the already published, reviewed ordinary candidate may be quiesced after
//! failed activation. General service inspection remains strict about transitions.
use super::{Activation, command, files, unit};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const NAME: &str = "voyage-vessel.service";
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextPin {
    accounts: PathBuf,
    candidate_manifest_sha256: String,
    namespace: BTreeMap<String, String>,
    manager_sha256: String,
    unit_environment_sha256: String,
    overrides_sha256: String,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn selected_environment(bytes: &[u8], separated: bool) -> Result<BTreeMap<String, String>> {
    ensure!(bytes.len() <= 65536, "Managed environment exceeds bound");
    if separated {
        let mut values = BTreeMap::new();
        for entry in bytes.split(|b| *b == 0) {
            let Some(position) = entry.iter().position(|b| *b == b'=') else {
                continue;
            };
            let key = std::str::from_utf8(&entry[..position])?;
            if ["HOME", "XDG_DATA_HOME", "VOYAGE_CREDENTIAL_KEY_FILE"].contains(&key) {
                ensure!(
                    values
                        .insert(
                            key.into(),
                            std::str::from_utf8(&entry[position + 1..])?.into()
                        )
                        .is_none(),
                    "Duplicate process namespace variable"
                );
            }
        }
        return Ok(values);
    }
    // systemctl uses shell quoting only for display. Parse, never execute it;
    // no complete environment or credential value enters a receipt/diagnostic.
    let script = r#"
import json,shlex,sys
try:
 raw=sys.stdin.buffer.read(65537)
 if len(raw)>65536: raise ValueError()
 values={}
 for line in raw.decode().splitlines():
  for word in shlex.split(line):
   key,separator,value=word.partition('=')
   if separator and key in ('HOME','XDG_DATA_HOME','VOYAGE_CREDENTIAL_KEY_FILE'):
    if key in values: raise ValueError()
    values[key]=value
 print(json.dumps(values))
except Exception:
 print('Managed namespace encoding refused',file=sys.stderr);sys.exit(1)
"#;
    Ok(serde_json::from_slice(&command::run(
        Path::new("/usr/bin/python3"),
        &["-I", "-c", script],
        Some(bytes),
    )?)?)
}
fn accounts(values: &BTreeMap<String, String>) -> Result<PathBuf> {
    let home = PathBuf::from(values.get("HOME").context("Managed HOME unavailable")?);
    let data = values
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    ensure!(
        home.is_absolute()
            && data.is_absolute()
            && !data.components().any(|c| matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
        "Noncanonical managed account namespace"
    );
    Ok(data.join("helm"))
}
fn environment(
    layout: &unit::Layout,
    expected_accounts: &Path,
    candidate_manifest_sha256: &str,
) -> Result<ContextPin> {
    unit::check_effective(layout)?;
    let manager = command::systemctl(&["show-environment"])?;
    let effective = command::query("Environment")?;
    let mut namespace = selected_environment(manager.as_bytes(), false)?;
    namespace.extend(selected_environment(effective.as_bytes(), false)?);
    ensure!(
        accounts(&namespace)? == expected_accounts,
        "Managed account namespace changed"
    );
    let paths = command::query("DropInPaths")?;
    ensure!(paths.len() <= 4096, "Managed override path bound exceeded");
    let mut pins = Vec::new();
    for path in paths.split_whitespace() {
        let path = Path::new(path);
        files::check_path(path, layout.uid)?;
        let before = fs::symlink_metadata(path)?;
        ensure!(
            before.is_file() && before.nlink() == 1 && before.len() <= 4096,
            "Managed override identity refused"
        );
        let bytes = crate::install::files::read(path, 4096)?;
        ensure!(
            super::credential_key_path(std::str::from_utf8(&bytes)?).is_some(),
            "Unknown managed service override"
        );
        let after = fs::symlink_metadata(path)?;
        ensure!(
            before.dev() == after.dev()
                && before.ino() == after.ino()
                && before.mode() == after.mode()
                && before.uid() == after.uid()
                && before.len() == after.len()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec()
                && before.ctime() == after.ctime()
                && before.ctime_nsec() == after.ctime_nsec(),
            "Managed override changed while reading"
        );
        pins.push((
            path.to_path_buf(),
            before.dev(),
            before.ino(),
            before.uid(),
            before.mode(),
            digest(&bytes),
        ));
    }
    ensure!(pins.len() <= 16, "Managed override count exceeded");
    Ok(ContextPin {
        accounts: expected_accounts.into(),
        candidate_manifest_sha256: candidate_manifest_sha256.into(),
        namespace,
        manager_sha256: digest(manager.as_bytes()),
        unit_environment_sha256: digest(effective.as_bytes()),
        overrides_sha256: digest(&serde_json::to_vec(&pins)?),
    })
}
fn process_file(pid: u32, name: &str) -> PathBuf {
    #[cfg(test)]
    {
        crate::fixture_tests::root()
            .expect("private process context required")
            .join(format!("process-{pid}-{name}"))
    }
    #[cfg(not(test))]
    {
        PathBuf::from(format!("/proc/{pid}/{name}"))
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Witness {
    pid: u32,
    start: u64,
}
fn start_ticks(stat: &[u8]) -> Result<u64> {
    let start = std::str::from_utf8(stat)?
        .rsplit_once(')')
        .context("Process start identity missing")?
        .1
        .split_whitespace()
        .nth(19)
        .context("Process start identity missing")?
        .parse::<u64>()?;
    ensure!(start > 0, "Invalid process start identity");
    Ok(start)
}
fn image(pid: u32, expected: &Path) -> Result<()> {
    #[cfg(test)]
    let path = crate::fixture_tests::root()
        .unwrap()
        .join(format!("pid-{pid}"));
    #[cfg(not(test))]
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    use std::io::Read;
    let mut actual = fs::File::open(&path)?;
    let meta = actual.metadata()?;
    let wanted = fs::metadata(expected)?;
    ensure!(
        meta.dev() == wanted.dev()
            && meta.ino() == wanted.ino()
            && meta.uid() == wanted.uid()
            && meta.len() == wanted.len()
            && meta.len() <= 512 * 1024 * 1024,
        "Executing image differs from qualified file identity"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    loop {
        let count = actual.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .context("Image read count overflow")?;
        ensure!(total <= meta.len(), "Executing image grew while reading");
        hash.update(&buffer[..count]);
    }
    let after = actual.metadata()?;
    let named = fs::metadata(expected)?;
    let stable = |next: &fs::Metadata| {
        meta.dev() == next.dev()
            && meta.ino() == next.ino()
            && meta.uid() == next.uid()
            && meta.len() == next.len()
            && meta.mode() == next.mode()
            && meta.mtime() == next.mtime()
            && meta.mtime_nsec() == next.mtime_nsec()
            && meta.ctime() == next.ctime()
            && meta.ctime_nsec() == next.ctime_nsec()
    };
    ensure!(
        total == meta.len() && stable(&after) && stable(&named),
        "Executing image grew or changed while reading"
    );
    ensure!(
        format!("{:x}", hash.finalize()) == crate::install::files::hash(expected)?,
        "Executing image bytes changed"
    );
    Ok(())
}
fn process(
    pid: u32,
    bin: &Path,
    state: &Path,
    pin: &ContextPin,
    allow_old_capacity: bool,
) -> Result<Witness> {
    ensure!(pid > 1, "Candidate process identity unavailable");
    #[cfg(test)]
    let executable = crate::fixture_tests::executable(pid)?;
    #[cfg(not(test))]
    let executable = fs::read_link(format!("/proc/{pid}/exe"))?;
    ensure!(
        executable == bin.join("vessel"),
        "Process is not the exact reviewed image"
    );
    #[cfg(test)]
    let owner = fs::metadata(process_file(pid, "environ"))?.uid();
    #[cfg(not(test))]
    let owner = fs::metadata(format!("/proc/{pid}"))?.uid();
    ensure!(owner == unsafe { libc::geteuid() }, "Process owner changed");
    let stat = crate::install::files::read(&process_file(pid, "stat"), 16384)?;
    let start = start_ticks(&stat)?;
    image(pid, &bin.join("vessel"))?;
    let bytes = crate::install::files::read(&process_file(pid, "cmdline"), 65536)?;
    let args = bytes
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .collect::<Vec<_>>();
    let expected = [
        bin.join("vessel"),
        PathBuf::from("local-serve"),
        PathBuf::from("--directory"),
        state.into(),
        PathBuf::from("--voyage-binary"),
        bin.join("voyage"),
    ];
    ensure!(
        (args.len() == 6
            || (allow_old_capacity
                && args.len() == 8
                && args[6] == b"--capacity"
                && args[7] == b"16"))
            && args[..6]
                .iter()
                .zip(&expected)
                .all(|(actual, path)| *actual == path.as_os_str().as_encoded_bytes()),
        "Process arguments differ from reviewed namespace"
    );
    let observed = selected_environment(
        &crate::install::files::read(&process_file(pid, "environ"), 65536)?,
        true,
    )?;
    ensure!(
        observed == pin.namespace && accounts(&observed)? == pin.accounts,
        "Process environment differs from reviewed namespace"
    );
    let after = crate::install::files::read(&process_file(pid, "stat"), 16384)?;
    ensure!(
        start_ticks(&after)? == start,
        "Process start identity changed during observation"
    );
    ensure!(
        crate::install::files::read(&process_file(pid, "cmdline"), 65536)? == bytes,
        "Process arguments changed during observation"
    );
    ensure!(
        selected_environment(
            &crate::install::files::read(&process_file(pid, "environ"), 65536)?,
            true
        )? == observed,
        "Process namespace changed during observation"
    );
    #[cfg(test)]
    let next_owner = fs::metadata(process_file(pid, "environ"))?.uid();
    #[cfg(not(test))]
    let next_owner = fs::metadata(format!("/proc/{pid}"))?.uid();
    ensure!(
        next_owner == owner,
        "Process owner changed during observation"
    );
    image(pid, &bin.join("vessel"))?;
    #[cfg(test)]
    let next = crate::fixture_tests::executable(pid)?;
    #[cfg(not(test))]
    let next = fs::read_link(format!("/proc/{pid}/exe"))?;
    ensure!(
        next == executable,
        "Process image changed during observation"
    );
    Ok(Witness { pid, start })
}
fn layout(prior: &Activation) -> Result<unit::Layout> {
    let layout = unit::Layout::discover()?;
    ensure!(
        layout.state == prior.state,
        "Reviewed service namespace changed"
    );
    Ok(layout)
}
pub(super) fn capture(
    bin: &Path,
    prior: &Activation,
    expected_accounts: &Path,
    candidate: &crate::install::release::Manifest,
) -> Result<ContextPin> {
    ensure!(
        super::systemd::review_activation(bin)? == *prior,
        "Original activation changed before context capture"
    );
    let layout = layout(prior)?;
    let pin = environment(
        &layout,
        expected_accounts,
        &digest(&serde_json::to_vec(candidate)?),
    )?;
    let pid = command::query("MainPID")?.parse()?;
    process(pid, bin, &prior.state, &pin, true)?;
    Ok(pin)
}
fn check(bin: &Path, prior: &Activation, pin: &ContextPin) -> Result<()> {
    let layout = layout(prior)?;
    ensure!(
        unit::existing(&layout)?.as_deref() == Some(unit::render(bin, &prior.state)?.as_str()),
        "Candidate unit changed independently"
    );
    ensure!(
        command::query("UnitFileState")? == prior.unit_file_state,
        "Candidate enablement changed independently"
    );
    let manifest = crate::install::release::Manifest::inspect(bin)?;
    ensure!(
        digest(&serde_json::to_vec(&manifest)?) == pin.candidate_manifest_sha256,
        "Qualified candidate declaration changed"
    );
    manifest.verify(bin.parent().context("Candidate root missing")?)?;
    ensure!(
        environment(&layout, &pin.accounts, &pin.candidate_manifest_sha256)? == *pin,
        "Managed environment or credential override changed independently"
    );
    Ok(())
}
fn state() -> Result<(String, u32, String)> {
    let text = command::systemctl(&["show", NAME, "--property=ActiveState,MainPID,Job"])?;
    ensure!(text.len() <= 4096, "Candidate observation bound exceeded");
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .context("Candidate observation malformed")?;
        ensure!(
            fields.insert(key, value).is_none(),
            "Duplicate candidate observation"
        );
    }
    ensure!(fields.len() == 3, "Candidate observation shape changed");
    Ok((
        fields
            .get("ActiveState")
            .context("Candidate state missing")?
            .to_string(),
        fields
            .get("MainPID")
            .context("Candidate PID missing")?
            .parse()?,
        fields
            .get("Job")
            .context("Candidate job missing")?
            .to_string(),
    ))
}
pub(super) fn quiesce(bin: &Path, prior: &Activation, pin: &ContextPin) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        check(bin, prior, pin)?;
        let (active, pid, _) = state()?;
        ensure!(
            ["active", "activating", "deactivating", "inactive", "failed"]
                .contains(&active.as_str()),
            "Unknown candidate transition"
        );
        ensure!(
            pid != 1 && (pid > 1 || active != "active"),
            "Candidate process identity unavailable"
        );
        if pid > 1 {
            let witness = match process(pid, bin, &prior.state, pin, false) {
                Ok(witness) => witness,
                Err(error) => {
                    if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                        matches!(
                            e.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                        )
                    }) && Instant::now() < deadline
                    {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    return Err(error);
                }
            };
            if state()?.1 != witness.pid {
                ensure!(
                    Instant::now() < deadline,
                    "Candidate identity did not stabilize"
                );
                continue;
            }
            ensure!(
                start_ticks(&crate::install::files::read(
                    &process_file(pid, "stat"),
                    16384
                )?)? == witness.start,
                "Candidate process identity changed before stop"
            );
            image(pid, &bin.join("vessel"))?;
        }
        // The exact owned unit also owns its pending auto-restart/start job.
        // One stop cancels that launch; no PID signal, restart or enable change.
        command::systemctl(&["--no-block", "stop", NAME])?;
        break;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        check(bin, prior, pin)?;
        let (active, pid, job) = state()?;
        if matches!(active.as_str(), "inactive" | "failed")
            && pid == 0
            && (job.is_empty() || job == "0")
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Candidate stop remains unconfirmed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub(super) fn unchanged(prior: &Activation, pin: &ContextPin) -> Result<()> {
    ensure!(
        environment(
            &layout(prior)?,
            &pin.accounts,
            &pin.candidate_manifest_sha256
        )? == *pin,
        "Reviewed environment changed before original activation"
    );
    Ok(())
}

#[cfg(test)]
#[path = "failed_candidate_tests.rs"]
mod tests;
