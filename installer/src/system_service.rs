//! Staged system-unit contract. Publication and activation remain unavailable
//! until protected system layout and scope-aware rollback are integrated.
#![allow(dead_code)]

use anyhow::{Context, Result, ensure};
use std::path::{Component, Path, PathBuf};

pub(super) const ROOT_UNIT: &str = "voyage-vessel.service";
pub(super) const GATEWAY_UNIT: &str = "voyage-gateway.service";

pub(super) struct Plan {
    pub(super) release_root: PathBuf,
    pub(super) bin: PathBuf,
    pub(super) control: PathBuf,
    pub(super) gateway_state: PathBuf,
    pub(super) execution_user: String,
    pub(super) execution_uid: u32,
    pub(super) gateway_user: String,
    pub(super) gateway_uid: u32,
    pub(super) origin: String,
    pub(super) socket: String,
    pub(super) credential_key: PathBuf,
    pub(super) credential_unit: String,
}

pub(super) struct Units {
    pub(super) root: String,
    pub(super) gateway: String,
}

fn account_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}

fn quote_path(path: &Path) -> Result<String> {
    ensure!(
        absolute(path),
        "system unit path must be absolute and normalized"
    );
    let value = path.to_str().context("system unit path requires UTF-8")?;
    ensure!(
        !value.chars().any(char::is_control),
        "system unit path contains a control character"
    );
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}

fn directive_path(path: &Path) -> Result<String> {
    ensure!(
        absolute(path),
        "system unit path must be absolute and normalized"
    );
    let value = path.to_str().context("system unit path requires UTF-8")?;
    ensure!(
        value
            .bytes()
            .all(|byte| { byte.is_ascii_alphanumeric() || b"/_-.".contains(&byte) }),
        "system unit directory path contains unsupported characters"
    );
    Ok(value.into())
}

fn origin(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.origin().ascii_serialization() == value
}

fn credential_unit(value: &str) -> bool {
    (9..=128).contains(&value.len())
        && value.ends_with(".service")
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.@-".contains(&byte))
        && value != ROOT_UNIT
        && value != GATEWAY_UNIT
}

impl Plan {
    pub(super) fn render(&self) -> Result<Units> {
        ensure!(
            account_name(&self.execution_user)
                && account_name(&self.gateway_user)
                && self.execution_uid != 0
                && self.gateway_uid != 0
                && self.execution_uid != self.gateway_uid
                && self.execution_user != self.gateway_user,
            "system execution and gateway require distinct ordinary accounts"
        );
        ensure!(
            origin(&self.origin),
            "system gateway requires one HTTPS origin"
        );
        ensure!(
            credential_unit(&self.credential_unit),
            "system credential provisioner must be a separate service unit"
        );
        ensure!(
            (1..=80).contains(&self.socket.len())
                && self.socket.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                }),
            "invalid system gateway socket identity"
        );
        ensure!(
            absolute(&self.release_root)
                && absolute(&self.bin)
                && absolute(&self.control)
                && absolute(&self.gateway_state)
                && self.credential_key.starts_with("/run/")
                && self.credential_key != Path::new("/run")
                && !self.control.starts_with(&self.gateway_state)
                && !self.gateway_state.starts_with(&self.control),
            "invalid system installation paths"
        );
        let release = self
            .bin
            .strip_prefix(self.release_root.join("releases"))
            .context("system unit must use a versioned release")?;
        let mut parts = release.components();
        let id = parts.next().context("system release identity missing")?;
        let id = id
            .as_os_str()
            .to_str()
            .context("system release ID is not UTF-8")?;
        ensure!(
            id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid system release identity"
        );
        ensure!(
            parts.next() == Some(Component::Normal("bin".as_ref())) && parts.next().is_none(),
            "system unit must pin the versioned binary directory"
        );
        let vessel = quote_path(&self.bin.join("vessel"))?;
        let voyage = quote_path(&self.bin.join("voyage"))?;
        let control = quote_path(&self.control)?;
        let database = quote_path(&self.gateway_state.join("vessel.db"))?;
        let control_working_directory = directive_path(&self.control)?;
        let gateway_working_directory = directive_path(&self.gateway_state)?;
        let credential_key = directive_path(&self.credential_key)?;
        let state_directory = self
            .gateway_state
            .strip_prefix("/var/lib")
            .context("gateway state must reside directly under /var/lib")?;
        ensure!(
            state_directory.components().count() == 1,
            "gateway state must be one /var/lib directory"
        );
        let state_directory = state_directory
            .to_str()
            .context("gateway state directory requires UTF-8")?;
        let root = format!(
            "[Unit]\nDescription=Voyage privileged Vessel supervisor\nRequires={}\nAfter=network.target {}\n\n[Service]\nType=simple\nEnvironment=VOYAGE_CREDENTIAL_KEY_FILE={credential_key}\nExecStart=:{vessel} local-serve --directory {control} --voyage-binary {voyage} --gateway-socket {} --gateway-uid {} --gateway-origin {}\nWorkingDirectory={control_working_directory}\nUMask=0077\nRuntimeDirectory=voyage\nRuntimeDirectoryMode=0711\nRuntimeDirectoryPreserve=yes\nRestart=on-failure\nRestartSec=2\nKillMode=process\nTimeoutStopSec=30\nStandardInput=null\nStandardOutput=journal\nStandardError=journal\n\n[Install]\nWantedBy=multi-user.target\n",
            self.credential_unit, self.credential_unit, self.socket, self.gateway_uid, self.origin
        );
        let gateway = format!(
            "[Unit]\nDescription=Voyage unprivileged public gateway\nWants={ROOT_UNIT}\nAfter={ROOT_UNIT}\n\n[Service]\nType=simple\nUser={}\nExecStart=:{vessel} --bind 127.0.0.1:9480 --database {database} --system-gateway-socket {} {}\nWorkingDirectory={gateway_working_directory}\nStateDirectory={state_directory}\nStateDirectoryMode=0700\nUMask=0077\nNoNewPrivileges=yes\nCapabilityBoundingSet=\nAmbientCapabilities=\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nRestart=on-failure\nRestartSec=2\nStandardInput=null\nStandardOutput=journal\nStandardError=journal\n\n[Install]\nWantedBy=multi-user.target\n",
            self.gateway_user, self.socket, self.origin
        );
        Ok(Units { root, gateway })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};

    fn plan() -> Plan {
        Plan {
            release_root: "/opt/voyage".into(),
            bin: format!("/opt/voyage/releases/{}/bin", "a".repeat(64)).into(),
            control: "/var/lib/voyage/vessel".into(),
            gateway_state: "/var/lib/voyage-gateway".into(),
            execution_user: "voyageordinary".into(),
            execution_uid: 1000,
            gateway_user: "voyagegateway".into(),
            gateway_uid: 1001,
            origin: "https://helm.example.test".into(),
            socket: "voyage-system-test".into(),
            credential_key: "/run/voyage-secrets/connections.key".into(),
            credential_unit: "voyage-key-provision.service".into(),
        }
    }

    #[test]
    fn two_units_pin_distinct_authority_and_independent_voyage_lifetime() {
        let units = plan().render().unwrap();
        assert!(units.root.contains("--gateway-uid 1001"));
        assert!(
            units
                .root
                .contains("--voyage-binary \"/opt/voyage/releases/")
        );
        assert!(units.root.contains("RuntimeDirectoryPreserve=yes"));
        assert!(units.root.contains("KillMode=process"));
        assert!(units.root.contains(
            "Environment=VOYAGE_CREDENTIAL_KEY_FILE=/run/voyage-secrets/connections.key"
        ));
        assert!(
            units
                .root
                .contains("Requires=voyage-key-provision.service\n")
        );
        assert!(!units.root.contains("User="));
        assert!(units.gateway.contains("User=voyagegateway\n"));
        assert!(units.gateway.contains("NoNewPrivileges=yes"));
        assert!(
            units
                .gateway
                .contains("--system-gateway-socket voyage-system-test")
        );
        assert!(!units.gateway.contains("--directory /var/lib/voyage/vessel"));
        assert!(!units.gateway.contains("KillMode=process"));
    }

    #[test]
    fn unit_plan_refuses_scope_confusion_and_injected_values() {
        let mut p = plan();
        p.gateway_uid = p.execution_uid;
        assert!(p.render().is_err());
        let mut p = plan();
        p.gateway_user = "root".into();
        p.gateway_uid = 0;
        assert!(p.render().is_err());
        let mut p = plan();
        p.bin = "/home/user/bin".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.bin = "/opt/voyage/current/bin".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.origin = "http://helm.example.test".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.origin = "https://user@helm.example.test".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.origin = "https://helm.example.test/path".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.origin = "https://helm.example.test:abc".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.socket = "voyage; ExecStart=/bin/sh".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.credential_key = "/etc/voyage/connections.key".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.credential_key = "/run/secret%N".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.credential_unit = "../other.service".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.credential_unit = ROOT_UNIT.into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.gateway_state = p.control.join("gateway");
        assert!(p.render().is_err());
        let mut p = plan();
        p.control = "/var/lib/voyage/space here".into();
        assert!(p.render().is_err());
        let mut p = plan();
        p.gateway_state = "/tmp/voyage-gateway".into();
        assert!(p.render().is_err());
    }

    #[test]
    #[ignore = "requires an explicitly disposable native-root Linux systemd fixture"]
    fn native_systemd_accepts_pinned_root_and_ordinary_gateway_units() {
        assert_eq!(
            std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
            Ok("1")
        );
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let base = PathBuf::from(format!(
            "/opt/voyage-unit-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(base.clone());
        let mut p = plan();
        p.release_root = base.join("releases-root");
        p.bin = p
            .release_root
            .join("releases")
            .join("a".repeat(64))
            .join("bin");
        p.control = base.join("control");
        p.gateway_state = "/var/lib/voyage-gateway".into();
        p.gateway_user = "voyageother".into();
        fs::create_dir_all(&p.bin).unwrap();
        fs::create_dir_all(&p.control).unwrap();
        for name in ["vessel", "voyage"] {
            let path = p.bin.join(name);
            fs::copy("/usr/bin/true", &path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let units = p.render().unwrap();
        let root = base.join(ROOT_UNIT);
        let gateway = base.join(GATEWAY_UNIT);
        let credential = base.join(&p.credential_unit);
        fs::write(&root, units.root).unwrap();
        fs::write(&gateway, units.gateway).unwrap();
        fs::write(
            &credential,
            "[Unit]\nDescription=Fixture credential provisioner\n\n[Service]\nType=oneshot\nExecStart=/usr/bin/true\nRemainAfterExit=yes\n",
        )
        .unwrap();
        let output = Command::new("/usr/bin/systemd-analyze")
            .arg("verify")
            .arg(&root)
            .arg(&gateway)
            .arg(&credential)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "systemd refused unit plan: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        drop(fixture);
    }
}
