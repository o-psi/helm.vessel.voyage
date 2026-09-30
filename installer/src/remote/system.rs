//! Explicit root system-scope updater. Every effect follows a saved operation;
//! activation failure never runs an older binary over potentially migrated state.
use crate::{
    install::{files, release::Manifest},
    service, source, system_install,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ROOT: &str = "/var/lib/voyage/install/system-updates";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    operation_id: String,
    channel: String,
    phase: String,
    created_at: u64,
    message: String,
    current_release: String,
    fingerprint: String,
    #[serde(default)]
    manifest_digest: Option<String>,
    #[serde(default)]
    candidate_fingerprint: Option<String>,
    release_id: Option<String>,
    version: Option<String>,
    description: Option<String>,
    staging: Option<PathBuf>,
    bin: Option<PathBuf>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn uuid(id: &str) -> Result<()> {
    ensure!(
        id.len() == 36
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }),
        "invalid system update identity"
    );
    Ok(())
}
fn root() -> Result<PathBuf> {
    ensure!(
        unsafe { libc::getuid() } == 0 && unsafe { libc::geteuid() } == 0,
        "system updater requires root"
    );
    system_install::remote_host()?;
    let root = PathBuf::from(ROOT);
    // Parent installation must already exist; never bootstrap from remote commands.
    let parent = root.parent().context("system update parent missing")?;
    service::files::check_path(parent, 0)?;
    ensure!(parent.is_dir(), "system installation missing");
    service::files::directory(&root, 0, true)?;
    Ok(root)
}
fn path(id: &str) -> Result<PathBuf> {
    uuid(id)?;
    Ok(root()?.join(format!("{id}.json")))
}
fn load(id: &str) -> Result<Record> {
    let path = path(id)?;
    service::files::check_path(&path, 0)?;
    let meta = fs::symlink_metadata(&path)?;
    ensure!(
        meta.is_file() && meta.uid() == 0 && meta.mode() & 0o077 == 0 && meta.nlink() == 1,
        "system receipt is not root-private"
    );
    let record: Record = serde_json::from_slice(&files::read(&path, 65536)?)?;
    ensure!(
        record.operation_id == id,
        "system receipt identity conflict"
    );
    validate_record(&record)?;
    Ok(record)
}
fn valid_digest(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn validate_record(record: &Record) -> Result<()> {
    uuid(&record.operation_id)?;
    ensure!(
        ["stable", "nightly"].contains(&record.channel.as_str())
            && [
                "preparing",
                "ready",
                "applying",
                "complete",
                "failed",
                "unconfirmed",
                "discarded"
            ]
            .contains(&record.phase.as_str())
            && valid_digest(&record.current_release)
            && valid_digest(&record.fingerprint)
            && record.release_id.as_ref().is_none_or(|id| valid_digest(id))
            && (!matches!(record.phase.as_str(), "ready" | "applying")
                || (record.bin.is_some()
                    && record.staging.is_some()
                    && record.release_id.is_some())),
        "invalid system update receipt"
    );
    Ok(())
}
fn save(record: &mut Record, phase: &str, message: &str) -> Result<()> {
    record.phase = phase.into();
    record.message = message.into();
    files::atomic_json(&path(&record.operation_id)?, record)
}
fn output(record: &Record) {
    println!(
        "{}",
        serde_json::json!({"operation_id":record.operation_id,"channel":record.channel,"phase":record.phase,"message":record.message,"current_release":record.current_release,"release_id":record.release_id,"version":record.version,"description":record.description,"services":["voyage-vessel.service","voyage-gateway.service"],"expires_at":record.created_at+3600,"installation_scope":"system","automatic_schema_rollback":false})
    );
}
fn command(command: Command) -> Result<String> {
    command_observed(command, false)
}
fn command_observed(mut command: Command, allow_absent_unit: bool) -> Result<String> {
    let logpath = root()?.join(format!("command-{}.log", std::process::id()));
    files::write_new(&logpath, b"")?;
    let log = fs::OpenOptions::new().write(true).open(&logpath)?;
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = command.spawn()?;
    let result: Result<String> = (|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }

            ensure!(
                Instant::now() < deadline && fs::metadata(&logpath)?.len() < 1024 * 1024,
                "system worker command unconfirmed"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        let output = String::from_utf8(files::read(&logpath, 1024 * 1024)?)?;
        ensure!(
            status.success()
                || (allow_absent_unit
                    && status.code() == Some(4)
                    && output.lines().any(|line| line == "LoadState=not-found")),
            "system worker command refused"
        );
        Ok(output)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = fs::remove_file(logpath);
    result
}
fn launch(record: &Record, phase: &str) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    service::files::executable(&exe, 0)?;
    let mut manager = Command::new("/usr/bin/systemd-run");
    manager
        .args(["--quiet", "--collect", "--service-type=exec"])
        .arg(format!(
            "--unit=voyage-system-update-{}-{phase}",
            record.operation_id
        ))
        .args([
            "--property=User=root",
            "--property=RuntimeMaxSec=1200",
            "--property=TimeoutStopSec=15",
            "--property=KillMode=control-group",
            "--property=UMask=0077",
            "--property=StandardOutput=null",
            "--property=StandardError=null",
        ])
        .arg(exe)
        .args([
            "remote-update",
            "system",
            "worker",
            phase,
            &record.operation_id,
        ]);
    command(manager)?;
    Ok(())
}
fn manifest_digest(manifest: &Manifest) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest)?)
    ))
}
fn facts_match(record: &Record, facts: &system_install::RemoteFacts) -> Result<()> {
    ensure!(
        facts.current_release == record.current_release && facts.fingerprint == record.fingerprint,
        "system installation or identity changed since review"
    );
    Ok(())
}
fn cleanup(record: &mut Record) -> Result<()> {
    if let Some(staging) = &record.staging {
        ensure!(
            staging.parent() == Some(Path::new(ROOT).join(".cache/voyage/upgrades").as_path())
                && staging
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("prepare-")),
            "unrecognized system staging path"
        );
        service::files::check_path(staging, 0)?;
        if staging.try_exists()? {
            fs::remove_dir_all(staging)?;
        }
    }
    record.staging = None;
    record.bin = None;
    Ok(())
}
fn probe(bin: &Path, binary: &str, args: &[&str]) -> Result<String> {
    // A downloaded candidate is never executed as root before exact approval.
    // The explicit system-manager transient uses a fresh unprivileged identity,
    // hides homes and disables network, devices, capability and privilege gain.
    let parent = Path::new("/var/lib/voyage-runtime");
    service::files::check_path(parent, 0)?;
    let meta = fs::symlink_metadata(parent)?;
    ensure!(
        meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o777 == 0o711,
        "protected runtime parent unavailable for candidate probe"
    );
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let unit = format!("voyage-update-probe-{}-{nonce}", std::process::id());
    let directory = parent.join(&unit);
    fs::DirBuilder::new().mode(0o755).create(&directory)?;
    let executable = directory.join(binary);
    fs::copy(bin.join(binary), &executable)?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
    let mut manager = Command::new("/usr/bin/systemd-run");
    manager
        .args([
            "--quiet",
            "--wait",
            "--pipe",
            "--collect",
            "--service-type=exec",
        ])
        .arg(format!("--unit={unit}"))
        .args([
            "--property=DynamicUser=yes",
            "--property=RuntimeMaxSec=10",
            "--property=TimeoutStopSec=5",
            "--property=KillMode=control-group",
            "--property=ProtectHome=yes",
            "--property=ProtectSystem=strict",
            "--property=PrivateNetwork=yes",
            "--property=PrivateDevices=yes",
            "--property=PrivateTmp=yes",
            "--property=ProtectControlGroups=yes",
            "--property=NoNewPrivileges=yes",
            "--property=CapabilityBoundingSet=",
            "--property=RestrictSUIDSGID=yes",
            "--setenv=HOME=/nonexistent",
            "--setenv=LANG=C.UTF-8",
        ])
        .arg(&executable)
        .args(args);
    let result = command(manager);
    if result.is_ok() {
        fs::remove_dir_all(&directory)?;
    }
    // Failure retains the probe directory. systemd still owns bounded cleanup;
    // publication cannot continue on a failed/unconfirmed compatibility probe.
    result
}

fn prepare_worker(record: &mut Record) -> Result<()> {
    facts_match(record, &system_install::remote_review(None)?)?;
    let cancellation = source::Cancellation::new()?;
    let mut prepared = source::prepare_public(
        if record.channel == "nightly" {
            source::Source::Nightly
        } else {
            source::Source::Latest
        },
        &cancellation.flag,
        root()?,
    )?;
    let manifest = Manifest::inspect(&prepared.bin_dir)?;
    if manifest.id()? == record.current_release {
        record.release_id = Some(record.current_release.clone());
        record.version = Some(manifest.version.clone());
        return save(
            record,
            "complete",
            "This exact system release is already installed; no installation or service change was applied.",
        );
    }
    let reviewed = system_install::remote_review(Some(&prepared.bin_dir))?;
    facts_match(record, &reviewed)?;
    probe(&prepared.bin_dir, "vessel", &["--version"])?;
    ensure!(
        probe(
            &prepared.bin_dir,
            "voyage-installer",
            &["remote-update", "system", "protocol"]
        )?
        .trim()
            == "1",
        "candidate does not retain system updater protocol"
    );
    record.candidate_fingerprint = reviewed.candidate_fingerprint.clone();
    record.manifest_digest = Some(manifest_digest(&manifest)?);
    record.release_id = Some(manifest.id()?);
    record.version = Some(manifest.version.clone());
    record.description = Some(format!(
        "{}; system identity and units pinned; active schema rollback requires explicit compatibility review",
        prepared
            .description
            .chars()
            .filter(|c| !c.is_control())
            .take(1024)
            .collect::<String>()
    ));
    record.staging = Some(prepared.staging_root().to_owned());
    record.bin = Some(prepared.bin_dir.clone());
    save(
        record,
        "ready",
        "System update prepared. Approve the exact release; active failure retains unresolved state and does not automatically downgrade persistent schema.",
    )?;
    prepared.retain();
    Ok(())
}
fn apply_worker(record: &mut Record) -> Result<()> {
    ensure!(
        now() <= record.created_at + 3600,
        "system update review expired"
    );
    let bin = record
        .bin
        .as_ref()
        .context("system prepared source missing")?;
    service::files::check_path(bin, 0)?;
    let prepared_manifest = Manifest::inspect(bin)?;
    ensure!(
        Some(prepared_manifest.id()?) == record.release_id
            && Some(manifest_digest(&prepared_manifest)?) == record.manifest_digest,
        "system prepared source or compatibility declaration changed"
    );
    facts_match(record, &system_install::remote_review(Some(bin))?)?;
    system_install::remote_apply(bin, &record.fingerprint)?;
    let installed = system_install::remote_review(None)?;
    ensure!(
        Some(installed.current_release) == record.release_id
            && Some(installed.fingerprint) == record.candidate_fingerprint
            && Some(installed.manifest_digest) == record.manifest_digest,
        "approved system release and reviewed identity/unit/format facts not observed"
    );
    cleanup(record)?;
    save(
        record,
        "complete",
        "Approved system release and configured services observed. Verify this same Vessel and release after reconnect.",
    )
}
fn claim_worker(path: &Path) -> Result<()> {
    files::write_new(path, b"claimed-before-worker-effects\n")
        .context("system worker already claimed or uncertain; no worker effects are replayed")
}
fn worker_retired(record: &Record, phase: &str) -> Result<bool> {
    let mut manager = Command::new("/usr/bin/systemctl");
    manager
        .args(["--no-pager", "show"])
        .arg(format!(
            "voyage-system-update-{}-{phase}.service",
            record.operation_id
        ))
        .arg("--property=LoadState,ActiveState,MainPID,Job");
    let output = command_observed(manager, true)?;
    let mut values = std::collections::BTreeMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            ensure!(
                values.insert(key, value).is_none(),
                "worker observation properties conflict"
            );
        }
    }
    Ok(values.get("MainPID") == Some(&"0")
        && values.get("Job") == Some(&"")
        && matches!(
            values.get("ActiveState"),
            Some(&"inactive") | Some(&"failed")
        )
        && matches!(
            values.get("LoadState"),
            Some(&"loaded") | Some(&"not-found")
        ))
}
fn reconcile(record: &mut Record) -> Result<()> {
    let _worker = files::lock(&root()?.join("worker.lock"))?;
    let action = if record.phase == "preparing" {
        "prepare"
    } else {
        "apply"
    };
    ensure!(
        worker_retired(record, action)?,
        "system worker is still active or pending"
    );
    if action == "prepare" {
        return save(
            record,
            "failed",
            "Preparation worker is observed stopped; no installation was applied and acquisition is not replayed.",
        );
    }
    if let Ok(facts) = system_install::remote_review(None) {
        if Some(facts.current_release.clone()) == record.release_id
            && Some(facts.fingerprint.clone()) == record.candidate_fingerprint
            && Some(facts.manifest_digest.clone()) == record.manifest_digest
        {
            cleanup(record)?;
            return save(
                record,
                "complete",
                "Exact approved system source, reviewed identities and all service readiness observed after stopped worker; no operation replayed.",
            );
        }
        if facts_match(record, &facts).is_ok() {
            return save(
                record,
                "failed",
                "Exact previous system source, identities and readiness observed after stopped worker; no operation replayed.",
            );
        }
    }
    save(
        record,
        "unconfirmed",
        "Stopped system worker has unresolved source/state/service obligations. No update or unqualified old binary is replayed.",
    )
}
fn pending(phase: &str) -> bool {
    matches!(phase, "preparing" | "ready" | "applying" | "unconfirmed")
}
fn approved(record: &Record, release: &str, at: u64) -> Result<bool> {
    ensure!(
        record.release_id.as_deref() == Some(release),
        "approved system release conflict"
    );
    if matches!(
        record.phase.as_str(),
        "applying" | "complete" | "unconfirmed"
    ) {
        return Ok(false);
    }
    ensure!(
        record.phase == "ready" && at <= record.created_at + 3600,
        "system review unavailable or expired"
    );
    Ok(true)
}
pub(super) fn run(args: &[String]) -> Result<()> {
    if args == ["protocol"] {
        println!("1");
        return Ok(());
    }
    ensure!(
        args.len() >= 2,
        "system update action and identity required"
    );
    let worker = args[0] == "worker";
    let (action, id) = if worker {
        ensure!(args.len() == 3, "invalid system worker invocation");
        (args[1].as_str(), args[2].as_str())
    } else {
        (args[0].as_str(), args[1].as_str())
    };
    uuid(id)?;
    let root = root()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let coordinator = loop {
        match files::lock(&root.join("coordinator.lock")) {
            Ok(lock) => break lock,
            Err(_) if worker && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(error),
        }
    };
    if worker {
        let mut record = load(id)?;
        ensure!(
            record.phase
                == if action == "prepare" {
                    "preparing"
                } else {
                    "applying"
                },
            "system worker phase conflict"
        );
        let _worker = files::lock(&root.join("worker.lock"))?;
        claim_worker(&root.join(format!("{id}-{action}.claim")))?;
        drop(coordinator);
        let result = match action {
            "prepare" => prepare_worker(&mut record),
            "apply" => apply_worker(&mut record),
            _ => anyhow::bail!("unsupported system worker phase"),
        };
        if result.is_err() {
            let previous_observed = action == "apply"
                && system_install::remote_review(None)
                    .is_ok_and(|facts| facts_match(&record, &facts).is_ok());
            save(
                &mut record,
                if action == "prepare" || previous_observed {
                    "failed"
                } else {
                    "unconfirmed"
                },
                if action == "prepare" {
                    "System preparation failed; installation was not applied. Inspect host acquisition requirements."
                } else if previous_observed {
                    "Candidate failed; exact compatible previous source, identities and service readiness are observed restored. Live state was not replaced and the update was not replayed."
                } else {
                    "System application unconfirmed; retained lifecycle and services require inspection. No source downgrade or update is replayed."
                },
            )?;
        }
        return result;
    }
    match action {
        "prepare" => {
            ensure!(
                args.len() == 3
                    && ["stable", "nightly"].contains(&args[2].as_str())
                    && id != "00000000-0000-0000-0000-000000000000",
                "invalid system update channel/identity"
            );
            if path(id)?.try_exists()? {
                let record = load(id)?;
                ensure!(record.channel == args[2], "system update identity conflict");
                output(&record);
                return Ok(());
            }
            ensure!(
                fs::read_dir(&root)?.count() < 260,
                "system receipt retention limit reached"
            );
            for entry in fs::read_dir(&root)? {
                let path = entry?.path();
                if path.extension().is_some_and(|x| x == "json") {
                    let other = load(
                        path.file_stem()
                            .context("receipt name missing")?
                            .to_str()
                            .context("invalid receipt name")?,
                    )?;
                    ensure!(!pending(&other.phase), "system update already pending");
                }
            }
            let facts = system_install::remote_review(None)?;
            let mut record = Record {
                operation_id: id.into(),
                channel: args[2].clone(),
                phase: String::new(),
                created_at: now(),
                message: String::new(),
                current_release: facts.current_release,
                fingerprint: facts.fingerprint,
                manifest_digest: None,
                candidate_fingerprint: None,
                release_id: None,
                version: None,
                description: None,
                staging: None,
                bin: None,
            };
            save(
                &mut record,
                "preparing",
                "Acquiring a public release for explicit system review; installation is unchanged.",
            )?;
            let _ = launch(&record, "prepare");
            output(&record);
        }
        "status" => {
            ensure!(args.len() == 2, "invalid system status");
            let mut record = if id == "00000000-0000-0000-0000-000000000000" {
                let mut records = Vec::new();
                for entry in fs::read_dir(&root)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|x| x == "json") {
                        records.push(load(
                            path.file_stem()
                                .unwrap()
                                .to_str()
                                .context("invalid receipt name")?,
                        )?);
                    }
                }
                records.sort_by_key(|r| r.created_at);
                if let Some(record) = records.pop() {
                    record
                } else {
                    println!("{{\"phase\":\"idle\",\"installation_scope\":\"system\"}}");
                    return Ok(());
                }
            } else {
                load(id)?
            };
            if matches!(
                record.phase.as_str(),
                "preparing" | "applying" | "unconfirmed"
            ) {
                let _ = reconcile(&mut record);
            }
            output(&record);
        }
        "apply" => {
            ensure!(args.len() == 3, "system reviewed release required");
            let mut record = load(id)?;
            if approved(&record, &args[2], now())? {
                facts_match(&record, &system_install::remote_review(None)?)?;
                save(
                    &mut record,
                    "applying",
                    "Applying approved system release through an independent updater; temporary disconnection is expected.",
                )?;
                let _ = launch(&record, "apply");
            }
            output(&record);
        }
        "discard" => {
            ensure!(args.len() == 2, "invalid system discard");
            let mut record = load(id)?;
            if record.phase != "discarded" {
                ensure!(
                    record.phase == "ready",
                    "only unapplied system review can be discarded"
                );
                cleanup(&mut record)?;
                save(
                    &mut record,
                    "discarded",
                    "Prepared system update discarded without applying.",
                )?;
            }
            output(&record);
        }
        _ => anyhow::bail!("unsupported system update action"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready() -> Record {
        Record { operation_id:"10000000-0000-4000-8000-000000000001".into(), channel:"nightly".into(), phase:"ready".into(), created_at:100,
            manifest_digest:Some("f".repeat(64)),candidate_fingerprint:Some("e".repeat(64)), message:String::new(), current_release:"a".repeat(64),fingerprint:"b".repeat(64), release_id:Some("c".repeat(64)),version:Some("1.0.3".into()),description:None,
            staging:Some("/var/lib/voyage/install/system-updates/.cache/voyage/upgrades/prepare-fixture".into()),bin:Some("/var/lib/voyage/install/system-updates/.cache/voyage/upgrades/prepare-fixture/release/bin".into()) }
    }
    #[test]
    fn remote_system_approval_is_exact_expiring_and_does_not_replay() {
        let mut record = ready();
        assert!(approved(&record, &"c".repeat(64), 3700).unwrap());
        assert!(approved(&record, &"c".repeat(64), 3701).is_err());
        assert!(approved(&record, &"d".repeat(64), 101).is_err());
        for phase in ["applying", "complete", "unconfirmed"] {
            record.phase = phase.into();
            assert!(!approved(&record, &"c".repeat(64), 99999).unwrap());
        }
        for phase in ["preparing", "failed", "discarded"] {
            record.phase = phase.into();
            assert!(approved(&record, &"c".repeat(64), 101).is_err());
        }
    }
    #[test]
    fn remote_system_receipts_refuse_unknown_or_incomplete_admission() {
        let mut record = ready();
        validate_record(&record).unwrap();
        record.phase = "unknown-outcome".into();
        assert!(validate_record(&record).is_err());
        record = ready();
        record.bin = None;
        assert!(validate_record(&record).is_err());
        record = ready();
        record.fingerprint = "changed".into();
        assert!(validate_record(&record).is_err());
        let mut value = serde_json::to_value(ready()).unwrap();
        value["uid"] = serde_json::json!(0);
        assert!(serde_json::from_value::<Record>(value).is_err());
    }
    #[test]
    fn remote_system_fingerprint_change_requires_fresh_review() {
        let record = ready();
        let mut facts = system_install::RemoteFacts {
            current_release: record.current_release.clone(),
            current_version: "1.0.2".into(),
            fingerprint: record.fingerprint.clone(),
            active: true,
            candidate_release: record.release_id.clone(),
            candidate_version: record.version.clone(),
            candidate_fingerprint: record.candidate_fingerprint.clone(),
            manifest_digest: record.manifest_digest.clone().unwrap(),
        };
        facts_match(&record, &facts).unwrap();
        facts.fingerprint = "d".repeat(64);
        assert!(facts_match(&record, &facts).is_err());
        facts.fingerprint = record.fingerprint.clone();
        facts.current_release = "e".repeat(64);
        assert!(facts_match(&record, &facts).is_err());
    }

    #[test]
    fn remote_system_worker_claim_survives_duplicate_and_process_loss() {
        let fixture = crate::fixture_tests::Fixture::new();
        let path = fixture.root.join("worker.claim");
        claim_worker(&path).unwrap();
        assert!(
            claim_worker(&path)
                .unwrap_err()
                .to_string()
                .contains("no worker effects are replayed")
        );
        assert_eq!(
            files::read(&path, 128).unwrap(),
            b"claimed-before-worker-effects\n"
        );
    }
    #[test]
    fn ordinary_remote_system_request_refuses_before_protected_effects() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        assert!(root().unwrap_err().to_string().contains("requires root"));
    }
}
