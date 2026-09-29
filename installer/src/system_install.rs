//! Explicit Linux system-scope installation. Fresh publication is deliberately
//! separate from the ordinary-user layout; update/adoption remain later gates.
use crate::{
    install::files, install::release::Manifest, service, system_preflight, system_service,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, DirBuilder},
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const RELEASE_ROOT: &str = "/opt/voyage";
const CONTROL_PARENT: &str = "/var/lib/voyage";
const CONTROL: &str = "/var/lib/voyage/vessel";
const RUNTIME: &str = "/var/lib/voyage-runtime";
const GATEWAY_STATE: &str = "/var/lib/voyage-gateway";
const CONFIG_ROOT: &str = "/etc/voyage";
const UNIT_ROOT: &str = "/etc/systemd/system";
const ROOT_UNIT: &str = "voyage-vessel.service";
const GATEWAY_UNIT: &str = "voyage-gateway.service";

struct Options {
    bin: PathBuf,
    execution_user: String,
    gateway_user: String,
    origin: String,
    credential_key: PathBuf,
    credential_unit: String,
    start: bool,
    dry_run: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        ensure!(
            args.first().map(String::as_str) == Some("install"),
            "system scope currently requires an explicit fresh install; update, rollback and adoption are not yet available"
        );
        let mut bin = None;
        let mut execution_user = None;
        let mut gateway_user = None;
        let mut origin = None;
        let mut credential_key = None;
        let mut credential_unit = None;
        let mut start = false;
        let mut no_start = false;
        let mut dry_run = false;
        let mut scope = false;
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
                "--bin-dir" => {
                    ensure!(bin.is_none(), "repeated --bin-dir");
                    bin = Some(PathBuf::from(
                        iter.next()
                            .context("--bin-dir needs an absolute directory")?,
                    ));
                }
                "--execution-user" => {
                    ensure!(execution_user.is_none(), "repeated --execution-user");
                    execution_user = Some(
                        iter.next()
                            .context("--execution-user needs a name")?
                            .clone(),
                    );
                }
                "--gateway-user" => {
                    ensure!(gateway_user.is_none(), "repeated --gateway-user");
                    gateway_user =
                        Some(iter.next().context("--gateway-user needs a name")?.clone());
                }
                "--gateway-origin" => {
                    ensure!(origin.is_none(), "repeated --gateway-origin");
                    origin = Some(
                        iter.next()
                            .context("--gateway-origin needs an HTTPS origin")?
                            .clone(),
                    );
                }
                "--credential-key" => {
                    ensure!(credential_key.is_none(), "repeated --credential-key");
                    credential_key = Some(PathBuf::from(
                        iter.next().context("--credential-key needs a /run path")?,
                    ));
                }
                "--credential-unit" => {
                    ensure!(credential_unit.is_none(), "repeated --credential-unit");
                    credential_unit = Some(
                        iter.next()
                            .context("--credential-unit needs a service name")?
                            .clone(),
                    );
                }
                "--start" => ensure!(!start && !no_start, "repeated or conflicting --start"),
                "--no-start" => {
                    ensure!(!start && !no_start, "repeated or conflicting --no-start");
                    no_start = true;
                }
                "--dry-run" => ensure!(!dry_run, "repeated --dry-run"),
                _ => bail!("unsupported system installation argument: {arg}"),
            }
            if arg == "--start" {
                start = true;
            }
            if arg == "--dry-run" {
                dry_run = true;
            }
        }
        ensure!(scope, "system install needs --scope system");
        let bin = bin.context("system install needs --bin-dir ABS from a verified archive")?;
        ensure!(bin.is_absolute(), "system source must be absolute");
        let execution_user = execution_user.context("system install needs --execution-user")?;
        let gateway_user = gateway_user.context("system install needs --gateway-user")?;
        ensure!(
            system_preflight::valid_name(&execution_user)
                && system_preflight::valid_name(&gateway_user)
                && execution_user != gateway_user,
            "system execution and gateway need distinct explicit ordinary accounts"
        );
        Ok(Self {
            bin,
            execution_user,
            gateway_user,
            origin: origin.context("system install needs --gateway-origin")?,
            credential_key: credential_key.context("system install needs --credential-key")?,
            credential_unit: credential_unit.context("system install needs --credential-unit")?,
            start,
            dry_run,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    phase: String,
    release: String,
    version: String,
    execution_user: String,
    execution_uid: u32,
    execution_gid: u32,
    execution_home: PathBuf,
    execution_groups: Vec<u32>,
    gateway_user: String,
    gateway_uid: u32,
    gateway_gid: u32,
    gateway_home: PathBuf,
    gateway_groups: Vec<u32>,
    gateway_origin: String,
    credential_key: PathBuf,
    credential_unit: String,
    credential_unit_sha256: String,
    root_unit: String,
    gateway_unit: String,
    start_requested: bool,
}

struct Plan {
    source: PathBuf,
    manifest: Manifest,
    record: Record,
}

impl Plan {
    fn prepare(options: &Options) -> Result<Self> {
        ensure!(
            Path::new("/run/systemd/system").is_dir(),
            "system install needs a running systemd system manager"
        );
        for path in [
            RELEASE_ROOT,
            CONTROL_PARENT,
            RUNTIME,
            GATEWAY_STATE,
            CONFIG_ROOT,
        ] {
            service::files::check_path(Path::new(path), 0)?;
            ensure!(
                !Path::new(path).exists(),
                "system install refuses existing state at {path}; adoption needs an exact inventory"
            );
        }
        for name in [ROOT_UNIT, GATEWAY_UNIT] {
            let path = Path::new(UNIT_ROOT).join(name);
            service::files::check_path(&path, 0)?;
            ensure!(
                !path.exists() && query(name, "LoadState")? == "not-found",
                "system install refuses an existing {name} unit"
            );
        }
        TcpListener::bind("127.0.0.1:9480")
            .context("system gateway loopback port 9480 is already occupied")?;
        let execution = system_preflight::account(&options.execution_user)?;
        let gateway = system_preflight::account(&options.gateway_user)?;
        ensure!(
            execution.uid != gateway.uid,
            "execution and gateway resolve to the same UID"
        );
        for (name, account) in [
            (&options.execution_user, &execution),
            (&options.gateway_user, &gateway),
        ] {
            let home = fs::symlink_metadata(&account.home)
                .with_context(|| format!("{name} home is unavailable"))?;
            ensure!(
                home.is_dir() && home.uid() == account.uid && !home.file_type().is_symlink(),
                "{name} home must be an owned ordinary directory"
            );
            ensure!(
                !account.home.join(".local/share/voyage/install").exists(),
                "an existing user installation needs separate reviewed adoption"
            );
        }
        service::files::check_path(&options.bin, 0)?;
        ensure!(
            options
                .bin
                .parent()
                .is_some_and(|root| root.join("release.json").is_file()),
            "system source needs a full manifest-verified release, not local build binaries"
        );
        let manifest = Manifest::inspect(&options.bin)?;
        let id = manifest.id()?;
        let unit_plan = system_service::Plan {
            release_root: RELEASE_ROOT.into(),
            bin: format!("{RELEASE_ROOT}/releases/{id}/bin").into(),
            control: CONTROL.into(),
            gateway_state: GATEWAY_STATE.into(),
            execution_user: options.execution_user.clone(),
            execution_uid: execution.uid,
            gateway_user: options.gateway_user.clone(),
            gateway_uid: gateway.uid,
            origin: options.origin.clone(),
            socket: "voyage-system".into(),
            credential_key: options.credential_key.clone(),
            credential_unit: options.credential_unit.clone(),
        };
        let units = unit_plan.render()?;
        let provisioner = Path::new(UNIT_ROOT).join(&options.credential_unit);
        service::files::check_path(&provisioner, 0)?;
        let metadata = fs::symlink_metadata(&provisioner)
            .context("named external credential provisioner unit is missing")?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == 0
                && metadata.nlink() == 1
                && metadata.mode() & 0o022 == 0
                && metadata.len() <= 16384,
            "credential provisioner unit must be an unchanged root-owned regular file"
        );
        ensure!(
            query(&options.credential_unit, "LoadState")? == "loaded"
                && query(&options.credential_unit, "FragmentPath")?
                    == provisioner.display().to_string()
                && query(&options.credential_unit, "DropInPaths")?.is_empty(),
            "credential provisioner must be loaded from its exact root-owned unit without overrides"
        );
        let credential_unit_sha256 = files::hash(&provisioner)?;
        let record = Record {
            schema_version: 1,
            phase: "planned".into(),
            release: id,
            version: manifest.version.clone(),
            execution_user: options.execution_user.clone(),
            execution_uid: execution.uid,
            execution_gid: execution.gid,
            execution_home: execution.home,
            execution_groups: execution.groups,
            gateway_user: options.gateway_user.clone(),
            gateway_uid: gateway.uid,
            gateway_gid: gateway.gid,
            gateway_home: gateway.home,
            gateway_groups: gateway.groups,
            gateway_origin: options.origin.clone(),
            credential_key: options.credential_key.clone(),
            credential_unit: options.credential_unit.clone(),
            credential_unit_sha256,
            root_unit: units.root,
            gateway_unit: units.gateway,
            start_requested: options.start,
        };
        Ok(Self {
            source: options.bin.clone(),
            manifest,
            record,
        })
    }

    fn describe(&self) {
        println!("System scope: fresh root supervisor and separate gateway only");
        println!("Release: {} ({})", self.record.release, self.record.version);
        println!("Source: {}", self.source.display());
        println!(
            "Execution: {} ({})",
            self.record.execution_user, self.record.execution_uid
        );
        println!(
            "Gateway: {} ({})",
            self.record.gateway_user, self.record.gateway_uid
        );
        println!("Origin: {}", self.record.gateway_origin);
        println!(
            "External key: {} via {} (unit SHA-256 {})",
            self.record.credential_key.display(),
            self.record.credential_unit,
            self.record.credential_unit_sha256
        );
        println!(
            "Activation: {}",
            if self.record.start_requested {
                "enable and start"
            } else {
                "leave disabled and inactive"
            }
        );
        println!("Root unit:\n{}", self.record.root_unit);
        println!("Gateway unit:\n{}", self.record.gateway_unit);
        println!(
            "User installations are not converted. Update, rollback and adoption are not available in this increment."
        );
    }
}

fn ctl(args: &[&str]) -> Result<String> {
    let mut all = vec!["--system", "--no-pager"];
    all.extend_from_slice(args);
    Ok(String::from_utf8(service::command::run(
        Path::new("/usr/bin/systemctl"),
        &all,
        None,
    )?)?
    .trim()
    .into())
}

fn query(unit: &str, property: &str) -> Result<String> {
    ctl(&["show", unit, "--value", "--property", property])
}

fn root_host() -> Result<()> {
    ensure!(
        unsafe { libc::getuid() } == 0 && unsafe { libc::geteuid() } == 0,
        "system installation needs a real root invocation"
    );
    ensure!(
        system_preflight::capability_bits()? & ((1 << 6) | (1 << 7)) == (1 << 6) | (1 << 7),
        "root invocation lacks setuid/setgid capabilities"
    );
    for kind in ["uid", "gid"] {
        let value = system_preflight::proc_value(&format!("/proc/self/{kind}_map"), 4096)?;
        ensure!(
            system_preflight::full_identity_map(&value),
            "container or partial user namespace requires separate review"
        );
    }
    Ok(())
}

fn create(path: &Path, mode: u32) -> Result<()> {
    service::files::check_path(path, 0)?;
    DirBuilder::new().mode(mode).create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    ensure!(
        fs::symlink_metadata(path)?.uid() == 0,
        "created system directory is not root-owned"
    );
    Ok(())
}

fn pid(unit: &str, expected_uid: u32, expected_exe: &Path) -> Result<()> {
    ensure!(
        query(unit, "ActiveState")? == "active",
        "{unit} is not active"
    );
    let value: u32 = query(unit, "MainPID")?
        .parse()
        .context("service PID unavailable")?;
    ensure!(value > 1, "{unit} has no live main PID");
    let metadata = fs::metadata(format!("/proc/{value}"))?;
    ensure!(
        metadata.uid() == expected_uid,
        "{unit} is running as the wrong UID"
    );
    ensure!(
        fs::read_link(format!("/proc/{value}/exe"))? == expected_exe,
        "{unit} is running a different binary"
    );
    Ok(())
}

fn preflight() -> Result<()> {
    let address: SocketAddr = "127.0.0.1:9480".parse()?;
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    socket.write_all(b"GET /v1/vessel/pair/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")?;
    let mut data = Vec::new();
    socket.take(65537).read_to_end(&mut data)?;
    ensure!(
        data.len() <= 65536 && data.starts_with(b"HTTP/1.1 200 "),
        "public gateway preflight is not ready"
    );
    let body = data
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .context("gateway response has no body")?
        + 4;
    let value: serde_json::Value = serde_json::from_slice(&data[body..])?;
    ensure!(
        value.get("protocol").and_then(serde_json::Value::as_u64) == Some(1)
            && value
                .get("vessel_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| !id.is_empty()),
        "gateway preflight identity is unavailable"
    );
    Ok(())
}

fn verify_effective(plan: &Plan) -> Result<()> {
    for (name, content) in [
        (ROOT_UNIT, &plan.record.root_unit),
        (GATEWAY_UNIT, &plan.record.gateway_unit),
    ] {
        let path = Path::new(UNIT_ROOT).join(name);
        ensure!(
            fs::read_to_string(&path)? == *content,
            "installed unit contents changed independently"
        );
        ensure!(
            query(name, "LoadState")? == "loaded"
                && query(name, "FragmentPath")? == path.display().to_string()
                && query(name, "DropInPaths")?.is_empty(),
            "systemd effective unit is not the exact installed unit"
        );
    }
    ensure!(
        query(ROOT_UNIT, "KillMode")? == "process"
            && query(GATEWAY_UNIT, "User")? == plan.record.gateway_user,
        "effective unit identity/lifetime differs from the reviewed plan"
    );
    Ok(())
}

fn verify_provisioner(plan: &Plan) -> Result<()> {
    let path = Path::new(UNIT_ROOT).join(&plan.record.credential_unit);
    service::files::check_path(&path, 0)?;
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == 0
            && metadata.nlink() == 1
            && metadata.mode() & 0o022 == 0
            && files::hash(&path)? == plan.record.credential_unit_sha256
            && query(&plan.record.credential_unit, "FragmentPath")? == path.display().to_string()
            && query(&plan.record.credential_unit, "DropInPaths")?.is_empty(),
        "external credential provisioner changed since review"
    );
    Ok(())
}

fn verify_unit_syntax() -> Result<()> {
    service::command::run(
        Path::new("/usr/bin/systemd-analyze"),
        &[
            "verify",
            "/etc/systemd/system/voyage-vessel.service",
            "/etc/systemd/system/voyage-gateway.service",
        ],
        None,
    )?;
    Ok(())
}

fn activate(plan: &Plan) -> Result<()> {
    let release = Path::new(RELEASE_ROOT)
        .join("releases")
        .join(&plan.record.release);
    ctl(&["daemon-reload"])?;
    verify_effective(plan)?;
    verify_unit_syntax()?;
    verify_provisioner(plan)?;
    ctl(&["enable", ROOT_UNIT, GATEWAY_UNIT])?;
    ctl(&["--no-block", "start", ROOT_UNIT, GATEWAY_UNIT])?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if pid(ROOT_UNIT, 0, &release.join("bin/vessel")).is_ok()
            && pid(
                GATEWAY_UNIT,
                plan.record.gateway_uid,
                &release.join("bin/vessel"),
            )
            .is_ok()
            && preflight().is_ok()
        {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "system supervisor/gateway readiness not observed within 30 seconds"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn rollback_units(plan: &Plan) -> Result<()> {
    for name in [GATEWAY_UNIT, ROOT_UNIT] {
        ctl(&["--no-block", "stop", name])?;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if [ROOT_UNIT, GATEWAY_UNIT]
            .iter()
            .all(|name| query(name, "MainPID").ok().as_deref() == Some("0"))
        {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "system service stop remains unobserved; release and units retained"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    ctl(&["disable", ROOT_UNIT, GATEWAY_UNIT])?;
    for (name, content) in [
        (GATEWAY_UNIT, &plan.record.gateway_unit),
        (ROOT_UNIT, &plan.record.root_unit),
    ] {
        let path = Path::new(UNIT_ROOT).join(name);
        service::files::remove_reviewed(&path, content)?;
    }
    ctl(&["daemon-reload"])?;
    Ok(())
}

fn apply(mut plan: Plan, options: &Options) -> Result<()> {
    root_host()?;
    let renewed = Plan::prepare(options)?;
    ensure!(
        serde_json::to_vec(&plan.record)? == serde_json::to_vec(&renewed.record)?,
        "system installation facts changed since review; review again"
    );
    let _lock = {
        ensure!(
            !Path::new(CONTROL_PARENT).exists(),
            "system installation appeared after review"
        );
        create(Path::new(CONTROL_PARENT), 0o700)?;
        create(&Path::new(CONTROL_PARENT).join("install"), 0o700)?;
        files::lock(&Path::new(CONTROL_PARENT).join("install/lock"))?
    };
    let transaction = Path::new(CONTROL_PARENT).join("install/transaction.json");
    plan.record.phase = "preparing-directories".into();
    files::atomic_json(&transaction, &plan.record)?;
    create(Path::new(RELEASE_ROOT), 0o755)?;
    create(&Path::new(RELEASE_ROOT).join("releases"), 0o755)?;
    create(Path::new(CONTROL), 0o700)?;
    create(Path::new(RUNTIME), 0o711)?;
    create(Path::new(CONFIG_ROOT), 0o700)?;
    let release = Path::new(RELEASE_ROOT)
        .join("releases")
        .join(&plan.record.release);
    plan.record.phase = "staging".into();
    files::atomic_json(&transaction, &plan.record)?;
    plan.manifest.stage_system(&plan.source, &release)?;
    let layout = serde_json::json!({"version":1,"runtime_root":RUNTIME});
    files::write_new(
        &Path::new(CONTROL).join("runtime-layout.json"),
        &serde_json::to_vec(&layout)?,
    )?;
    plan.record.phase = "publishing-units".into();
    files::atomic_json(&transaction, &plan.record)?;
    let root_path = Path::new(UNIT_ROOT).join(ROOT_UNIT);
    let gateway_path = Path::new(UNIT_ROOT).join(GATEWAY_UNIT);
    service::files::replace(&root_path, &plan.record.root_unit, None)?;
    if let Err(error) = service::files::replace(&gateway_path, &plan.record.gateway_unit, None) {
        let rollback = service::files::remove_reviewed(&root_path, &plan.record.root_unit);
        plan.record.phase = "unresolved-unit-publication".into();
        files::atomic_json(&transaction, &plan.record)?;
        return Err(error.context(format!(
            "gateway unit publication failed; root-unit cleanup observed={}",
            rollback.is_ok()
        )));
    }
    plan.record.phase = "activating".into();
    files::atomic_json(&transaction, &plan.record)?;
    let applied = if plan.record.start_requested {
        activate(&plan)
    } else {
        (|| {
            ctl(&["daemon-reload"])?;
            verify_effective(&plan)?;
            verify_unit_syntax()?;
            verify_provisioner(&plan)?;
            for name in [ROOT_UNIT, GATEWAY_UNIT] {
                ensure!(
                    query(name, "MainPID")? == "0" && query(name, "UnitFileState")? == "disabled",
                    "inactive system installation has unexpected activation state"
                );
            }
            Ok(())
        })()
    };
    if let Err(error) = applied {
        let rollback = rollback_units(&plan);
        plan.record.phase = if rollback.is_ok() {
            "inactive-failed"
        } else {
            "unresolved-activation"
        }
        .into();
        files::atomic_json(&transaction, &plan.record)?;
        return Err(error.context(format!("system activation failed; unit rollback observed={}; staged release and transaction retained", rollback.is_ok())));
    }
    let config = Path::new(CONFIG_ROOT).join("system-install.json");
    plan.record.phase = if plan.record.start_requested {
        "active"
    } else {
        "inactive"
    }
    .into();
    files::write_new(&config, &serde_json::to_vec_pretty(&plan.record)?)?;
    files::atomic_json(&transaction, &plan.record)?;
    println!(
        "System installation staged: {} ({})",
        plan.record.release, plan.record.version
    );
    println!("Supervisor/gateway: {}", plan.record.phase);
    println!(
        "Update, rollback and adoption remain unavailable; do not use this fresh-install increment as a supported production path."
    );
    Ok(())
}

pub(super) fn run(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) == Some("status") {
        ensure!(
            args == ["status", "--scope", "system"],
            "system status takes only --scope system"
        );
        return status();
    }
    let options = Options::parse(args)?;
    let plan = Plan::prepare(&options)?;
    plan.describe();
    if options.dry_run {
        println!("Dry run complete; no installation changes applied.");
        return Ok(());
    }
    apply(plan, &options)
}

fn status() -> Result<()> {
    root_host()?;
    let transaction = Path::new(CONTROL_PARENT).join("install/transaction.json");
    if !Path::new(CONTROL_PARENT).exists() {
        println!("System installation: absent");
        return Ok(());
    }
    service::files::check_path(&transaction, 0)?;
    let record: Record = serde_json::from_slice(&files::read(&transaction, 65536)?)?;
    ensure!(
        record.schema_version == 1
            && record.release.len() == 64
            && record.release.bytes().all(|b| b.is_ascii_hexdigit()),
        "system installation transaction identity is invalid"
    );
    println!("System transaction: {}", record.phase);
    println!("Release: {} ({})", record.release, record.version);
    println!(
        "Execution UID: {}; gateway UID: {}",
        record.execution_uid, record.gateway_uid
    );
    if !matches!(record.phase.as_str(), "active" | "inactive") {
        println!(
            "Unresolved system installation; inspect the exact units, state and service PIDs before another mutation."
        );
        return Ok(());
    }
    let config: Record = serde_json::from_slice(&files::read(
        &Path::new(CONFIG_ROOT).join("system-install.json"),
        65536,
    )?)?;
    ensure!(
        serde_json::to_vec(&record)? == serde_json::to_vec(&config)?,
        "system configuration and transaction differ"
    );
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
    for (name, expected) in [
        (ROOT_UNIT, record.root_unit.as_str()),
        (GATEWAY_UNIT, record.gateway_unit.as_str()),
    ] {
        ensure!(
            fs::read_to_string(Path::new(UNIT_ROOT).join(name))? == expected,
            "system unit changed: {name}"
        );
        println!("{name}: {}", query(name, "ActiveState")?);
    }
    if record.phase == "active" {
        pid(ROOT_UNIT, 0, &release.join("bin/vessel"))?;
        pid(
            GATEWAY_UNIT,
            record.gateway_uid,
            &release.join("bin/vessel"),
        )?;
        preflight()?;
        println!("Readiness: exact release and separate root/gateway processes observed");
    } else {
        ensure!(
            query(ROOT_UNIT, "MainPID")? == "0" && query(GATEWAY_UNIT, "MainPID")? == "0",
            "inactive installation has a live supervisor or gateway"
        );
        println!("Readiness: intentionally inactive");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options> {
        Options::parse(&args.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>())
    }
    #[test]
    fn requires_explicit_system_and_bound_identity_plan() {
        let base = [
            "install",
            "--scope",
            "system",
            "--bin-dir",
            "/root/release/bin",
            "--execution-user",
            "ordinary",
            "--gateway-user",
            "gateway",
            "--gateway-origin",
            "https://helm.example.test",
            "--credential-key",
            "/run/voyage-secrets/key",
            "--credential-unit",
            "voyage-key-provision.service",
        ];
        assert!(parse(&base).is_ok());
        assert!(parse(&base[..base.len() - 2]).is_err());
        assert!(parse(&["install", "--scope", "system", "--execution-user", "root"]).is_err());
        let mut same = base.to_vec();
        same[8] = "ordinary";
        assert!(parse(&same).is_err());
        let mut relative = base.to_vec();
        relative[4] = "relative/bin";
        assert!(parse(&relative).is_err());
        for suffix in [
            vec!["--start", "--no-start"],
            vec!["--no-start", "--start"],
            vec!["--start", "--start"],
            vec!["--dry-run", "--dry-run"],
            vec!["--scope", "system"],
        ] {
            let mut arguments = base.to_vec();
            arguments.extend(suffix);
            assert!(parse(&arguments).is_err());
        }
        let mut review = base.to_vec();
        review.extend(["--no-start", "--dry-run"]);
        let options = parse(&review).unwrap();
        assert!(!options.start && options.dry_run);
    }

    #[test]
    fn ordinary_installation_cannot_become_system_authority() {
        if unsafe { libc::geteuid() } != 0 {
            assert!(root_host().unwrap_err().to_string().contains("real root"));
        }
    }
}
