use anyhow::{Context, Result, bail, ensure};
use std::path::PathBuf;

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
                "--bin-dir" => {
                    options.bin_dir =
                        PathBuf::from(args.next().context("--bin-dir requires a directory")?)
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
        if options.action == Some(Action::Rollback) {
            ensure!(
                !seen.contains("--bin-dir") && !options.replace_existing,
                "rollback uses the recorded previous release, without --bin-dir or --replace-existing"
            );
        }
        Ok(options)
    }
}
pub fn help() {
    println!(
        "voyage-installer — install a complete local release\n\nNo action: interactive Linux review/apply/cancel wizard.\nNoninteractive actions: install, upgrade, rollback\n  voyage-installer status  (read-only installed versions and integrity)\n  voyage-installer install [--bin-dir DIRECTORY] [--replace-existing] [--start | --no-start] [--dry-run]\n  voyage-installer upgrade [same options]\n  voyage-installer rollback [--start | --no-start] [--dry-run]\n\nThe release defaults to the installer executable's sibling binaries.\nReview identifies release versions, paths and retained rollback data.\n--replace-existing explicitly permits backing up unmanaged PATH binaries.\n--start enables/starts the Linux Vessel user service. Default --no-start leaves an inactive service stopped; an already active service restarts for upgrades.\n--dry-run validates and prints the plan without changing installation or services.\nProvider configuration and session data are preserved. No provider login or network setup is performed.\n\nExisting service commands: install-user-service --bin-dir ABS [--start] [--dry-run],\nservice-status, service-stop, service-uninstall\n\n--help, --version"
    );
}
