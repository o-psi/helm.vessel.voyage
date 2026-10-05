//! Explicit local-owner forward recovery of one proved mixed legacy installation.
//! Original unknown receipts and canonical state are retained; no old apply or DB restore.
use super::*;
use sha2::{Digest, Sha256};
use std::{os::unix::fs::MetadataExt, path::Path};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    schema_version: u32,
    operation_id: String,
    original_operation: String,
    original_sha256: String,
    old_release: String,
    metadata_sha256: String,
    running_release: String,
    running_manifest_sha256: String,
    target_release: String,
    target_manifest_sha256: String,
    staged_bin: PathBuf,
    activation: service::Activation,
    accounts: PathBuf,
    source_evidence: serde_json::Value,
    gateway_unit: String,
    gateway_path: PathBuf,
    gateway_original: String,
    gateway_repaired: String,
    gateway_effective: String,
    gateway_credentials: GatewayCredentials,
    gateway_enablement: String,
    public_origin: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    review: Review,
    phase: String,
    proof: Option<crate::legacy::Proof>,
}
fn directory() -> Result<PathBuf> {
    let path = installation_root()?.join("recoveries");
    files::private_directory(&path)?;
    Ok(path)
}
fn record_path(operation: &str) -> Result<PathBuf> {
    id(operation)?;
    Ok(directory()?.join(format!("{operation}.json")))
}
fn save_recovery(record: &mut Recovery, phase: &str) -> Result<()> {
    record.phase = phase.into();
    files::atomic_json(&record_path(&record.review.operation_id)?, record)
}
fn bounded_hash(path: &Path, limit: u64) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(files::read(path, limit)?)))
}
fn review_hash(review: &Review) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(review)?)))
}
fn release(identity: &str) -> Result<PathBuf> {
    ensure!(
        identity.len() == 64
            && identity
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "Invalid pinned release identity"
    );
    let root = installation_root()?.join("releases").join(identity);
    let manifest = installed_manifest(identity)?;
    manifest.verify(&root)?;
    Ok(root)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedFormats {
    catalogue_schema: u32,
    journal_schemas: Vec<u32>,
    process_protocols: Vec<u32>,
}

/// Current source-owned formats, never guessed readers for an undeclared source.
fn current_forward_formats() -> Result<std::collections::BTreeMap<String, Vec<u32>>> {
    fn normalized(source: &str) -> String {
        source.chars().filter(|c| !c.is_whitespace()).collect()
    }
    fn source_constant(source: &str, name: &str) -> Result<u32> {
        let source = normalized(source);
        let marker = format!("const{name}:i64=");
        ensure!(
            source.matches(&marker).count() == 1,
            "Recovery writer source contract changed"
        );
        Ok(source
            .split_once(&marker)
            .unwrap()
            .1
            .split(';')
            .next()
            .unwrap()
            .parse()?)
    }
    fn catalogue_values(name: &str) -> Result<Vec<u32>> {
        let source = normalized(include_str!("../../../vessel/src/process/database.rs"));
        let marker = format!("const{name}:&[i64]=&[");
        ensure!(
            source.matches(&marker).count() == 1,
            "Recovery catalogue source contract changed"
        );
        source
            .split_once(&marker)
            .unwrap()
            .1
            .split("];")
            .next()
            .unwrap()
            .split(',')
            .filter(|value| !value.is_empty())
            .map(|value| value.parse().map_err(Into::into))
            .collect()
    }
    let journal = include_str!("../../../voyage/src/attachment/journal.rs");
    let writer = source_constant(journal, "SCHEMA_VERSION")?;
    let source = normalized(journal);
    let marker = "matches!(version,";
    ensure!(
        source.matches(marker).count() == 1,
        "Recovery journal reader source contract changed"
    );
    let read = source
        .split_once(marker)
        .unwrap()
        .1
        .split(')')
        .next()
        .unwrap()
        .split('|')
        .map(|value| {
            if value == "SCHEMA_VERSION" {
                Ok(writer)
            } else {
                value.parse().map_err(Into::into)
            }
        })
        .collect::<Result<Vec<u32>>>()?;
    Ok(std::collections::BTreeMap::from([
        (
            "catalogue_read".into(),
            catalogue_values("CATALOGUE_READ_SCHEMAS")?,
        ),
        (
            "catalogue_write".into(),
            catalogue_values("CATALOGUE_WRITE_SCHEMAS")?,
        ),
        ("journal_read".into(), read),
        ("journal_write".into(), vec![writer]),
        (
            "process_protocol".into(),
            vec![voyage_protocol::process::PROCESS_PROTOCOL],
        ),
        (
            "vessel_protocol".into(),
            vec![voyage_protocol::vessel::VESSEL_API_VERSION],
        ),
        (
            "execution_identity".into(),
            vec![voyage_protocol::execution_identity::EXECUTION_SCHEMA],
        ),
    ]))
}

fn forward_formats(
    installed: &Manifest,
    candidate: &Manifest,
    evidence: &serde_json::Value,
    executing_installer_sha256: &str,
) -> Result<()> {
    installed.validate()?;
    candidate.validate()?;
    if installed.update_compatibility.is_some() {
        return rollback_formats(installed, candidate);
    }
    let next = candidate
        .update_compatibility
        .as_ref()
        .context("Forward target format declaration missing")?;
    ensure!(
        candidate.binaries["voyage-installer"].sha256 == executing_installer_sha256,
        "Undeclared-source forward recovery requires the exact current target installer"
    );
    ensure!(
        evidence["recovery_mode"] == "forward-existing-schema2",
        "Exact forward-only evidence required"
    );
    let observed: ObservedFormats = serde_json::from_value(evidence["observed_formats"].clone())?;
    let sessions = evidence["session_count"]
        .as_u64()
        .context("Observed session count missing")?;
    ensure!(
        sessions <= 4096
            && observed.catalogue_schema == 2
            && observed.journal_schemas.len() <= 64
            && observed
                .journal_schemas
                .windows(2)
                .all(|values| values[0] < values[1])
            && observed.journal_schemas.iter().all(|value| *value >= 2)
            && if sessions == 0 {
                observed.journal_schemas.is_empty() && observed.process_protocols.is_empty()
            } else {
                !observed.journal_schemas.is_empty()
                    && observed.process_protocols == [voyage_protocol::process::PROCESS_PROTOCOL]
            },
        "Observed ordinary forward formats are unsupported or malformed"
    );
    let current = current_forward_formats()?;
    ensure!(
        next.formats == current,
        "Forward target differs from the current recovery writer/read contract"
    );
    ensure!(
        next.formats["catalogue_read"].contains(&observed.catalogue_schema)
            && next.formats["catalogue_write"].contains(&observed.catalogue_schema)
            && observed
                .journal_schemas
                .iter()
                .all(|version| next.formats["journal_read"].contains(version)),
        "Forward target cannot read the exact observed formats"
    );
    Ok(())
}

fn original_unchanged(review: &Review) -> Result<()> {
    ensure!(
        bounded_hash(&path(&review.original_operation)?, 65536)? == review.original_sha256,
        "Original uncertain receipt changed"
    );
    Ok(())
}
fn gateway_arguments(argv: &[u8], state: &Path, origin: &str) -> Result<()> {
    let args = argv
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();
    ensure!(
        args.iter()
            .filter(|arg| **arg == b"--process-directory")
            .count()
            == 1,
        "Gateway private route is ambiguous"
    );
    let position = args
        .iter()
        .position(|arg| *arg == b"--process-directory")
        .unwrap();
    ensure!(
        args.get(position + 1).copied() == Some(state.as_os_str().as_encoded_bytes()),
        "Gateway state namespace changed"
    );
    ensure!(
        !args.contains(&b"--system-gateway-socket".as_slice())
            && !args.contains(&b"--allow-insecure-loopback".as_slice()),
        "Recovery supports the reviewed ordinary HTTPS gateway only"
    );
    let flags = args
        .iter()
        .filter(|arg| **arg == b"--public-origin")
        .count();
    ensure!(
        flags <= 1 && args.iter().filter(|arg| **arg == origin.as_bytes()).count() == 1,
        "Gateway origin is ambiguous"
    );
    if flags == 1 {
        let position = args
            .iter()
            .position(|arg| *arg == b"--public-origin")
            .unwrap();
        ensure!(
            args.get(position + 1).copied() == Some(origin.as_bytes()),
            "Gateway origin flag changed"
        );
    } else {
        ensure!(
            args.last().copied() == Some(origin.as_bytes()),
            "Recovery requires the exact old positional origin at end"
        );
    }
    Ok(())
}
fn gateway_repair(
    content: &str,
    executable: &Path,
    persistent: &Path,
    origin: &str,
) -> Result<String> {
    let url = url::Url::parse(origin)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "Canonical reviewed HTTPS origin required"
    );
    ensure!(
        !origin.chars().any(|c| c.is_whitespace()
            || c.is_control()
            || c == '\"'
            || c == '%'
            || c == '\\'),
        "Unsupported origin escaping"
    );
    let exe = executable.to_str().context("Gateway executable encoding")?;
    let next = persistent.to_str().context("Gateway executable encoding")?;
    ensure!(
        ![exe, next].iter().any(|value| value
            .chars()
            .any(|c| c.is_whitespace() || c == '%' || c == '\"' || c == '\\')),
        "Unsupported gateway path escaping"
    );
    let commands = content
        .lines()
        .filter(|line| line.starts_with("ExecStart="))
        .collect::<Vec<_>>();
    ensure!(
        commands.len() == 1,
        "One reviewed gateway ExecStart is required"
    );
    let line = commands[0];
    let command = line.strip_prefix("ExecStart=").unwrap();
    let (prefix, command) = if let Some(rest) = command.strip_prefix(':') {
        ("ExecStart=:", rest)
    } else {
        ("ExecStart=", command)
    };
    let rest = command
        .strip_prefix(&format!("{exe} "))
        .or_else(|| command.strip_prefix(&format!("\"{exe}\" ")))
        .context("Gateway command does not match live immutable release")?;
    let origin_token = format!(" {origin}");
    ensure!(
        rest.matches(origin).count() == 1,
        "Gateway unit origin is ambiguous"
    );
    let repaired = if rest.contains(&format!("--public-origin {origin}")) {
        rest.to_owned()
    } else {
        let head = rest
            .strip_suffix(&origin_token)
            .context("Unsupported positional gateway origin")?;
        format!("{head} --public-origin {origin}")
    };
    Ok(content.replacen(line, &format!("{prefix}{next} {repaired}"), 1))
}
fn owned_process(pid: u32) -> Result<()> {
    #[cfg(test)]
    let path = home()?.join(format!("pid-{pid}"));
    #[cfg(not(test))]
    let path = PathBuf::from(format!("/proc/{pid}"));
    ensure!(
        pid > 1 && fs::metadata(path)?.uid() == unsafe { libc::geteuid() },
        "Recovery process UID does not match ordinary owner"
    );
    Ok(())
}
fn only_reviewed_services(running: &Path, gateway: &str) -> Result<()> {
    let units: Vec<serde_json::Value> = serde_json::from_str(&systemctl(&[
        "list-units",
        "--type=service",
        "--state=running",
        "--output=json",
    ])?)?;
    ensure!(units.len() <= 128, "Running user-service bound exceeded");
    for unit in units {
        let name = unit["unit"].as_str().context("Running unit name missing")?;
        if name == "voyage-vessel.service" || name == gateway {
            continue;
        }
        let pid: u32 = systemctl(&["show", name, "--property=MainPID", "--value"])?
            .trim()
            .parse()?;
        ensure!(
            pid == 0 || process_executable(pid)? != running.join("bin/vessel"),
            "Additional running candidate gateway requires explicit recovery review"
        );
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CredentialDropIn {
    path: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    mode: u32,
    sha256: String,
    key_path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GatewayCredentials {
    effective_environment: String,
    drop_in: Option<CredentialDropIn>,
}
fn credential_drop_in(path: &Path, gateway_path: &Path) -> Result<CredentialDropIn> {
    credential_drop_in_for_owner(path, gateway_path, unsafe { libc::geteuid() })
}
fn credential_drop_in_for_owner(
    path: &Path,
    gateway_path: &Path,
    owner: u32,
) -> Result<CredentialDropIn> {
    ensure!(
        path.parent() == Some(Path::new(&format!("{}.d", gateway_path.display()))),
        "Gateway credential override must be in its exact owned unit directory"
    );
    files::safe(path)?;
    let before = fs::symlink_metadata(path)?;
    ensure!(
        before.is_file()
            && before.uid() == owner
            && before.nlink() == 1
            && before.mode() & 0o777 == 0o600,
        "Gateway credential override must be private owned regular file"
    );
    let bytes = files::read(path, 4096)?;
    let content = std::str::from_utf8(&bytes)?;
    let key_path = service::credential_key_path(content).context("Unsupported gateway override; only maintained credential key-file environment is admitted")?.to_owned();
    let after = fs::symlink_metadata(path)?;
    ensure!(
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.uid() == after.uid()
            && before.mode() == after.mode()
            && before.nlink() == after.nlink()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        "Gateway credential override changed while reading"
    );
    Ok(CredentialDropIn {
        path: path.into(),
        device: before.dev(),
        inode: before.ino(),
        uid: before.uid(),
        mode: before.mode(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
        key_path,
    })
}
fn reviewed_gateway_credentials(
    paths: &str,
    gateway_path: &Path,
    effective_environment: &str,
    live_environment: &[u8],
    recovery_key: Option<&std::ffi::OsStr>,
) -> Result<GatewayCredentials> {
    ensure!(
        paths.len() <= 4096
            && effective_environment.len() <= 8192
            && live_environment.len() <= 1024 * 1024,
        "Gateway credential context exceeds bound"
    );
    let paths = paths.split_whitespace().collect::<Vec<_>>();
    ensure!(
        paths.len() <= 1,
        "Multiple gateway overrides require separate explicit support"
    );
    let drop_in = paths
        .first()
        .map(|path| credential_drop_in(Path::new(path), gateway_path))
        .transpose()?;
    if let Some(pin) = &drop_in {
        ensure!(
            effective_environment == format!("VOYAGE_CREDENTIAL_KEY_FILE={}", pin.key_path),
            "Effective gateway credential environment differs from owned override"
        );
        let keys = live_environment
            .split(|b| *b == 0)
            .filter_map(|entry| entry.strip_prefix(b"VOYAGE_CREDENTIAL_KEY_FILE="))
            .collect::<Vec<_>>();
        ensure!(
            keys.len() == 1 && keys[0] == pin.key_path.as_bytes(),
            "Live gateway credential namespace differs from owned override"
        );
        ensure!(
            recovery_key == Some(std::ffi::OsStr::new(&pin.key_path)),
            "Recovery credential namespace differs from gateway"
        );
    }
    Ok(GatewayCredentials {
        effective_environment: effective_environment.into(),
        drop_in,
    })
}
fn gateway_credentials(unit: &str, path: &Path, pid: u32) -> Result<GatewayCredentials> {
    let paths = systemctl(&["show", unit, "--property=DropInPaths", "--value"])?;
    let environment = systemctl(&["show", unit, "--property=Environment", "--value"])?;
    #[cfg(not(test))]
    let live_environment =
        files::read(&PathBuf::from(format!("/proc/{pid}/environ")), 1024 * 1024)?;
    #[cfg(test)]
    let live_environment = files::read(&home()?.join(format!("pid-{pid}-environ")), 1024 * 1024)?;
    reviewed_gateway_credentials(
        paths.trim(),
        path,
        environment.trim(),
        &live_environment,
        std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").as_deref(),
    )
}

fn gateway_check(review: &Review, running: &Path, repaired: bool) -> Result<()> {
    let expected = if repaired {
        &review.gateway_repaired
    } else {
        &review.gateway_original
    };
    ensure!(
        String::from_utf8(files::read(&review.gateway_path, 16384)?)? == *expected,
        "Gateway unit changed independently"
    );
    ensure!(
        systemctl(&[
            "show",
            &review.gateway_unit,
            "--property=FragmentPath",
            "--value"
        ])?
        .trim()
            == review.gateway_path.to_str().context("Unit path encoding")?,
        "Gateway fragment path changed"
    );
    ensure!(
        systemctl(&[
            "show",
            &review.gateway_unit,
            "--property=UnitFileState",
            "--value"
        ])?
        .trim()
            == review.gateway_enablement,
        "Gateway enablement changed"
    );
    verified_service(&review.gateway_unit, running)?;
    let pid: u32 = systemctl(&[
        "show",
        &review.gateway_unit,
        "--property=MainPID",
        "--value",
    ])?
    .trim()
    .parse()?;
    owned_process(pid)?;
    ensure!(
        gateway_credentials(&review.gateway_unit, &review.gateway_path, pid)?
            == review.gateway_credentials,
        "Gateway credential configuration changed since review"
    );
    let args = process_arguments(pid)?;
    gateway_arguments(&args, &review.activation.state, &review.public_origin)?;
    if repaired {
        ensure!(
            args.split(|b| *b == 0).next()
                == Some(
                    installation_root()?
                        .join("current/bin/vessel")
                        .as_os_str()
                        .as_encoded_bytes()
                ),
            "Gateway is not using the repaired persistent managed path"
        );
    }
    Ok(())
}
fn gateway_address(arguments: &[u8]) -> Result<std::net::SocketAddr> {
    let args = arguments.split(|b| *b == 0).collect::<Vec<_>>();
    ensure!(
        args.iter().filter(|arg| **arg == b"--bind").count() == 1,
        "Recovery requires one explicit gateway bind"
    );
    let position = args.iter().position(|arg| *arg == b"--bind").unwrap();
    let address: std::net::SocketAddr =
        std::str::from_utf8(args.get(position + 1).context("Gateway bind missing")?)?.parse()?;
    ensure!(
        address.ip().is_loopback() && address.port() != 0,
        "Gateway health must remain literal loopback"
    );
    Ok(address)
}
fn gateway_health(review: &Review, running: &Path) -> Result<()> {
    let pid: u32 = systemctl(&[
        "show",
        &review.gateway_unit,
        "--property=MainPID",
        "--value",
    ])?
    .trim()
    .parse()?;
    let address = gateway_address(&process_arguments(pid)?)?;
    let endpoint = format!("http://{address}/health");
    let bytes = crate::service::command::run(
        Path::new("/usr/bin/curl"),
        &[
            "--disable",
            "--noproxy",
            "*",
            "--fail",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "2",
            "--max-time",
            "5",
            "--max-filesize",
            "4096",
            &endpoint,
        ],
        None,
    )?;
    let health: voyage_protocol::HealthResponse = serde_json::from_slice(&bytes)?;
    let manifest: Manifest =
        serde_json::from_slice(&files::read(&running.join("release.json"), 1024 * 1024)?)?;
    ensure!(
        health.status == "ok" && health.version == manifest.version.trim_start_matches('v'),
        "Gateway health/source version not ready"
    );
    Ok(())
}
fn observe(record: &Recovery, held: &crate::legacy::Guard) -> Result<()> {
    let review = &record.review;
    original_unchanged(review)?;
    ensure!(
        current()? == review.target_release,
        "Recovery target pointer not verified"
    );
    let target = release(&review.target_release)?;
    ensure!(
        bounded_hash(&target.join("release.json"), 1024 * 1024)? == review.target_manifest_sha256,
        "Target declaration changed"
    );
    service::observe_activation(&target.join("bin"), &review.activation, false)?;
    let pid: u32 = systemctl(&[
        "show",
        "voyage-vessel.service",
        "--property=MainPID",
        "--value",
    ])?
    .trim()
    .parse()?;
    ensure!(
        crate::legacy::accounts_for_process(pid)? == review.accounts,
        "Live recovered account namespace changed"
    );
    owned_process(pid)?;
    let arguments = process_arguments(pid)?;
    ensure!(
        legacy_namespace_matches_fields(&review.activation.state, &review.accounts, &arguments),
        "Live recovered state namespace changed"
    );
    gateway_check(review, &target, true)?;
    gateway_health(review, &target)?;
    held.verify()?;
    Ok(())
}
fn legacy_namespace_matches_fields(state: &Path, _accounts: &Path, argv: &[u8]) -> bool {
    let args = argv.split(|b| *b == 0).collect::<Vec<_>>();
    args.windows(2)
        .filter(|pair| pair[0] == b"--directory")
        .count()
        == 1
        && args.windows(2).any(|pair| {
            pair[0] == b"--directory" && pair[1] == state.as_os_str().as_encoded_bytes()
        })
}
fn prepare(operation: &str, arguments: &[String]) -> Result<()> {
    id(operation)?;
    ensure!(
        !record_path(operation)?.exists(),
        "Recovery identity already exists; observe it, never replay prepare"
    );
    ensure!(
        arguments.len() == 10,
        "Recovery requires five explicit reviewed options"
    );
    let mut options = std::collections::BTreeMap::new();
    for pair in arguments.as_chunks::<2>().0 {
        ensure!(
            options.insert(pair[0].as_str(), pair[1].as_str()).is_none(),
            "Repeated recovery option"
        );
    }
    let option = |key: &str| {
        options
            .get(key)
            .copied()
            .context("Missing recovery review option")
    };
    ensure!(
        options.keys().all(|key| [
            "--original-operation",
            "--running-release",
            "--bin-dir",
            "--gateway-unit",
            "--public-origin"
        ]
        .contains(key)),
        "Unknown recovery option"
    );
    let original_operation = option("--original-operation")?.to_owned();
    id(&original_operation)?;
    ensure!(
        operation != original_operation,
        "Recovery must use a fresh identity distinct from the original uncertain operation"
    );
    let original = load(&original_operation)?;
    ensure!(
        original.phase == "unconfirmed",
        "Original operation must remain explicitly uncertain"
    );
    let old = current()?;
    ensure!(
        old == original.current_release,
        "Mixed managed metadata does not match original uncertain receipt"
    );
    let old_manifest = installed_manifest(&old)?;
    ensure!(
        old_manifest.is_legacy_v102(&installation_root()?.join("releases").join(&old))?
            && old_manifest.update_compatibility.is_none(),
        "Recovery supports only the proved legacy metadata state"
    );
    release(&old)?;
    let running_id = option("--running-release")?.to_owned();
    ensure!(
        original.release_id.as_deref() == Some(running_id.as_str()),
        "Running release is not the originally reviewed update candidate"
    );
    let running = release(&running_id)?;
    ensure!(
        running_id != old,
        "Recovery requires a distinct proved running candidate"
    );
    let target_bin = PathBuf::from(option("--bin-dir")?);
    let target = Manifest::inspect(&target_bin)?;
    let target_id = target.id()?;
    ensure!(
        target_id != old && target_id != running_id,
        "Recovery target must be a distinct qualified release"
    );
    let activation = service::review_activation(&running.join("bin"))?;
    ensure!(activation.active, "Known running supervisor must be active");
    service::observe_activation(&running.join("bin"), &activation, true)?;
    verified_service("voyage-vessel.service", &running)?;
    let pid: u32 = systemctl(&[
        "show",
        "voyage-vessel.service",
        "--property=MainPID",
        "--value",
    ])?
    .trim()
    .parse()?;
    owned_process(pid)?;
    let accounts = crate::legacy::accounts_for_process(pid)?;
    ensure!(
        legacy_namespace_matches_fields(&activation.state, &accounts, &process_arguments(pid)?),
        "Running supervisor state namespace differs from reviewed unit"
    );
    ensure!(
        !activation.state.join("update-quarantine.json").exists(),
        "Another recovery quarantine exists"
    );
    let source_evidence =
        crate::legacy::forward_evidence(&activation.state, &accounts, &installation_root()?)?;
    forward_formats(
        &installed_manifest(&running_id)?,
        &target,
        &source_evidence,
        &files::hash(&std::env::current_exe()?)?,
    )?;
    let gateway_unit = option("--gateway-unit")?.to_owned();
    ensure!(
        gateway_unit.ends_with(".service")
            && gateway_unit != "voyage-vessel.service"
            && gateway_unit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "Ordinary gateway unit name required"
    );
    let gateway_path = home()?.join(".config/systemd/user").join(&gateway_unit);
    files::safe(&gateway_path)?;
    let metadata = fs::symlink_metadata(&gateway_path)?;
    ensure!(
        metadata.is_file() && metadata.uid() == unsafe { libc::geteuid() } && metadata.nlink() == 1,
        "Gateway unit must be exact owned regular file"
    );
    verified_service(&gateway_unit, &running)?;
    let gateway_pid: u32 = systemctl(&["show", &gateway_unit, "--property=MainPID", "--value"])?
        .trim()
        .parse()?;
    owned_process(gateway_pid)?;
    only_reviewed_services(&running, &gateway_unit)?;
    let origin = option("--public-origin")?;
    gateway_arguments(&process_arguments(gateway_pid)?, &activation.state, origin)?;
    let gateway_original = String::from_utf8(files::read(&gateway_path, 16384)?)?;
    let gateway_repaired = gateway_repair(
        &gateway_original,
        &running.join("bin/vessel"),
        &installation_root()?.join("current/bin/vessel"),
        origin,
    )?;
    let stage = directory()?.join(operation);
    files::private_directory(&stage)?;
    target.stage(&target_bin, &stage.join("target"))?;
    let review = Review {
        schema_version: 1,
        operation_id: operation.into(),
        original_operation,
        original_sha256: bounded_hash(&path(&original.operation_id)?, 65536)?,
        old_release: old,
        metadata_sha256: bounded_hash(&installation_root()?.join("transaction.json"), 65536)?,
        running_release: running_id,
        running_manifest_sha256: bounded_hash(&running.join("release.json"), 1024 * 1024)?,
        target_release: target_id,
        target_manifest_sha256: bounded_hash(&stage.join("target/release.json"), 1024 * 1024)?,
        staged_bin: stage.join("target/bin"),
        activation,
        accounts,
        source_evidence,
        gateway_unit: gateway_unit.clone(),
        gateway_path: gateway_path.clone(),
        gateway_original,
        gateway_repaired,
        gateway_effective: unit_definition(&gateway_unit)?,
        gateway_credentials: gateway_credentials(&gateway_unit, &gateway_path, gateway_pid)?,
        gateway_enablement: systemctl(&[
            "show",
            &gateway_unit,
            "--property=UnitFileState",
            "--value",
        ])?
        .trim()
        .into(),
        public_origin: origin.into(),
    };
    gateway_check(&review, &running, false)?;
    gateway_health(&review, &running)?;
    let mut record = Recovery {
        review,
        phase: "reviewed".into(),
        proof: None,
    };
    save_recovery(&mut record, "reviewed")?;
    println!(
        "{}",
        serde_json::json!({"operation_id":operation,"review_sha256":review_hash(&record.review)?,"original_operation_retained":record.review.original_operation,"old_metadata":record.review.old_release,"verified_running_release":record.review.running_release,"target_release":record.review.target_release,"gateway_unit":record.review.gateway_unit,"action":"Forward recovery only; quiescent schema2 unchanged-state proof; no original apply replay or database restore"})
    );
    Ok(())
}
/// Idle notification SQLite headers/unused cold journal bytes can drift while
/// the reviewed source is still live. Full semantic notification evidence and
/// every other file/context claim remain pinned; held proof thereafter is raw.
fn ensure_reviewed_source(
    approved: &serde_json::Value,
    observed: &serde_json::Value,
    snapshot: bool,
) -> Result<()> {
    let approved = approved
        .as_object()
        .context("Reviewed source evidence missing")?;
    let observed = observed
        .as_object()
        .context("Observed source evidence missing")?;
    for name in ["state_sha256", "review_state_sha256"] {
        for object in [approved, observed] {
            ensure!(
                object
                    .get(name)
                    .and_then(|value| value.as_str())
                    .is_some_and(|hash| hash.len() == 64
                        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())),
                "Full raw and semantic notification review evidence required; use a fresh reviewed operation"
            );
        }
    }
    let mut expected = approved
        .keys()
        .filter(|key| !snapshot || key.as_str() != "sessions")
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if snapshot {
        expected.insert("backup_sha256".into());
    }
    ensure!(
        expected
            == observed
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
        "Reviewed source evidence shape changed"
    );
    for (claim, value) in approved {
        if claim != "state_sha256" && (!snapshot || claim != "sessions") {
            ensure!(
                observed.get(claim) == Some(value),
                "Canonical/private/notification logical state changed since owner review"
            );
        }
    }
    Ok(())
}
fn install_release(bin: &Path, expected: &str) -> Result<()> {
    let result = install::run(install::Options {
        bin_dir: bin.into(),
        replace_existing: false,
        dry_run: false,
    })?;
    ensure!(
        result.release == expected && current()? == expected,
        "Recovery publication identity changed"
    );
    Ok(())
}
fn apply(record: &mut Recovery, approved: &str) -> Result<()> {
    ensure!(
        review_hash(&record.review)? == approved,
        "Recovery review approval changed"
    );
    ensure!(
        record.phase == "reviewed",
        "Recovery operation already attempted; observe only, never replay"
    );
    let _operation = install::operation_lock()?;
    let _coordinator = files::lock(&root()?.join("coordinator.lock"))?;
    let _worker = files::lock(&root()?.join("worker.lock"))?;
    let review = &record.review;
    original_unchanged(review)?;
    ensure!(
        current()? == review.old_release
            && bounded_hash(&installation_root()?.join("transaction.json"), 65536)?
                == review.metadata_sha256,
        "Managed metadata changed since owner review"
    );
    let running = release(&review.running_release)?;
    ensure!(
        bounded_hash(&running.join("release.json"), 1024 * 1024)? == review.running_manifest_sha256,
        "Running release declaration changed"
    );
    let target = Manifest::inspect(&review.staged_bin)?;
    ensure!(
        target.id()? == review.target_release
            && bounded_hash(
                &review.staged_bin.parent().unwrap().join("release.json"),
                1024 * 1024
            )? == review.target_manifest_sha256,
        "Reviewed target changed"
    );
    service::observe_activation(&running.join("bin"), &review.activation, true)?;
    gateway_check(review, &running, false)?;
    only_reviewed_services(&running, &review.gateway_unit)?;
    ensure!(
        unit_definition(&review.gateway_unit)? == review.gateway_effective,
        "Effective gateway changed since review"
    );
    let pid: u32 = systemctl(&[
        "show",
        "voyage-vessel.service",
        "--property=MainPID",
        "--value",
    ])?
    .trim()
    .parse()?;
    ensure!(
        crate::legacy::accounts_for_process(pid)? == review.accounts,
        "Account namespace changed since owner review"
    );
    let observed_source = crate::legacy::forward_evidence(
        &review.activation.state,
        &review.accounts,
        &installation_root()?,
    )?;
    ensure_reviewed_source(&review.source_evidence, &observed_source, false)?;
    forward_formats(
        &installed_manifest(&review.running_release)?,
        &target,
        &observed_source,
        &files::hash(&std::env::current_exe()?)?,
    )?;
    for entry in fs::read_dir(root()?)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let other: Record = serde_json::from_slice(&files::read(&path, 65536)?)?;
            ensure!(
                other.operation_id == record.review.original_operation
                    || ![
                        "preparing",
                        "ready",
                        "applying",
                        "committing",
                        "unconfirmed"
                    ]
                    .contains(&other.phase.as_str()),
                "Another unresolved update exists"
            );
        }
    }
    save_recovery(record, "applying")?;
    let mut guard = None;
    let effect = (|| -> Result<()> {
        let review = &record.review;
        crate::legacy::fence(
            &review.activation.state,
            &review.operation_id,
            &review.old_release,
            &review.target_release,
        )?;
        systemctl(&["stop", &review.gateway_unit])?;
        service::quiesce(&running.join("bin"), &review.activation)?;
        guard = Some(crate::legacy::begin_forward(
            &review.activation.state,
            &review.accounts,
            &directory()?.join(&review.operation_id),
        )?);
        record.proof = guard.as_ref().map(|held| held.proof.clone());
        ensure_reviewed_source(
            &record.review.source_evidence,
            &record.proof.as_ref().unwrap().evidence,
            true,
        )?;
        save_recovery(record, "quiescent")?;
        // Repair metadata to the *already approved actual source* first. It is
        // the supported previous reader; legacy v1.0.2 is never rollback target.
        install_release(&running.join("bin"), &record.review.running_release)?;
        save_recovery(record, "source-reconciled")?;
        install_release(&record.review.staged_bin, &record.review.target_release)?;
        guard.as_mut().unwrap().permit_supervisor();
        let target = release(&record.review.target_release)?;
        service::start_quarantined(&target.join("bin"), &record.review.activation)?;
        crate::service::files::replace(
            &record.review.gateway_path,
            &record.review.gateway_repaired,
            Some(&record.review.gateway_original),
        )?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["reset-failed", &record.review.gateway_unit])?;
        systemctl(&["start", &record.review.gateway_unit])?;
        guard.as_ref().unwrap().verify()?;
        save_recovery(record, "committing")?;
        // Retain every journal/startup/guardian lease through observation and fence clear.
        observe(
            record,
            guard.as_ref().context("Forward ownership unavailable")?,
        )?;
        crate::legacy::clear(&record.review.activation.state, &record.review.operation_id)?;
        save_recovery(record, "complete")?;
        Ok(())
    })();
    if let Err(error) = effect {
        drop(guard);
        save_recovery(record, "unconfirmed")?;
        return Err(error.context("Forward recovery unconfirmed; quarantine, original receipt, source and target retained. No database restore, old apply replay or automatic repair was attempted"));
    }
    Ok(())
}
fn root_for_proof(operation: &str) -> Result<PathBuf> {
    id(operation)?;
    Ok(installation_root()?.join("recoveries").join(operation))
}
pub(super) fn supersedes(original: &Record) -> Result<bool> {
    let root = installation_root()?.join("recoveries");
    if !root.exists() {
        return Ok(false);
    }
    let entries = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    ensure!(entries.len() <= 260, "Recovery receipt bound exceeded");
    for entry in entries {
        if entry.path().extension().is_some_and(|ext| ext == "json") {
            let recovered: Recovery =
                serde_json::from_slice(&files::read(&entry.path(), 1024 * 1024)?)?;
            if recovered.phase == "complete"
                && recovered.review.original_operation == original.operation_id
            {
                original_unchanged(&recovered.review)?;
                ensure!(
                    recovered.review.schema_version == 1,
                    "Recovery record version changed"
                );
                let proof = recovered
                    .proof
                    .as_ref()
                    .context("Completed recovery proof missing")?;
                ensure!(
                    proof.state == recovered.review.activation.state
                        && proof.accounts == recovered.review.accounts
                        && proof.stage == root_for_proof(&recovered.review.operation_id)?,
                    "Completed recovery proof namespace changed"
                );
                ensure!(
                    files::hash(&proof.stage.join("legacy-catalogue.sqlite3"))?
                        == proof.evidence["backup_sha256"]
                            .as_str()
                            .context("Pinned forward snapshot missing")?,
                    "Completed recovery snapshot changed"
                );

                ensure!(
                    recovered.review.old_release == original.current_release
                        && original.release_id.as_deref()
                            == Some(recovered.review.running_release.as_str()),
                    "Recovery receipt original identity mismatch"
                );
                release(&recovered.review.target_release)?;
                ensure!(
                    bounded_hash(
                        &release(&recovered.review.target_release)?.join("release.json"),
                        1024 * 1024
                    )? == recovered.review.target_manifest_sha256,
                    "Completed recovery target declaration changed"
                );
                return Ok(true);
            }
        }
    }
    Ok(false)
}
pub(super) fn run(args: &[String]) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } != 0 && unsafe { libc::geteuid() } == unsafe { libc::getuid() },
        "Ordinary owner recovery must not run as root"
    );
    ensure!(
        args.len() >= 2,
        "recover-user requires action and fresh operation UUID"
    );
    let operation = &args[1];
    id(operation)?;
    match args[0].as_str() {
        "prepare" => prepare(operation, &args[2..]),
        "apply" => {
            ensure!(
                args.len() == 4 && args[2] == "--review",
                "Exact owner review hash required"
            );
            let mut record: Recovery =
                serde_json::from_slice(&files::read(&record_path(operation)?, 1024 * 1024)?)?;
            apply(&mut record, &args[3])
        }
        "status" => {
            ensure!(args.len() == 2, "Status takes only the recovery UUID");
            let mut record: Recovery =
                serde_json::from_slice(&files::read(&record_path(operation)?, 1024 * 1024)?)?;
            if record.phase == "committing" || record.phase == "unconfirmed" {
                let _worker = files::lock(&root()?.join("worker.lock"))?;
                if let Some(proof) = &record.proof
                    && let Ok(held) = crate::legacy::hold_forward(proof)
                    && observe(&record, &held).is_ok()
                {
                    let fence = record
                        .review
                        .activation
                        .state
                        .join("update-quarantine.json");
                    if fence.exists() {
                        crate::legacy::clear(&record.review.activation.state, operation)?;
                    }
                    save_recovery(&mut record, "complete")?;
                }
            }
            println!(
                "{}",
                serde_json::json!({"operation_id":operation,"phase":record.phase,"original_operation_retained":record.review.original_operation,"target_release":record.review.target_release})
            );
            Ok(())
        }
        _ => anyhow::bail!("Unknown recover-user action"),
    }
}
#[cfg(test)]
#[path = "forward_recovery_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "forward_owned_transaction_tests.rs"]
mod owned_transaction_tests;
