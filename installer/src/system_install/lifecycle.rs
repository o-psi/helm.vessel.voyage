//! Root-local lifecycle transactions. They never restore an old binary after a
//! candidate may have opened persistent state; schema rollback is not assumed.
use super::*;

const OPERATION: &str = "/var/lib/voyage/install/lifecycle.json";
const TRANSACTION: &str = "/var/lib/voyage/install/transaction.json";
const CONFIG: &str = "/etc/voyage/system-install.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    schema_version: u32,
    kind: String,
    phase: String,
    previous: Record,
    candidate: Option<Record>,
    candidate_start_attempted: bool,
}

struct Arguments {
    kind: String,
    bin: Option<PathBuf>,
    dry_run: bool,
}
impl Arguments {
    fn parse(args: &[String]) -> Result<Self> {
        let kind = args.first().context("system lifecycle command missing")?;
        ensure!(
            matches!(kind.as_str(), "upgrade" | "rollback" | "uninstall"),
            "unsupported system operation"
        );
        let mut scope = false;
        let mut bin = None;
        let mut dry_run = false;
        let mut iter = args[1..].iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--scope" => {
                    ensure!(
                        !scope && iter.next().map(String::as_str) == Some("system"),
                        "expected one --scope system"
                    );
                    scope = true;
                }
                "--bin-dir" if kind == "upgrade" => {
                    ensure!(bin.is_none(), "repeated --bin-dir");
                    let path = PathBuf::from(
                        iter.next()
                            .context("--bin-dir needs an absolute directory")?,
                    );
                    ensure!(path.is_absolute(), "system source must be absolute");
                    bin = Some(path);
                }
                "--dry-run" => {
                    ensure!(!dry_run, "repeated --dry-run");
                    dry_run = true;
                }
                _ => bail!("unsupported system lifecycle argument: {arg}"),
            }
        }
        ensure!(
            scope && (kind != "upgrade" || bin.is_some()),
            "system upgrade requires --scope system and --bin-dir ABS"
        );
        Ok(Self {
            kind: kind.clone(),
            bin,
            dry_run,
        })
    }
}

fn read_record(path: &str) -> Result<Record> {
    service::files::check_path(Path::new(path), 0)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && m.uid() == 0 && m.mode() & 0o077 == 0 && m.nlink() == 1,
        "system record is not root-private"
    );
    let r: Record = serde_json::from_slice(&files::read(Path::new(path), 65536)?)?;
    ensure!(
        r.schema_version == 1 && valid_release(&r.release),
        "invalid system record identity"
    );
    Ok(r)
}
fn valid_release(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn read_operation() -> Result<Option<Operation>> {
    if !Path::new(OPERATION).try_exists()? {
        return Ok(None);
    }
    service::files::check_path(Path::new(OPERATION), 0)?;
    let m = fs::symlink_metadata(OPERATION)?;
    ensure!(
        m.uid() == 0 && m.mode() & 0o077 == 0 && m.nlink() == 1,
        "lifecycle record is not root-private"
    );
    let op: Operation = serde_json::from_slice(&files::read(Path::new(OPERATION), 196608)?)?;
    ensure!(
        op.schema_version == 1
            && valid_release(&op.previous.release)
            && op
                .candidate
                .as_ref()
                .is_none_or(|c| valid_release(&c.release)),
        "invalid lifecycle record"
    );
    Ok(Some(op))
}
fn write_operation(op: &Operation) -> Result<()> {
    files::atomic_json(Path::new(OPERATION), op)
}
pub(super) fn report_pending() -> Result<()> {
    if let Some(op) = read_operation()? {
        println!("System lifecycle: {} / {}", op.kind, op.phase);
        ensure!(
            matches!(op.phase.as_str(), "complete" | "rolled-back"),
            "system lifecycle remains unresolved; inspect exact services and retained records; no operation is automatically replayed"
        );
    }
    Ok(())
}
fn installed() -> Result<Plan> {
    let record = read_record(CONFIG)?;
    let transaction = read_record(TRANSACTION)?;
    ensure!(
        serde_json::to_vec(&record)? == serde_json::to_vec(&transaction)?
            && matches!(record.phase.as_str(), "active" | "inactive"),
        "system configuration and completed transaction differ"
    );
    for (name, uid, gid, home, groups) in [
        (
            &record.execution_user,
            record.execution_uid,
            record.execution_gid,
            &record.execution_home,
            &record.execution_groups,
        ),
        (
            &record.gateway_user,
            record.gateway_uid,
            record.gateway_gid,
            &record.gateway_home,
            &record.gateway_groups,
        ),
    ] {
        let current = system_preflight::account(name)?;
        ensure!(
            current.uid == uid
                && current.gid == gid
                && current.home == *home
                && current.groups == *groups,
            "configured system account changed since installation"
        );
    }
    let release = Path::new(RELEASE_ROOT)
        .join("releases")
        .join(&record.release);
    let manifest: Manifest =
        serde_json::from_slice(&files::read(&release.join("release.json"), 1024 * 1024)?)?;
    ensure!(
        manifest.id()? == record.release && manifest.version == record.version,
        "system release identity changed"
    );
    manifest.verify_system(&release)?;
    let plan = Plan {
        source: release.join("bin"),
        manifest,
        record,
    };
    verify_effective(&plan)?;
    verify_provisioner(&plan)?;
    if plan.record.phase == "active" {
        pid(ROOT_UNIT, 0, &release.join("bin/vessel"))?;
        pid(
            GATEWAY_UNIT,
            plan.record.gateway_uid,
            &release.join("bin/vessel"),
        )?;
        preflight()?;
    } else {
        inactive()?;
    }
    Ok(plan)
}
fn inactive() -> Result<()> {
    for unit in [ROOT_UNIT, GATEWAY_UNIT] {
        ensure!(
            query(unit, "MainPID")? == "0"
                && matches!(query(unit, "ActiveState")?.as_str(), "inactive" | "failed"),
            "system service stop is not observed"
        );
    }
    Ok(())
}
fn stop() -> Result<()> {
    ctl(&["--no-block", "stop", GATEWAY_UNIT, ROOT_UNIT])?;
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if inactive().is_ok() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "system stop remains unobserved; independent voyages and release retained"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn publish(next: &Record, previous: &Record) -> Result<()> {
    for (name, content, expected) in [
        (ROOT_UNIT, &next.root_unit, &previous.root_unit),
        (GATEWAY_UNIT, &next.gateway_unit, &previous.gateway_unit),
    ] {
        service::files::check_path(&Path::new(UNIT_ROOT).join(name), 0)?;
        service::files::replace(&Path::new(UNIT_ROOT).join(name), content, Some(expected))?;
    }
    ctl(&["daemon-reload"])?;
    Ok(())
}
fn save(record: &Record) -> Result<()> {
    files::atomic_json(Path::new(CONFIG), record)?;
    files::atomic_json(Path::new(TRANSACTION), record)
}
fn candidate(old: &Plan, source: &Path) -> Result<Plan> {
    service::files::check_path(source, 0)?;
    ensure!(
        source
            .parent()
            .is_some_and(|p| p.join("release.json").is_file()),
        "system upgrade requires a complete verified release"
    );
    let manifest = Manifest::inspect(source)?;
    let mut record = old.record.clone();
    ensure!(
        semver::Version::parse(&manifest.version)? > semver::Version::parse(&old.manifest.version)?,
        "system upgrade must advance the installed version"
    );
    record.release = manifest.id()?;
    record.version = manifest.version.clone();
    ensure!(
        record.release != old.record.release && manifest.target == old.manifest.target,
        "upgrade needs a different release for the installed target"
    );
    let units = system_service::Plan {
        release_root: RELEASE_ROOT.into(),
        bin: Path::new(RELEASE_ROOT)
            .join("releases")
            .join(&record.release)
            .join("bin"),
        control: CONTROL.into(),
        gateway_state: GATEWAY_STATE.into(),
        execution_user: record.execution_user.clone(),
        execution_uid: record.execution_uid,
        gateway_user: record.gateway_user.clone(),
        gateway_uid: record.gateway_uid,
        origin: record.gateway_origin.clone(),
        socket: "voyage-system".into(),
        credential_key: record.credential_key.clone(),
        credential_unit: record.credential_unit.clone(),
    }
    .render()?;
    record.root_unit = units.root;
    record.gateway_unit = units.gateway;
    Ok(Plan {
        source: source.to_owned(),
        manifest,
        record,
    })
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let arguments = Arguments::parse(args)?;
    root_host()?;
    service::files::check_path(&Path::new(CONTROL_PARENT).join("install/lock"), 0)?;
    let _lock = files::lock(&Path::new(CONTROL_PARENT).join("install/lock"))?;
    report_pending()?;
    let old = installed()?;
    let next = arguments
        .bin
        .as_ref()
        .map(|b| candidate(&old, b))
        .transpose()?;
    if arguments.kind == "rollback" {
        let op = read_operation()?.context("no retained system upgrade")?;
        ensure!(
            op.kind == "upgrade"
                && op.phase == "complete"
                && !op.candidate_start_attempted
                && old.record.phase == "inactive"
                && op
                    .candidate
                    .as_ref()
                    .is_some_and(|r| r.release == old.record.release),
            "rollback refused: candidate may have opened persistent state; schema compatibility review is required"
        );
    }
    println!(
        "System {}: release {} ({})",
        arguments.kind, old.record.release, old.record.version
    );
    println!(
        "Identity, private control/runtime state and independent voyages are retained. Active candidate failure never automatically runs an older binary over potentially migrated state."
    );
    if arguments.dry_run {
        println!("Dry run complete; no changes applied.");
        return Ok(());
    }
    // Recheck source/identity/unit facts under the same lock immediately before journaling effects.
    let renewed = installed()?;
    ensure!(
        serde_json::to_vec(&renewed.record)? == serde_json::to_vec(&old.record)?,
        "installation changed since review"
    );
    let mut op = Operation {
        schema_version: 1,
        kind: arguments.kind.clone(),
        phase: "prepared".into(),
        previous: old.record.clone(),
        candidate: next.as_ref().map(|n| n.record.clone()),
        candidate_start_attempted: false,
    };
    if arguments.kind == "rollback" {
        let previous = read_operation()?
            .context("retained upgrade disappeared")?
            .previous;
        ensure!(
            previous.phase == "inactive",
            "previous release was active; rollback requires schema review"
        );
        let root = Path::new(RELEASE_ROOT)
            .join("releases")
            .join(&previous.release);
        let manifest: Manifest =
            serde_json::from_slice(&files::read(&root.join("release.json"), 1024 * 1024)?)?;
        ensure!(
            manifest.id()? == previous.release && manifest.version == previous.version,
            "rollback release changed"
        );
        manifest.verify_system(&root)?;
        op.candidate = Some(previous);
    }
    write_operation(&op)?;
    if let Some(next) = &next {
        op.phase = "staging".into();
        write_operation(&op)?;
        let release = Path::new(RELEASE_ROOT)
            .join("releases")
            .join(&next.record.release);
        if release.try_exists()? {
            let retained: Manifest =
                serde_json::from_slice(&files::read(&release.join("release.json"), 1024 * 1024)?)?;
            ensure!(
                retained.id()? == next.record.release && retained.version == next.record.version,
                "retained candidate manifest differs from the reviewed release"
            );
            next.manifest.verify_system(&release)?;
        } else {
            next.manifest.stage_system(&next.source, &release)?;
        }
    }
    op.phase = "stopping".into();
    write_operation(&op)?;
    stop()?;
    op.phase = "publishing".into();
    write_operation(&op)?;
    if arguments.kind == "uninstall" {
        ctl(&["disable", ROOT_UNIT, GATEWAY_UNIT])?;
        for (name, content) in [
            (GATEWAY_UNIT, &old.record.gateway_unit),
            (ROOT_UNIT, &old.record.root_unit),
        ] {
            service::files::remove_reviewed(&Path::new(UNIT_ROOT).join(name), content)?;
        }
        ctl(&["daemon-reload"])?;
        inactive()?;
        let mut record = old.record.clone();
        record.phase = "uninstalled-retained".into();
        save(&record)?;
    } else {
        let record = op.candidate.as_ref().context("candidate missing")?;
        publish(record, &old.record)?;
        let release = Path::new(RELEASE_ROOT)
            .join("releases")
            .join(&record.release);
        let manifest: Manifest =
            serde_json::from_slice(&files::read(&release.join("release.json"), 1024 * 1024)?)?;
        let plan = Plan {
            source: release.join("bin"),
            manifest,
            record: record.clone(),
        };
        verify_effective(&plan)?;
        verify_unit_syntax()?;
        verify_provisioner(&plan)?;
        if record.phase == "active" {
            op.phase = "activating".into();
            op.candidate_start_attempted = true;
            write_operation(&op)?;
            if let Err(error) = activate(&plan) {
                let stopped = stop().is_ok();
                op.phase = "unresolved-activation".into();
                write_operation(&op)?;
                return Err(error.context(format!("candidate activation failed; stopped={stopped}; old binary is not restarted without schema compatibility review")));
            }
        } else {
            inactive()?;
        }
        op.phase = "saving".into();
        write_operation(&op)?;
        save(&plan.record)?;
    }
    op.phase = "complete".into();
    write_operation(&op)?;
    println!(
        "System {} complete; releases and private state retained.",
        arguments.kind
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_needs_explicit_scope_and_exact_arguments() {
        for args in [
            vec!["upgrade", "--scope", "system"],
            vec!["uninstall", "--scope", "user"],
            vec!["rollback", "--scope", "system", "--start"],
            vec!["upgrade", "--scope", "system", "--bin-dir", "relative"],
        ] {
            assert!(
                Arguments::parse(&args.iter().map(|v| v.to_string()).collect::<Vec<_>>()).is_err()
            );
        }
        for args in [
            vec![
                "upgrade",
                "--scope",
                "system",
                "--bin-dir",
                "/root/release/bin",
                "--dry-run",
            ],
            vec!["rollback", "--scope", "system"],
            vec!["uninstall", "--scope", "system", "--dry-run"],
        ] {
            assert!(
                Arguments::parse(&args.iter().map(|v| v.to_string()).collect::<Vec<_>>()).is_ok()
            );
        }
    }
    #[test]
    fn retained_release_identity_is_bounded() {
        assert!(valid_release(&"a".repeat(64)));
        assert!(!valid_release(&"a".repeat(65)));
        assert!(!valid_release("../../etc"));
    }
}
