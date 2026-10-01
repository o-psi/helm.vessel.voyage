use crate::{
    cli, flow,
    install::{self, files, release::Manifest},
    service, source,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
struct Gateway {
    unit: String,
    definition: String,
}
#[derive(Serialize, Deserialize)]
struct Record {
    operation_id: String,
    channel: String,
    phase: String,
    message: String,
    created_at: u64,
    updated_at: u64,
    current_release: String,
    release_id: Option<String>,
    version: Option<String>,
    description: Option<String>,
    bin_dir: Option<PathBuf>,
    #[serde(default)]
    staging_root: Option<PathBuf>,
    gateways: Vec<Gateway>,
    #[serde(default)]
    contracts_sha256: Option<[String; 2]>,
    #[serde(default)]
    supervisor_activation: Option<service::Activation>,
    #[serde(default)]
    legacy_mode: bool,
    #[serde(default)]
    legacy_proof: Option<crate::legacy::Proof>,
    #[serde(default)]
    legacy_accounts: Option<PathBuf>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn id(value: &str) -> Result<()> {
    ensure!(
        value.len() == 36
            && value
                .bytes()
                .enumerate()
                .all(|(i, c)| if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }),
        "Invalid update identity"
    );
    Ok(())
}
fn home() -> Result<PathBuf> {
    #[cfg(test)]
    {
        crate::fixture_tests::root().context("Remote updater test requires isolated fixture")
    }
    #[cfg(not(test))]
    {
        Ok(std::env::var_os("HOME").context("HOME required")?.into())
    }
}
fn installation_root() -> Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(home()?.join("install"))
    }
    #[cfg(not(test))]
    {
        Ok(home()?.join(".local/share/voyage/install"))
    }
}
fn root() -> Result<PathBuf> {
    let root = installation_root()?.join("updates");
    files::private_directory(&root)?;
    Ok(root)
}
fn current() -> Result<String> {
    let root = installation_root()?;
    let state: serde_json::Value =
        serde_json::from_slice(&files::read(&root.join("transaction.json"), 65536)?)?;
    ensure!(
        state["pending"].is_null(),
        "An installation transaction is unresolved"
    );
    let id = state["current"]
        .as_str()
        .context("No managed installation")?;
    ensure!(
        id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid installed identity"
    );
    ensure!(
        fs::read_link(root.join("current"))? == root.join("releases").join(id),
        "Installation pointer changed"
    );
    Ok(id.into())
}
fn path(operation: &str) -> Result<PathBuf> {
    id(operation)?;
    Ok(root()?.join(format!("{operation}.json")))
}
fn load(operation: &str) -> Result<Record> {
    Ok(serde_json::from_slice(&files::read(
        &path(operation)?,
        65536,
    )?)?)
}
fn save(record: &mut Record, phase: &str, message: &str) -> Result<()> {
    record.phase = phase.into();
    record.message = message.into();
    record.updated_at = now();
    files::atomic_json(&path(&record.operation_id)?, record)
}
fn output(record: &Record) -> Result<()> {
    // Internal paths and subprocess diagnostics never cross the public connection.
    println!(
        "{}",
        serde_json::json!({"operation_id":record.operation_id,"channel":record.channel,"phase":record.phase,"message":record.message,"current_release":record.current_release,"release_id":record.release_id,"version":record.version,"description":record.description,"services":std::iter::once("voyage-vessel.service").chain(record.gateways.iter().map(|g|g.unit.as_str())).collect::<Vec<_>>(),"expires_at":record.created_at+3600})
    );
    Ok(())
}
fn command(command: Command) -> Result<String> {
    command_for(command, Duration::from_secs(20))
}
fn command_for(mut command: Command, timeout: Duration) -> Result<String> {
    // Output goes to a private bounded file, not a pipe that can deadlock.
    let p = root()?.join(format!("command-{}-{}.log", std::process::id(), now()));
    files::write_new(&p, b"")?;
    let log = fs::OpenOptions::new().write(true).open(&p)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let deadline = Instant::now() + timeout;
    let result = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "Service manager command failed");
                break;
            }
            if Instant::now() >= deadline || fs::metadata(&p)?.len() > 1024 * 1024 {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("Service manager did not confirm the operation");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(String::from_utf8(files::read(&p, 1024 * 1024)?)?)
    })();
    let _ = fs::remove_file(p);
    result
}
fn systemctl(args: &[&str]) -> Result<String> {
    #[cfg(test)]
    {
        crate::fixture_tests::systemctl(args)
    }
    #[cfg(not(test))]
    {
        let mut c = Command::new("systemctl");
        c.args(["--user", "--no-pager"]).args(args);
        command(c)
    }
}
fn unit_definition(unit: &str) -> Result<String> {
    stable_unit_definition(&systemctl(&[
        "show",
        unit,
        "--property=ExecStart,FragmentPath,DropInPaths",
    ])?)
}
fn stable_unit_definition(definition: &str) -> Result<String> {
    let mut stable = String::new();
    for line in definition.lines() {
        if line.starts_with("ExecStart=") && line.contains(" ; start_time=") {
            // systemd appends process observations to the configured command.
            // A supervisor restart can also restart Requires= gateways.
            let (command, observations) = line.rsplit_once(" ; start_time=").unwrap();
            ensure!(
                observations.contains(" ; stop_time=")
                    && observations.contains(" ; pid=")
                    && observations.contains(" ; code=")
                    && observations.contains(" ; status=")
                    && observations.ends_with(" }")
                    && !observations.contains("argv[]="),
                "Unrecognized gateway command observations"
            );
            stable.push_str(command);
            stable.push_str(" }");
        } else {
            stable.push_str(line);
        }
        stable.push('\n');
    }
    Ok(stable)
}
fn unchanged_gateway(gateway: &Gateway) -> Result<bool> {
    // Normalize receipts produced before runtime observations were excluded,
    // allowing read-only reconciliation of their already-stopped operations.
    Ok(unit_definition(&gateway.unit)? == stable_unit_definition(&gateway.definition)?)
}
fn persistent_gateway_command(command: Option<&[u8]>, home: &std::path::Path) -> bool {
    [
        ".local/bin/vessel",
        ".local/share/voyage/install/current/bin/vessel",
    ]
    .iter()
    .any(|relative| command == Some(home.join(relative).as_os_str().as_encoded_bytes()))
}
fn process_executable(pid: u32) -> std::io::Result<PathBuf> {
    #[cfg(test)]
    {
        crate::fixture_tests::executable(pid)
    }
    #[cfg(not(test))]
    {
        fs::read_link(format!("/proc/{pid}/exe"))
    }
}
fn process_arguments(pid: u32) -> std::io::Result<Vec<u8>> {
    #[cfg(test)]
    {
        fs::read(
            home()
                .expect("isolated process fixture")
                .join(format!("cmdline-{pid}")),
        )
    }
    #[cfg(not(test))]
    {
        fs::read(format!("/proc/{pid}/cmdline"))
    }
}
fn gateways() -> Result<Vec<Gateway>> {
    let exe = installation_root()?
        .join("current/bin/vessel")
        .canonicalize()?;
    #[cfg(test)]
    let state = home()?.join(".local/state/voyage/vessel");
    #[cfg(not(test))]
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".local/state"))
        .join("voyage/vessel");
    let units: Vec<serde_json::Value> = serde_json::from_str(&systemctl(&[
        "list-units",
        "--type=service",
        "--state=running",
        "--output=json",
    ])?)?;
    let mut found = Vec::new();
    for unit in units {
        let Some(name) = unit["unit"].as_str() else {
            continue;
        };
        if name == "voyage-vessel.service" {
            continue;
        }
        let pid = systemctl(&["show", name, "--property=MainPID", "--value"])?;
        let Ok(pid) = pid.trim().parse::<u32>() else {
            continue;
        };
        if process_executable(pid).ok().as_ref() != Some(&exe) {
            continue;
        }
        let argv = process_arguments(pid)?;
        let args: Vec<_> = argv.split(|c| *c == 0).filter(|v| !v.is_empty()).collect();
        if !args
            .windows(2)
            .any(|v| v[0] == b"--process-directory" && v[1] == state.as_os_str().as_encoded_bytes())
        {
            continue;
        }
        ensure!(
            persistent_gateway_command(args.first().copied(), &home()?),
            "Gateway must use a managed command or install/current/bin/vessel before remote update"
        );
        ensure!(
            name.ends_with(".service")
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_@.".contains(&c)),
            "Unsupported gateway unit name"
        );
        found.push(Gateway {
            unit: name.into(),
            definition: unit_definition(name)?,
        });
    }
    found.sort_by(|a, b| a.unit.cmp(&b.unit));
    Ok(found)
}
fn launch(record: &Record, phase: &str) -> Result<()> {
    #[cfg(test)]
    {
        crate::fixture_tests::systemctl(&["launch-update", &record.operation_id, phase]).map(|_| ())
    }
    #[cfg(not(test))]
    {
        let exe = std::env::current_exe()?;
        let mut c = Command::new("systemd-run");
        c.args(["--user", "--quiet", "--collect", "--service-type=exec"])
            .arg(format!(
                "--unit=voyage-update-{}-{phase}",
                record.operation_id
            ))
            .args([
                "--property=RuntimeMaxSec=1200",
                "--property=TimeoutStopSec=15",
                "--property=UMask=0077",
                "--property=StandardOutput=null",
                "--property=StandardError=null",
            ])
            .arg(exe)
            .args(["remote-update", "worker", phase, &record.operation_id]);
        command(c)?;
        Ok(())
    }
}
/// Binary rollback cannot recover a database written in an unreadable format.
/// No backup/transaction replay is inferred from an absent legacy declaration.
fn rollback_formats(installed: &Manifest, candidate: &Manifest) -> Result<()> {
    installed.validate()?;
    candidate.validate()?;
    let previous = installed.update_compatibility.as_ref().context(
        "Installed release has no declared rollback reader formats; a verified legacy migration/backup plan is required before publication"
    )?;
    let next = candidate.update_compatibility.as_ref().context(
        "Candidate release has no declared persistent writer formats; rollback compatibility is unknown"
    )?;
    for (read, write) in [
        ("catalogue_read", "catalogue_write"),
        ("journal_read", "journal_write"),
    ] {
        ensure!(
            next.formats[write]
                .iter()
                .all(|format| previous.formats[read].contains(format)),
            "Previous release cannot read candidate {write} formats; no publication performed and no database downgrade is permitted"
        );
    }
    Ok(())
}

fn contract_id(manifest: &Manifest) -> Result<String> {
    use sha2::{Digest, Sha256};
    let declaration = manifest
        .update_compatibility
        .as_ref()
        .context("Persistent format declaration unavailable")?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(declaration)?)
    ))
}

fn installed_manifest(identity: &str) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&files::read(
        &installation_root()?
            .join("releases")
            .join(identity)
            .join("release.json"),
        1024 * 1024,
    )?)?;
    ensure!(
        manifest.id()? == identity,
        "Installed manifest identity changed"
    );
    Ok(manifest)
}

fn verified_service(unit: &str, release: &std::path::Path) -> Result<()> {
    ensure!(
        systemctl(&["show", unit, "--property=ActiveState", "--value"])?.trim() == "active",
        "Service is not active"
    );
    let pid: u32 = systemctl(&["show", unit, "--property=MainPID", "--value"])?
        .trim()
        .parse()?;
    ensure!(
        pid > 1 && process_executable(pid)? == release.join("bin/vessel"),
        "Service executable not verified"
    );
    Ok(())
}

/// Restore only unchanged reviewed gateways, even if supervisor readiness failed.
/// Every attempted activation must independently expose the previous executable.
fn rollback_gateways(record: &Record, previous: &std::path::Path) -> Result<()> {
    let mut failures = Vec::new();
    for gateway in &record.gateways {
        let result = (|| -> Result<()> {
            ensure!(
                unchanged_gateway(gateway)?,
                "Refusing rollback over changed gateway configuration"
            );
            systemctl(&["reset-failed", &gateway.unit])?;
            systemctl(&["restart", &gateway.unit])?;
            verified_service(&gateway.unit, previous)
        })();
        if let Err(error) = result {
            failures.push(format!("{}: {error}", gateway.unit));
        }
    }
    ensure!(
        failures.is_empty(),
        "Gateway rollback remains unconfirmed: {}",
        failures.join("; ")
    );
    Ok(())
}

fn restore_previous(record: &Record, error: anyhow::Error) -> Result<()> {
    let previous = install::rollback(false)
        .context("Binary rollback could not be confirmed; services retained for inspection")?;
    let supervisor = record
        .supervisor_activation
        .as_ref()
        .context("Prior supervisor activation was not pinned; rollback remains unconfirmed")
        .and_then(|activation| {
            service::restore_activation(
                &previous.release_dir.join("bin"),
                &installation_root()?
                    .join("releases")
                    .join(
                        record
                            .release_id
                            .as_deref()
                            .context("Candidate identity missing")?,
                    )
                    .join("bin"),
                activation,
            )
        });
    let gateways = rollback_gateways(record, &previous.release_dir);
    // Configuration already verifies authenticated readiness for an active unit;
    // these independent kernel executable checks forbid a pointer-only claim.
    let observed = if record
        .supervisor_activation
        .as_ref()
        .is_some_and(|activation| !activation.active)
    {
        Ok(())
    } else {
        verified_service("voyage-vessel.service", &previous.release_dir)
    };
    let failures = [supervisor, gateways, observed]
        .into_iter()
        .filter_map(Result::err)
        .map(|failure| failure.to_string())
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(error.context(format!("Previous binary pointer restored but service rollback remains unconfirmed: {}; persistent state was not downgraded", failures.join("; "))));
    }
    Err(error.context(
        "Previous installation and all reviewed services verified active after activation failed",
    ))
}

fn prepare_worker(record: &mut Record) -> Result<()> {
    record.gateways = gateways()?;
    let cancel = source::Cancellation::new()?;
    let mut prepared = source::prepare(
        if record.channel == "nightly" {
            source::Source::Nightly
        } else {
            source::Source::Latest
        },
        &cancel.flag,
    )?;
    let manifest = Manifest::inspect(&prepared.bin_dir)?;
    let installed: Manifest = serde_json::from_slice(&files::read(
        &installation_root()?
            .join("releases")
            .join(&record.current_release)
            .join("release.json"),
        1024 * 1024,
    )?)?;
    if let (Ok(old), Ok(new)) = (
        semver::Version::parse(installed.version.trim_start_matches('v')),
        semver::Version::parse(manifest.version.trim_start_matches('v')),
    ) {
        ensure!(
            new >= old,
            "Selected release is older than the installed version; remote downgrade is refused"
        );
    }

    if manifest.id()? == record.current_release {
        record.release_id = Some(manifest.id()?);
        record.version = Some(manifest.version.clone());
        record.description = Some(prepared.description.clone());
        return save(
            record,
            "complete",
            "This exact release is already installed. No installation or service changes were made.",
        );
    }
    if installed.update_compatibility.is_none()
        && installed.version.trim_start_matches('v') == "1.0.2"
    {
        manifest
            .update_compatibility
            .as_ref()
            .context("Candidate format declaration unavailable")?
            .validate()?;
        let activation = service::review_activation(
            &installation_root()?
                .join("releases")
                .join(&record.current_release)
                .join("bin"),
        )?;
        ensure!(
            activation.active,
            "Legacy migration requires a verified managed active supervisor"
        );
        service::catalogue(
            &installation_root()?
                .join("releases")
                .join(&record.current_release)
                .join("bin"),
            &activation.state,
        )?;
        let pid: u32 = systemctl(&[
            "show",
            "voyage-vessel.service",
            "--property=MainPID",
            "--value",
        ])?
        .trim()
        .parse()?;
        ensure!(
            process_executable(pid)?
                == installation_root()?
                    .join("releases")
                    .join(&record.current_release)
                    .join("bin/vessel"),
            "Legacy supervisor identity changed"
        );
        let accounts = crate::legacy::accounts_for_process(pid)?;
        crate::legacy::eligible(&activation.state, &accounts, prepared.staging_root())?;
        record.legacy_accounts = Some(accounts);
        record.legacy_mode = true;
        record.contracts_sha256 = Some([
            format!("legacy-quiescent:{}", record.current_release),
            contract_id(&manifest)?,
        ]);
    } else {
        rollback_formats(&installed, &manifest)?;
        record.contracts_sha256 = Some([contract_id(&installed)?, contract_id(&manifest)?]);
    }
    // Exercise the candidate loader before approval; no unit has been changed.
    let mut check = Command::new(prepared.bin_dir.join("vessel"));
    check.arg("--version");
    command(check).context("Prepared Vessel cannot run on this host operating system")?;
    let mut check_updater = Command::new(prepared.bin_dir.join("voyage-installer"));
    check_updater.args(["remote-update", "protocol"]);
    ensure!(command(check_updater).context("This release cannot retain remote updates; choose a newer release or development build")?.trim() == "1", "Prepared release uses an unsupported updater protocol");
    let options = cli::Options::parse(&[
        "upgrade".into(),
        "--bin-dir".into(),
        prepared.bin_dir.to_string_lossy().into_owned(),
    ])?;
    let plan = flow::plan(&options)?;
    record.supervisor_activation = Some(service::review_activation(&plan.release_dir.join("bin"))?);
    record.release_id = Some(manifest.id()?);
    record.version = Some(manifest.version);
    record.description = Some(prepared.description.clone());
    record.bin_dir = Some(prepared.bin_dir.clone());
    record.staging_root = Some(prepared.staging_root().to_path_buf());
    save(
        record,
        "ready",
        "Update prepared. Review the exact release and approve installation.",
    )?;
    prepared.retain();
    Ok(())
}
fn cleanup_staging(record: &mut Record) -> Result<()> {
    if let Some(staging) = &record.staging_root {
        let cache = home()?.join(".cache/voyage/upgrades");
        ensure!(
            staging.parent() == Some(cache.as_path())
                && staging
                    .file_name()
                    .is_some_and(|v| v.to_string_lossy().starts_with("prepare-")),
            "Unrecognized staging directory"
        );
        files::safe(staging)?;
        if staging.exists() {
            fs::remove_dir_all(staging)?;
        }
    }
    record.staging_root = None;
    record.bin_dir = None;
    Ok(())
}
fn stop_gateways(record: &Record) -> Result<()> {
    for gateway in &record.gateways {
        ensure!(
            unchanged_gateway(gateway)?,
            "Gateway changed before legacy quiescence"
        );
        systemctl(&["stop", &gateway.unit])?;
        let state = systemctl(&["show", &gateway.unit, "--property=ActiveState", "--value"])?;
        ensure!(
            matches!(state.trim(), "inactive" | "failed"),
            "Gateway stop remains unconfirmed"
        );
        let pid = systemctl(&["show", &gateway.unit, "--property=MainPID", "--value"])?;
        ensure!(pid.trim() == "0", "Gateway process remains present");
    }
    Ok(())
}

fn apply_legacy(record: &mut Record, options: &cli::Options) -> Result<()> {
    let activation = record
        .supervisor_activation
        .clone()
        .context("Legacy activation intent unavailable")?;
    ensure!(
        activation.active,
        "Legacy update requires quiescent managed service handover"
    );
    let previous = installation_root()?
        .join("releases")
        .join(&record.current_release);
    let candidate = installation_root()?.join("releases").join(
        record
            .release_id
            .as_deref()
            .context("Candidate identity missing")?,
    );
    let stage = record
        .staging_root
        .clone()
        .context("Legacy staging unavailable")?;
    let accounts = record
        .legacy_accounts
        .clone()
        .context("Legacy account namespace was not pinned")?;
    crate::legacy::fence(
        &activation.state,
        &record.operation_id,
        &record.current_release,
        record.release_id.as_deref().unwrap(),
    )?;
    let mut guard = None;
    let mut published = false;
    let applied = (|| -> Result<()> {
        stop_gateways(record)?;
        service::quiesce(&previous.join("bin"), &activation)?;
        guard = Some(crate::legacy::begin(&activation.state, &accounts, &stage)?);
        record.legacy_proof = guard.as_ref().map(|guard| guard.proof.clone());
        save(
            record,
            "applying",
            "Legacy owners quiescent; exact private snapshot retained before migration",
        )?;
        flow::execute(options, false)?;
        published = true;
        guard.as_mut().unwrap().permit_supervisor();
        service::start_quarantined(&candidate.join("bin"), &activation)?;
        for gateway in &record.gateways {
            ensure!(
                unchanged_gateway(gateway)?,
                "Gateway changed during legacy activation"
            );
            systemctl(&["reset-failed", &gateway.unit])?;
            systemctl(&["restart", &gateway.unit])?;
            verified_service(&gateway.unit, &candidate)?;
        }
        verified_service("voyage-vessel.service", &candidate)?;
        let pid: u32 = systemctl(&[
            "show",
            "voyage-vessel.service",
            "--property=MainPID",
            "--value",
        ])?
        .trim()
        .parse()?;
        ensure!(
            crate::legacy::accounts_for_process(pid)? == accounts,
            "Candidate account namespace changed; commit refused"
        );
        guard.as_ref().unwrap().verify()?;
        ensure!(
            current()? == record.release_id.as_deref().unwrap(),
            "Legacy activation pointer changed"
        );
        // Commit is durable before lifting quarantine. Reconciliation can finish
        // this observation without installing/replaying the operation again.
        save(
            record,
            "committing",
            "Approved legacy activation verified; removing quarantine",
        )?;
        crate::legacy::clear(&activation.state, &record.operation_id)?;
        Ok(())
    })();
    if let Err(error) = applied {
        if record.phase == "committing" {
            return Err(error.context("Commit remains unconfirmed; installation is not replayed"));
        }
        let rollback = (|| -> Result<()> {
            let observed_pointer = current()?;
            ensure!(
                observed_pointer == record.current_release
                    || Some(observed_pointer.as_str()) == record.release_id.as_deref(),
                "Legacy pointer outcome is not matched; rollback remains unconfirmed"
            );
            published |= observed_pointer != record.current_release;
            if published {
                stop_gateways(record)?;
                let current_activation = service::review_activation(&candidate.join("bin"))?;
                service::quiesce(&candidate.join("bin"), &current_activation)?;
                guard
                    .as_mut()
                    .context("Legacy snapshot ownership unavailable")?
                    .restore()?;
                install::rollback(false)?;
                ensure!(
                    current()? == record.current_release,
                    "Previous legacy pointer not verified"
                );
            }
            // Release journal exclusion only after the original snapshot is back.
            drop(guard.take());
            service::restore_activation(
                &previous.join("bin"),
                &candidate.join("bin"),
                &activation,
            )?;
            rollback_gateways(record, &previous)?;
            verified_service("voyage-vessel.service", &previous)?;
            crate::legacy::clear(&activation.state, &record.operation_id)?;
            Ok(())
        })();
        return match rollback {
            Ok(())=>Err(error.context("Legacy update refused or failed; exact previous state and services restored, no operation replayed")),
            Err(rollback)=>Err(error.context(format!("Legacy rollback unconfirmed: {rollback}; snapshot, quarantine and releases retained"))),
        };
    }
    drop(guard);
    cleanup_staging(record)?;
    record.legacy_proof = None;
    save(
        record,
        "complete",
        "Approved quiescent legacy update committed and services verified; independent history retained",
    )
}

fn apply_worker(record: &mut Record) -> Result<()> {
    let _installation = install::operation_lock()?;
    ensure!(now() <= record.created_at + 3600, "Update review expired");
    ensure!(
        current()? == record.current_release,
        "Installation changed since review"
    );
    let bin = record.bin_dir.as_ref().context("Prepared source missing")?;
    let candidate = Manifest::inspect(bin)?;
    ensure!(
        candidate.id()? == record.release_id.as_deref().unwrap_or(""),
        "Prepared source changed"
    );
    let installed = installed_manifest(&record.current_release)?;
    if record.legacy_mode {
        ensure!(
            installed.version.trim_start_matches('v') == "1.0.2"
                && installed.update_compatibility.is_none(),
            "Legacy installation identity changed"
        );
        ensure!(
            record.contracts_sha256.as_ref()
                == Some(&[
                    format!("legacy-quiescent:{}", record.current_release),
                    contract_id(&candidate)?
                ]),
            "Legacy contract approval changed"
        );
    } else {
        rollback_formats(&installed, &candidate)?;
        ensure!(
            record.contracts_sha256.as_ref()
                == Some(&[contract_id(&installed)?, contract_id(&candidate)?]),
            "Persistent format declarations changed or were not pinned before approval; no publication performed"
        );
    }
    ensure!(
        serde_json::to_vec(&gateways()?)? == serde_json::to_vec(&record.gateways)?,
        "Gateway services changed since review"
    );
    let options = cli::Options::parse(&[
        "upgrade".into(),
        "--bin-dir".into(),
        bin.to_string_lossy().into_owned(),
    ])?;
    ensure!(
        record.supervisor_activation.as_ref() == Some(&service::review_activation(bin)?),
        "Supervisor activation or definition changed since review; no publication performed"
    );
    if record.legacy_mode {
        return apply_legacy(record, &options);
    }
    // Existing transactional publication validates all binary and asset hashes.
    let report = flow::execute(&options, false)?;
    if let Err(error) = service::configure(&report.release_dir.join("bin"), false, false) {
        return restore_previous(record, error);
    }
    let activated = (|| -> Result<()> {
        for gateway in &record.gateways {
            ensure!(
                unchanged_gateway(gateway)?,
                "Gateway definition changed during update"
            );
            systemctl(&["restart", &gateway.unit])?;
            let pid = systemctl(&["show", &gateway.unit, "--property=MainPID", "--value"])?;
            let pid: u32 = pid.trim().parse()?;
            ensure!(
                process_executable(pid)? == report.release_dir.join("bin/vessel"),
                "Gateway did not activate the approved release"
            );
        }
        Ok(())
    })();
    if let Err(error) = activated {
        return restore_previous(record, error);
    }
    ensure!(
        current()? == record.release_id.as_deref().unwrap_or(""),
        "Update identity could not be verified"
    );
    cleanup_staging(record)?;
    save(
        record,
        "complete",
        "Approved release installed and services restarted. Helm must verify the reconnected Vessel.",
    )?;
    Ok(())
}

fn reconcile(record: &mut Record) -> Result<()> {
    let _worker = files::lock(&root()?.join("worker.lock"))?;
    let _installation = install::operation_lock()?;
    let updater = format!("voyage-update-{}-apply.service", record.operation_id);
    let state = systemctl(&["show", &updater, "--property=ActiveState", "--value"])?;
    ensure!(
        matches!(state.trim(), "inactive" | "failed"),
        "Updater may still be active"
    );
    ensure!(
        systemctl(&["show", &updater, "--property=Job", "--value"])?
            .trim()
            .is_empty(),
        "Updater job remains pending"
    );
    let installed = current()?;
    ensure!(
        installed == record.current_release
            || Some(installed.as_str()) == record.release_id.as_deref(),
        "Another installation replaced this operation"
    );
    let release = installation_root()?.join("releases").join(&installed);
    let manifest: Manifest =
        serde_json::from_slice(&files::read(&release.join("release.json"), 1024 * 1024)?)?;
    ensure!(
        manifest.id()? == installed,
        "Installed manifest identity changed"
    );
    manifest.verify(&release)?;
    for unit in std::iter::once("voyage-vessel.service")
        .chain(record.gateways.iter().map(|g| g.unit.as_str()))
    {
        verified_service(unit, &release)?;
    }
    for gateway in &record.gateways {
        ensure!(unchanged_gateway(gateway)?, "Gateway configuration changed");
    }
    if record.legacy_mode {
        let activation = record
            .supervisor_activation
            .as_ref()
            .context("Legacy activation unavailable")?;
        let quarantine = activation.state.join("update-quarantine.json");
        if quarantine.try_exists()? {
            if Some(installed.as_str()) == record.release_id.as_deref() {
                crate::legacy::verify(
                    record
                        .legacy_proof
                        .as_ref()
                        .context("Legacy commit proof unavailable")?,
                )?;
            } else {
                // Previous services can only reopen this original schema after
                // a proved snapshot restoration, never an in-place downgrade.
                let proof = record
                    .legacy_proof
                    .as_ref()
                    .context("Pinned previous-state restoration proof unavailable")?;
                ensure!(
                    proof.state == activation.state
                        && record.legacy_accounts.as_ref() == Some(&proof.accounts)
                        && record.staging_root.as_ref() == Some(&proof.stage),
                    "Pinned legacy source namespace changed"
                );
                crate::legacy::restored(proof)?;
            }
            crate::legacy::clear(&activation.state, &record.operation_id)?;
        }
        record.legacy_proof = None;
    }
    cleanup_staging(record)?;
    if Some(installed.as_str()) == record.release_id.as_deref() {
        save(
            record,
            "complete",
            "Observed the approved installed release and all running services after reconnect. No update was replayed.",
        )
    } else {
        save(
            record,
            "failed",
            "The previous release and all services are verified active. You can prepare a new update; the uncertain operation was not replayed.",
        )
    }
}
pub(super) fn run(args: &[String]) -> Result<()> {
    // The existing updater snapshots user units and user layout only. A root
    // system invocation must never reach those effects with broader authority.
    ensure!(
        !(unsafe { libc::geteuid() } == 0
            && std::path::Path::new("/etc/voyage/system-install.json").try_exists()?),
        "System installation requires the scope-aware updater; user updater refused before any journal or service effect"
    );
    ensure!(args.len() >= 2, "Update action and identity required");
    let (action, operation) = if args[0] == "worker" {
        ensure!(args.len() == 3, "Invalid worker invocation");
        (args[1].as_str(), args[2].as_str())
    } else {
        (args[0].as_str(), args[1].as_str())
    };
    id(operation)?;
    let lock_path = root()?.join("coordinator.lock");
    let deadline = Instant::now() + Duration::from_secs(5);
    let _lock = loop {
        match files::lock(&lock_path) {
            Ok(lock) => break lock,
            Err(_) if args[0] == "worker" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error),
        }
    };
    if args[0] == "worker" {
        let mut record = load(operation)?;
        ensure!(
            record.phase
                == if action == "prepare" {
                    "preparing"
                } else {
                    "applying"
                },
            "Worker phase conflict"
        );
        // Separate process/manager unit owns this lock through all effects. Requests
        // observe atomically written receipts while worker is active.
        drop(_lock);
        let deadline = Instant::now() + Duration::from_secs(5);
        let _worker = loop {
            match files::lock(&root()?.join("worker.lock")) {
                Ok(lock) => break lock,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(error) => return Err(error),
            }
        };
        let result = match action {
            "prepare" => prepare_worker(&mut record),
            "apply" => apply_worker(&mut record),
            _ => anyhow::bail!("Invalid worker phase"),
        };
        if let Err(error) = &result {
            let detail: String = error
                .to_string()
                .chars()
                .filter(|c| !c.is_control())
                .take(512)
                .collect();
            let message = if action == "prepare" {
                format!("Preparation failed: {detail}. Installation was not applied.")
            } else {
                format!(
                    "Update could not be confirmed: {detail}. Check installation and services before another update; it was not replayed."
                )
            };
            save(
                &mut record,
                if action == "prepare" {
                    "failed"
                } else {
                    "unconfirmed"
                },
                &message,
            )?;
        }
        return result;
    }
    match action {
        "prepare" => {
            ensure!(
                args.len() == 3 && ["stable", "nightly"].contains(&args[2].as_str()),
                "Invalid update channel"
            );
            if path(operation)?.exists() {
                let record = load(operation)?;
                ensure!(record.channel == args[2], "Update identity conflict");
                return output(&record);
            }
            ensure!(
                fs::read_dir(root()?)?.count() < 260,
                "Update receipt limit reached; administrator must archive completed receipts"
            );
            // One active/ready receipt prevents overlapping downloads or applications.
            for entry in fs::read_dir(root()?)? {
                let p = entry?.path();
                if p.extension().is_some_and(|v| v == "json") {
                    let other: Record = serde_json::from_slice(&files::read(&p, 65536)?)?;
                    ensure!(
                        ![
                            "preparing",
                            "applying",
                            "committing",
                            "ready",
                            "unconfirmed"
                        ]
                        .contains(&other.phase.as_str()),
                        "An update is already pending; resolve or discard it first"
                    );
                }
            }
            let mut record = Record {
                operation_id: operation.into(),
                channel: args[2].clone(),
                phase: String::new(),
                message: String::new(),
                created_at: now(),
                updated_at: now(),
                current_release: current()?,
                release_id: None,
                version: None,
                description: None,
                bin_dir: None,
                staging_root: None,
                gateways: Vec::new(),
                contracts_sha256: None,
                supervisor_activation: None,
                legacy_mode: false,
                legacy_proof: None,
                legacy_accounts: None,
            };
            save(
                &mut record,
                "preparing",
                "Downloading and verifying an update; installation is unchanged.",
            )?;
            // Admission uncertainty is retained: duplicate prepare only reads status.
            let _ = launch(&record, "prepare");
            output(&record)
        }
        "status" => {
            ensure!(args.len() == 2, "Invalid status request");
            let mut record = if operation == "00000000-0000-0000-0000-000000000000" {
                let mut records = Vec::<Record>::new();
                for entry in fs::read_dir(root()?)? {
                    let p = entry?.path();
                    if p.extension().is_some_and(|v| v == "json") {
                        records.push(serde_json::from_slice(&files::read(&p, 65536)?)?);
                    }
                }
                records.sort_by_key(|r| r.created_at);
                let Some(record) = records.pop() else {
                    println!("{{\"phase\":\"idle\"}}");
                    return Ok(());
                };
                record
            } else {
                load(operation)?
            };
            if ["preparing", "applying", "committing"].contains(&record.phase.as_str())
                && now() > record.updated_at + 1230
            {
                let phase = if record.phase == "preparing" {
                    "failed"
                } else {
                    "unconfirmed"
                };
                save(
                    &mut record,
                    phase,
                    "Updater exceeded its deadline. Inspect the retained operation before retrying; the update was not replayed.",
                )?;
            }
            if record.phase == "unconfirmed" {
                let _ = reconcile(&mut record);
            }
            output(&record)
        }
        "apply" => {
            ensure!(args.len() == 3, "Reviewed release required");
            let mut record = load(operation)?;
            ensure!(
                record.release_id.as_deref() == Some(args[2].as_str()),
                "Approved release identity conflict"
            );
            if [
                "applying",
                "committing",
                "complete",
                "failed",
                "unconfirmed",
            ]
            .contains(&record.phase.as_str())
            {
                return output(&record);
            }
            ensure!(
                record.phase == "ready" && now() <= record.created_at + 3600,
                "Update is not ready or review expired"
            );
            ensure!(
                current()? == record.current_release,
                "Installation changed since review"
            );
            save(
                &mut record,
                "applying",
                "Installing the approved release. Temporary disconnection is expected.",
            )?;
            let _ = launch(&record, "apply");
            output(&record)
        }
        "discard" => {
            ensure!(args.len() == 2, "Invalid discard request");
            let mut record = load(operation)?;
            ensure!(
                record.phase == "ready",
                "Only a prepared, unapplied update can be discarded"
            );
            cleanup_staging(&mut record)?;
            save(
                &mut record,
                "discarded",
                "Prepared update discarded without applying it.",
            )?;
            output(&record)
        }
        _ => anyhow::bail!("Unsupported remote update action"),
    }
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod expanded_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture_tests::Fixture;
    const OP: &str = "10000000-0000-4000-8000-000000000001";
    #[test]
    fn gateway_configuration_comparison_ignores_only_runtime_observations() {
        let before = "ExecStart={ path=/managed/vessel ; argv[]=/managed/vessel --bind localhost ; ignore_errors=no ; start_time=[before] ; stop_time=[n/a] ; pid=1 ; code=(null) ; status=0/0 }\nFragmentPath=/unit\nDropInPaths=/credentials\n";
        let restarted = before
            .replace("[before]", "[after]")
            .replace("pid=1", "pid=2");
        let stable = stable_unit_definition(before).unwrap();
        assert_eq!(stable, stable_unit_definition(&restarted).unwrap());
        assert_eq!(stable, stable_unit_definition(&stable).unwrap());
        for changed in [
            restarted.replace("--bind localhost", "--bind another"),
            restarted.replace("path=/managed/vessel", "path=/other/vessel"),
            restarted.replace("ignore_errors=no", "ignore_errors=yes"),
            restarted.replace("FragmentPath=/unit", "FragmentPath=/other"),
            restarted.replace("DropInPaths=/credentials", "DropInPaths=/other"),
        ] {
            assert_ne!(stable, stable_unit_definition(&changed).unwrap());
        }
        assert!(stable_unit_definition("ExecStart={ path=x ; start_time=unexpected }").is_err());
    }
    #[test]
    fn gateway_restart_requires_a_path_that_follows_the_release_pointer() {
        let home = std::path::Path::new("/home/operator");
        for command in [
            "/home/operator/.local/bin/vessel",
            "/home/operator/.local/share/voyage/install/current/bin/vessel",
        ] {
            assert!(persistent_gateway_command(Some(command.as_bytes()), home));
        }
        for command in [
            "/home/operator/.local/share/voyage/install/releases/old/bin/vessel",
            "/home/other/.local/bin/vessel",
            "/usr/local/bin/vessel",
        ] {
            assert!(!persistent_gateway_command(Some(command.as_bytes()), home));
        }
        assert!(!persistent_gateway_command(None, home));
    }
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).into()).collect()
    }
    fn installation() -> Fixture {
        let f = Fixture::new();
        let install = installation_root().unwrap();
        files::private_directory(&install).unwrap();
        let current = "a".repeat(64);
        files::atomic_json(
            &install.join("transaction.json"),
            &serde_json::json!({"current":current,"pending":null}),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            install.join("releases").join(&current),
            install.join("current"),
        )
        .unwrap();
        f
    }
    fn record(phase: &str) -> Record {
        let mut record = Record {
            operation_id: OP.into(),
            channel: "nightly".into(),
            phase: String::new(),
            message: String::new(),
            created_at: now(),
            updated_at: now(),
            current_release: "a".repeat(64),
            release_id: Some("b".repeat(64)),
            version: Some("next".into()),
            description: Some("pinned".into()),
            bin_dir: None,
            staging_root: None,
            gateways: vec![],
            contracts_sha256: None,
            supervisor_activation: None,
            legacy_mode: false,
            legacy_proof: None,
            legacy_accounts: None,
        };
        save(&mut record, phase, "fixture").unwrap();
        record
    }
    #[test]
    fn remote_update_requires_exact_review_and_admits_apply_once() {
        let f = installation();
        record("ready");
        assert!(run(&args(&["apply", OP, &"c".repeat(64)])).is_err());
        assert_eq!(load(OP).unwrap().phase, "ready");
        f.call(&["launch-update", OP, "apply"], "");
        run(&args(&["apply", OP, &"b".repeat(64)])).unwrap();
        assert_eq!(load(OP).unwrap().phase, "applying");
        run(&args(&["apply", OP, &"b".repeat(64)])).unwrap(); // no second launch
        assert!(run(&args(&["discard", OP])).is_err());
        assert!(run(&args(&["prepare", OP, "stable"])).is_err());
    }
    #[test]
    fn remote_update_expired_or_changed_installation_cannot_apply() {
        let _f = installation();
        let mut receipt = record("ready");
        receipt.created_at = now() - 3601;
        save(&mut receipt, "ready", "").unwrap();
        assert!(run(&args(&["apply", OP, &"b".repeat(64)])).is_err());
        receipt.created_at = now();
        receipt.current_release = "c".repeat(64);
        save(&mut receipt, "ready", "").unwrap();
        assert!(run(&args(&["apply", OP, &"b".repeat(64)])).is_err());
        assert_eq!(load(OP).unwrap().phase, "ready");
    }
    #[test]
    fn remote_update_uncertain_launch_is_not_repeated() {
        let f = installation();
        record("ready");
        f.fail(&["launch-update", OP, "apply"], "manager timeout");
        run(&args(&["apply", OP, &"b".repeat(64)])).unwrap();
        run(&args(&["apply", OP, &"b".repeat(64)])).unwrap();
        let mut receipt = load(OP).unwrap();
        receipt.updated_at = now() - 1231;
        files::atomic_json(&path(OP).unwrap(), &receipt).unwrap();
        f.call(
            &[
                "show",
                &format!("voyage-update-{OP}-apply.service"),
                "--property=ActiveState",
                "--value",
            ],
            "active",
        );
        run(&args(&["status", OP])).unwrap();
        assert_eq!(load(OP).unwrap().phase, "unconfirmed");
        assert!(
            run(&args(&[
                "prepare",
                "20000000-0000-4000-8000-000000000002",
                "nightly"
            ]))
            .is_err()
        );
    }
    #[test]
    fn remote_update_discard_and_latest_status_do_not_apply() {
        let _f = installation();
        run(&args(&["status", "00000000-0000-0000-0000-000000000000"])).unwrap();
        record("ready");
        run(&args(&["status", "00000000-0000-0000-0000-000000000000"])).unwrap();
        run(&args(&["discard", OP])).unwrap();
        assert_eq!(load(OP).unwrap().phase, "discarded");
        assert!(run(&args(&["apply", OP, &"b".repeat(64)])).is_err());
        assert!(run(&args(&["status", "../../elsewhere"])).is_err());
    }
    #[test]
    fn remote_update_worker_failure_is_durable_without_installation_changes() {
        let _f = installation();
        record("preparing");
        crate::fixture_tests::set_acquire("raise RuntimeError('unavailable fixture source')");
        assert!(run(&args(&["worker", "prepare", OP])).is_err());
        assert_eq!(load(OP).unwrap().phase, "failed");
        assert_eq!(current().unwrap(), "a".repeat(64));
        record("applying");
        assert!(run(&args(&["worker", "apply", OP])).is_err());
        assert_eq!(load(OP).unwrap().phase, "unconfirmed");
        assert_eq!(current().unwrap(), "a".repeat(64));
    }
}
