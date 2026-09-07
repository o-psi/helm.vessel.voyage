use anyhow::{Context, Result, bail, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade,
    Rollback,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Upgrade => "Upgrade",
            Self::Rollback => "Rollback",
        }
    }
}
#[derive(Clone)]
pub struct Options {
    pub action: Option<Action>,
    pub bin_dir: PathBuf,
    pub replace_existing: bool,
    pub dry_run: bool,
    pub start: bool,
    pub local_source: bool,
    pub dev: bool,
    pub prepared: Option<Arc<crate::source::Prepared>>,
}
impl Options {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self {
            action: None,
            bin_dir: std::env::current_exe()?
                .parent()
                .context("installer directory missing")?
                .to_path_buf(),
            replace_existing: false,
            dry_run: false,
            start: false,
            local_source: false,
            dev: false,
            prepared: None,
        };
        let mut seen = std::collections::HashSet::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            ensure!(seen.insert(arg.clone()), "repeated argument: {arg}");
            match arg.as_str() {
                "install" | "upgrade" | "rollback" => {
                    ensure!(options.action.is_none(), "choose one action");
                    options.action = Some(match arg.as_str() {
                        "install" => Action::Install,
                        "upgrade" => Action::Upgrade,
                        _ => Action::Rollback,
                    });
                }
                "--dev" => options.dev = true,
                "--bin-dir" => {
                    options.local_source = true;
                    options.bin_dir =
                        PathBuf::from(args.next().context("--bin-dir requires a directory")?);
                }
                "--replace-existing" => options.replace_existing = true,
                "--dry-run" => options.dry_run = true,
                "--start" => {
                    ensure!(
                        !seen.contains("--no-start"),
                        "--start conflicts with --no-start"
                    );
                    options.start = true;
                }
                "--no-start" => {
                    ensure!(
                        !seen.contains("--start"),
                        "--start conflicts with --no-start"
                    );
                    options.start = false;
                }
                _ => bail!("unknown argument: {arg}; use --help"),
            }
        }
        options.validate()?;
        Ok(options)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !(self.dev && self.local_source),
            "--dev conflicts with --bin-dir"
        );
        ensure!(
            !self.dev || matches!(self.action, None | Some(Action::Upgrade)),
            "--dev is only supported for upgrade"
        );
        if self.action == Some(Action::Rollback) {
            ensure!(
                !self.local_source && !self.replace_existing && !self.dev,
                "rollback uses the recorded previous release, without --bin-dir, --dev or --replace-existing"
            );
        }
        Ok(())
    }
    pub fn source_label(&self) -> String {
        if let Some(prepared) = &self.prepared {
            prepared.description.clone()
        } else if self.action == Some(Action::Rollback) {
            "Recorded previous release".into()
        } else if self.local_source || self.action == Some(Action::Install) {
            format!("Local binaries: {}", self.bin_dir.display())
        } else if self.dev {
            "GitHub main: pin and build a development commit (executes trusted build code)".into()
        } else {
            "Latest published GitHub release (no development fallback)".into()
        }
    }
    pub fn prepare(&mut self, cancelled: &AtomicBool) -> Result<()> {
        self.validate()?;
        if self.action == Some(Action::Upgrade) && !self.local_source && self.prepared.is_none() {
            let source = if self.dev {
                crate::source::Source::Main
            } else {
                crate::source::Source::Latest
            };
            let prepared = Arc::new(crate::source::prepare(source, cancelled)?);
            self.bin_dir = prepared.bin_dir.clone();
            self.prepared = Some(prepared);
        }
        Ok(())
    }
}
pub fn help() {
    println!("voyage-installer — install and update Voyage

No action: interactive Linux review/apply/cancel wizard.
  voyage-installer install [--bin-dir DIRECTORY] [--replace-existing] [--start | --no-start] [--dry-run]
  voyage-installer upgrade [--dev | --bin-dir DIRECTORY] [--replace-existing] [--start | --no-start] [--dry-run]
  voyage-installer rollback [--start | --no-start] [--dry-run]
  voyage-installer status

upgrade defaults to the latest published GitHub release, downloaded and verified.
--dev explicitly fetches GitHub main and builds its pinned commit with stable Rust.
--bin-dir explicitly uses local binaries instead; it conflicts with --dev.
install defaults to the installer executable's sibling binaries.
Automatic upgrades require Linux, Python 3.11+ and curl; --dev also needs Git,
stable Rust/Cargo, native build tools and network access to build dependencies.
Private sources use your existing GitHub CLI login (gh auth login) on this machine.
Development builds execute code from main as your account, not inside a sandbox.
--dry-run may download/build into private temporary staging to review the exact
release; it does not change installation or services. Cancellation removes staging
after subprocess cleanup; forced termination can leave staging for inspection.
--replace-existing backs up unmanaged PATH binaries before replacing them.
--start enables/starts the Linux Vessel user service. --no-start leaves an inactive
service stopped; an already active service restarts for upgrades.
Configuration, credentials, sessions and prior releases are preserved.
Close and relaunch Helm after upgrading; existing voyages remain independent.
No provider login is performed. Missing releases never silently fall back to main.

Service commands: install-user-service --bin-dir ABS [--start] [--dry-run],
service-status, service-stop, service-uninstall
--help, --version");
}
