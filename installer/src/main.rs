mod cli;
mod flow;
mod install;
mod planning;
mod service;
mod source;
mod ui;

use anyhow::{Context, Result};

fn run() -> Result<bool> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") if args.len() == 1 => {
            cli::help();
            return Ok(false);
        }
        Some("status") if args.len() == 1 => {
            for line in install::status()? {
                println!("{line}");
            }
            return Ok(false);
        }
        Some("--version") if args.len() == 1 => {
            println!("voyage-installer {}", env!("CARGO_PKG_VERSION"));
            return Ok(false);
        }
        Some(command @ ("service-status" | "service-stop" | "service-uninstall")) => {
            let _operation = if command == "service-status" {
                None
            } else {
                Some(install::operation_lock()?)
            };
            service::manage(command, &args[1..])?;
            return Ok(false);
        }
        _ => {}
    }
    if args
        .first()
        .is_some_and(|arg| arg == "install-user-service")
    {
        args[0] = "install".into();
    }
    let options = cli::Options::parse(&args)?;
    let (mut options, reviewed) = if options.action.is_none() {
        let Some((options, reviewed)) = ui::review(options)? else {
            println!("Installation cancelled.");
            return Ok(true);
        };
        (options, Some(reviewed))
    } else {
        (options, None)
    };
    let cancellation = source::Cancellation::new()?;
    if reviewed.is_none() {
        println!("Source: {}", options.source_label());
        println!(
            "Preparing source for review; downloads/builds may take several minutes. Ctrl+C cancels."
        );
        options.prepare(&cancellation.flag)?;
    }
    if cancellation.flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(true);
    }
    // Serialize the entire publication/service/rollback operation, after TTY restoration.
    let _operation = if options.dry_run {
        None
    } else {
        Some(install::operation_lock()?)
    };
    let plan = flow::plan(&options)?;
    let mut description = flow::describe(&plan, &options);
    description.push(service::preview(
        &plan.release_dir.join("bin"),
        options.start,
    )?);
    if let Some(reviewed) = reviewed {
        anyhow::ensure!(
            reviewed == description,
            "installation state changed since review; review again"
        );
    }
    for line in description {
        println!("{line}");
    }
    if options.dry_run {
        println!("Dry run complete; no installation changes applied.");
        return Ok(false);
    }
    if cancellation.flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(true);
    }
    if !plan.changed && options.action == Some(cli::Action::Upgrade) {
        println!(
            "Already current: {}. No binary release change is needed; checking service configuration.",
            plan.release
        );
    }
    let report = flow::execute(&options, false)?;
    if let Err(error) = service::configure(&report.release_dir.join("bin"), options.start, false) {
        if report.changed && report.current_release.is_some() {
            install::rollback(false)
                .context("service configuration failed and binary rollback also failed")?;
            return Err(
                error.context("service configuration failed; previous binary release restored")
            );
        }
        return Err(error.context("binaries installed; service configuration failed"));
    }
    println!(
        "{} complete: {}",
        options.action.context("action missing")?.label(),
        report.release
    );
    println!("Executables: {}", report.bin_dir.display());
    println!("Close and relaunch Helm to use the installed build. Existing voyages keep running.");
    Ok(false)
}
fn main() {
    let outcome = run();
    if let Err(error) = source::cleanup_result() {
        eprintln!("Installer: {error:#}");
        std::process::exit(1);
    }
    match outcome {
        Ok(true) => std::process::exit(130),
        Ok(false) => {}
        Err(error) => {
            eprintln!("Installer: {error:#}");
            std::process::exit(1);
        }
    }
}
