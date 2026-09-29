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
    read_operation_at(Path::new(OPERATION), 0)
}
fn read_operation_at(path: &Path, expected_uid: u32) -> Result<Option<Operation>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    service::files::check_path(path, expected_uid)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.uid() == expected_uid && m.mode() & 0o077 == 0 && m.nlink() == 1,
        "lifecycle record is not owner-private"
    );
    let op: Operation = serde_json::from_slice(&files::read(path, 196608)?)?;
    validate_operation(&op)?;
    Ok(Some(op))
}
fn validate_operation(op: &Operation) -> Result<()> {
    ensure!(
        op.schema_version == 1
            && op.previous.schema_version == 1
            && valid_release(&op.previous.release)
            && op
                .candidate
                .as_ref()
                .is_none_or(|c| c.schema_version == 1 && valid_release(&c.release)),
        "invalid lifecycle record"
    );
    ensure!(
        matches!(op.kind.as_str(), "upgrade" | "rollback" | "uninstall")
            && matches!(
                op.phase.as_str(),
                "prepared"
                    | "staging"
                    | "stopping"
                    | "publishing"
                    | "activating"
                    | "unresolved-activation"
                    | "saving"
                    | "complete"
            )
            && (op.kind == "uninstall") == op.candidate.is_none(),
        "invalid lifecycle operation or candidate"
    );
    ensure!(
        matches!(op.previous.phase.as_str(), "active" | "inactive")
            && op
                .candidate
                .as_ref()
                .is_none_or(|c| matches!(c.phase.as_str(), "active" | "inactive")),
        "lifecycle release has an incomplete installation phase"
    );
    if let Some(candidate) = &op.candidate {
        let old = serde_json::to_value(&op.previous)?;
        let next = serde_json::to_value(candidate)?;
        for field in [
            "execution_user",
            "execution_uid",
            "execution_gid",
            "execution_home",
            "execution_groups",
            "gateway_user",
            "gateway_uid",
            "gateway_gid",
            "gateway_home",
            "gateway_groups",
            "gateway_origin",
            "credential_key",
            "credential_unit",
            "credential_unit_sha256",
        ] {
            ensure!(
                old[field] == next[field],
                "lifecycle journal changes configured identity or external provisioner"
            );
        }
        ensure!(
            op.kind != "upgrade"
                || candidate.phase != "active"
                || op.phase != "complete"
                || op.candidate_start_attempted,
            "completed active upgrade lacks durable startup attempt"
        );
    }
    ensure!(
        !op.candidate_start_attempted
            || (op.kind == "upgrade" && op.candidate.as_ref().is_some_and(|r| r.phase == "active")),
        "candidate startup record conflicts with operation identity"
    );
    ensure!(
        !matches!(op.phase.as_str(), "activating" | "unresolved-activation")
            || op.candidate_start_attempted,
        "activation phase lacks durable startup attempt"
    );
    Ok(())
}
fn completed(op: &Operation) -> Result<()> {
    validate_operation(op)?;
    ensure!(
        op.phase == "complete",
        "system lifecycle remains unresolved; inspect exact services and retained records; no operation is automatically replayed"
    );
    Ok(())
}
fn rollback_eligible(op: &Operation, current: &Record) -> Result<()> {
    completed(op)?;
    ensure!(
        op.kind == "upgrade"
            && !op.candidate_start_attempted
            && op.previous.phase == "inactive"
            && current.phase == "inactive"
            && op
                .candidate
                .as_ref()
                .is_some_and(|r| r.release == current.release),
        "rollback refused: candidate may have opened persistent state; schema compatibility review is required"
    );
    Ok(())
}
fn write_operation(op: &Operation) -> Result<()> {
    files::atomic_json(Path::new(OPERATION), op)
}
pub(super) fn report_pending() -> Result<()> {
    if let Some(op) = read_operation()? {
        println!("System lifecycle: {} / {}", op.kind, op.phase);
        completed(&op)?;
    }
    Ok(())
}
fn installed() -> Result<Plan> {
    let record = read_record(CONFIG)?;
    verify_default_execution(&record)?;
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
        rollback_eligible(&op, &old.record)?;
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
    fn record(id: char, phase: &str) -> Record {
        Record {
            schema_version: 1,
            phase: phase.into(),
            release: id.to_string().repeat(64),
            version: "1.0.3".into(),
            execution_user: "ordinary".into(),
            execution_uid: 1000,
            execution_gid: 1000,
            execution_home: "/home/ordinary".into(),
            execution_groups: vec![1000],
            gateway_user: "gateway".into(),
            gateway_uid: 1001,
            gateway_gid: 1001,
            gateway_home: "/home/gateway".into(),
            gateway_groups: vec![1001],
            gateway_origin: "https://helm.example.test".into(),
            credential_key: "/run/voyage-secrets/key".into(),
            credential_unit: "voyage-key.service".into(),
            credential_unit_sha256: "c".repeat(64),
            root_unit: "root unit".into(),
            gateway_unit: "gateway unit".into(),
            start_requested: phase == "active",
        }
    }
    fn operation(phase: &str, active: bool) -> Operation {
        let state = if active { "active" } else { "inactive" };
        Operation {
            schema_version: 1,
            kind: "upgrade".into(),
            phase: phase.into(),
            previous: record('a', state),
            candidate: Some(record('b', state)),
            candidate_start_attempted: active,
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("target")
                .join(format!(
                    "lifecycle-fixture-{}-{}",
                    std::process::id(),
                    super::super::kernel_uuid().unwrap()
                ));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
        fn journal(&self) -> PathBuf {
            self.0.join("lifecycle.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn installer_default_identity_preserves_explicit_account_without_root_or_primary_group() {
        let mut explicit = record('a', "inactive");
        explicit.execution_groups = vec![1000, 1002, 1003];
        let first = super::super::default_execution(&explicit).unwrap();
        let second = super::super::default_execution(&explicit).unwrap();
        super::super::validate_default_execution(&explicit, &first).unwrap();
        for (field, changed) in [
            ("uid", serde_json::json!(0)),
            ("enabled", serde_json::json!(false)),
            ("authority", serde_json::json!("administrator")),
            ("home", serde_json::json!("/root")),
            ("supplementary_groups", serde_json::json!([0])),
        ] {
            let mut invalid = first.clone();
            invalid[field] = changed;
            assert!(super::super::validate_default_execution(&explicit, &invalid).is_err());
        }
        let mut invalid = first.clone();
        invalid["unexpected"] = true.into();
        assert!(super::super::validate_default_execution(&explicit, &invalid).is_err());
        assert_eq!(first["uid"], 1000);
        assert_eq!(first["gid"], 1000);
        assert_eq!(first["user_name"], "ordinary");
        assert_eq!(first["home"], "/home/ordinary");
        assert_eq!(
            first["supplementary_groups"],
            serde_json::json!([1002, 1003])
        );
        assert_eq!(first["authority"], "ordinary");
        assert_eq!(first["identity"]["revision"], 1);
        assert_ne!(first["identity"]["id"], first["account_context"]["id"]);
        assert_ne!(first["identity"]["id"], second["identity"]["id"]);
        explicit.execution_groups.push(0);
        assert!(super::super::default_execution(&explicit).is_err());
        explicit.execution_groups.pop();
        explicit.execution_uid = 0;
        assert!(super::super::default_execution(&explicit).is_err());
    }
    #[test]
    fn interrupted_journal_survives_reopen_and_never_admits_replay() {
        let fixture = Fixture::new();
        let path = fixture.journal();
        let uid = unsafe { libc::geteuid() };
        assert!(read_operation_at(&path, uid).unwrap().is_none());
        for phase in ["prepared", "staging", "stopping", "publishing", "saving"] {
            let op = operation(phase, false);
            files::atomic_json(&path, &op).unwrap();
            let reopened = read_operation_at(&path, uid).unwrap().unwrap();
            assert_eq!(reopened.phase, phase);
            assert_eq!(reopened.previous.release, "a".repeat(64));
            assert_eq!(reopened.candidate.as_ref().unwrap().release, "b".repeat(64));
            assert!(
                completed(&reopened)
                    .unwrap_err()
                    .to_string()
                    .contains("no operation is automatically replayed")
            );
            assert!(rollback_eligible(&reopened, reopened.candidate.as_ref().unwrap()).is_err());
        }
        let op = operation("unresolved-activation", true);
        files::atomic_json(&path, &op).unwrap();
        let reopened = read_operation_at(&path, uid).unwrap().unwrap();
        assert!(reopened.candidate_start_attempted);
        assert!(completed(&reopened).is_err());
        assert!(rollback_eligible(&reopened, reopened.candidate.as_ref().unwrap()).is_err());
        assert!(
            fs::read_dir(&fixture.0)
                .unwrap()
                .all(|p| p.unwrap().file_name() == "lifecycle.json")
        );
    }
    #[test]
    fn rollback_requires_never_started_candidate_and_exact_retained_identity() {
        let op = operation("complete", false);
        let current = op.candidate.as_ref().unwrap();
        rollback_eligible(&op, current).unwrap();
        let mut replaced = record('d', "inactive");
        assert!(rollback_eligible(&op, &replaced).is_err());
        replaced.release = current.release.clone();
        replaced.phase = "active".into();
        assert!(rollback_eligible(&op, &replaced).is_err());
        let mut active = operation("complete", true);
        active.candidate.as_mut().unwrap().phase = "inactive".into();
        assert!(rollback_eligible(&active, current).is_err());
        let active = operation("complete", true);
        assert!(completed(&active).is_ok());
        assert!(rollback_eligible(&active, active.candidate.as_ref().unwrap()).is_err());
    }
    #[test]
    fn journal_rejects_changed_schema_unsafe_inode_and_inconsistent_attempt() {
        let fixture = Fixture::new();
        let path = fixture.journal();
        let uid = unsafe { libc::geteuid() };
        let op = operation("complete", false);
        let mut value = serde_json::to_value(&op).unwrap();
        value["extra"] = true.into();
        files::atomic_json(&path, &value).unwrap();
        assert!(read_operation_at(&path, uid).is_err());
        value.as_object_mut().unwrap().remove("extra");
        value["schema_version"] = 2.into();
        files::atomic_json(&path, &value).unwrap();
        assert!(read_operation_at(&path, uid).is_err());
        files::atomic_json(&path, &op).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_operation_at(&path, uid).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&path, fixture.0.join("alias")).unwrap();
        assert!(read_operation_at(&path, uid).is_err());
        fs::remove_file(fixture.0.join("alias")).unwrap();
        let linked = fixture.0.join("symlink.json");
        std::os::unix::fs::symlink(&path, &linked).unwrap();
        assert!(read_operation_at(&linked, uid).is_err());
        let mut changed_identity = operation("complete", false);
        changed_identity.candidate.as_mut().unwrap().execution_uid = 0;
        assert!(validate_operation(&changed_identity).is_err());
        let mut missing_attempt = operation("activating", false);
        assert!(validate_operation(&missing_attempt).is_err());
        missing_attempt.phase = "complete".into();
        missing_attempt.kind = "uninstall".into();
        assert!(validate_operation(&missing_attempt).is_err());
        missing_attempt.candidate = None;
        validate_operation(&missing_attempt).unwrap();
        assert!(rollback_eligible(&missing_attempt, &op.previous).is_err());
    }
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
