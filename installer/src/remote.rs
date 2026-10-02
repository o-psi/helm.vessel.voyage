//! Durable, owner-initiated remote updates. The client can choose a channel and
//! approve a prepared identity, never executable paths, commands or download URLs.
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod system;

pub fn run(args: &[String]) -> anyhow::Result<()> {
    if args == ["protocol"] {
        println!("1");
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        if args.first().is_some_and(|s| s == "system") {
            return system::run(&args[1..]);
        }
        linux::run(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("Remote updates require a managed Linux installation")
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn local_legacy_review(
    options: &crate::cli::Options,
    report: &crate::install::Report,
) -> anyhow::Result<Vec<String>> {
    linux::local_legacy_review(options, report)
}
#[cfg(target_os = "linux")]
pub(crate) fn local_legacy_bootstrap(
    options: &crate::cli::Options,
    report: &crate::install::Report,
) -> anyhow::Result<bool> {
    linux::local_legacy_bootstrap(options, report)
}
