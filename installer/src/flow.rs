//! Reviewable plans shared by interactive and explicit command-line installation.
use crate::{
    cli::{Action, Options},
    install,
};
use anyhow::{Context, Result};

pub fn plan(options: &Options) -> Result<install::Report> {
    execute(options, true)
}
pub fn execute(options: &Options, dry_run: bool) -> Result<install::Report> {
    match options
        .action
        .context("an installation action is required")?
    {
        Action::Rollback => install::rollback(dry_run),
        Action::Install | Action::Upgrade => install::run(install::Options {
            bin_dir: options.bin_dir.clone(),
            replace_existing: options.replace_existing,
            dry_run,
        }),
    }
}
pub fn describe(report: &install::Report, options: &Options) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{} release: {}",
            options.action.map_or("Selected", Action::label),
            report.release
        ),
        format!("Binary directory: {}", report.bin_dir.display()),
    ];
    lines.push(format!(
        "Current release: {}",
        report.current_release.as_deref().unwrap_or("none")
    ));
    lines.extend(report.actions.iter().cloned());
    lines
        .push("Existing provider configuration, credentials and voyage data are preserved.".into());
    lines.push(if options.start {
        "After installation: enable/start the Vessel user service. Existing voyages retain their independent processes.".into()
    } else {
        "Install/update the user-service unit. Restart an already active service; leave an inactive service stopped.".into()
    });
    lines
}
