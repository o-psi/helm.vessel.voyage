//! Durable, owner-initiated remote updates. The client can choose a channel and
//! approve a prepared identity, never executable paths, commands or download URLs.
#[cfg(target_os = "linux")]
mod linux;

pub fn run(args: &[String]) -> anyhow::Result<()> {
    if args == ["protocol"] {
        println!("1");
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        linux::run(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("Remote updates require a managed Linux installation")
    }
}
