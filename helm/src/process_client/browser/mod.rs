//! Human-authorized local browser resource. Voyage remains the sole agent runtime.
mod assets;
mod helper;
mod journal;
pub(crate) use journal::reconcile;
mod runner;
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(clap::Args)]
pub struct BrowserArgs {
    #[command(subcommand)]
    command: BrowserCommand,
}
#[derive(clap::Subcommand)]
enum BrowserCommand {
    /// Explicitly install the bundled lockfile-pinned Node adapter dependency. No browser is shared.
    Setup,
    /// Inspect local requirements without opening a browser or contacting a provider.
    Status,
}
pub async fn run_cli(args: &BrowserArgs) -> Result<()> {
    match args.command {
        BrowserCommand::Setup => assets::setup().await,
        BrowserCommand::Status => {
            let path = assets::distribution()?;
            println!("Adapter installed: {}", assets::installed(&path));
            println!("Chromium available: {}", assets::executable().is_ok());
            println!(
                "Use Helm F6 or /browser on a selected voyage. Sharing and private return are explicit local companion operations."
            );
            Ok(())
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub summary: String,
    /// Local file launcher only, never a credential-bearing URL.
    pub launcher: Option<PathBuf>,
    pub finished: bool,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            summary: "Starting local browser; not shared".into(),
            launcher: None,
            finished: false,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Control {
    Human,
    Private,
}
pub(crate) struct Handle {
    pub state: watch::Receiver<Status>,
    control: mpsc::Sender<Control>,
    stop: CancellationToken,
    job: Option<tokio::task::JoinHandle<Result<()>>>,
}
impl Handle {
    pub fn start(client: Client, session: Uuid, incarnation: Uuid, label: String) -> Self {
        let (state, status) = watch::channel(Status::default());
        let (control, input) = mpsc::channel(8);
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let job = tokio::spawn(async move {
            let result = runner::run(
                client,
                session,
                incarnation,
                label,
                input,
                cancelled,
                &state,
            )
            .await;
            state.send_modify(|s| {
                s.finished = true;
                if result.is_err() && s.launcher.is_none() && s.summary != Status::default().summary { return; }
                s.summary = if result.is_ok() { "Local browser closed; Voyage was not cancelled".into() }
                    else { "Local browser unavailable; sharing stopped. Cleanup or action outcome may require reconciliation".into() };
            });
            result
        });
        Self {
            state: status,
            control,
            stop,
            job: Some(job),
        }
    }
    pub fn control(&self, value: Control) -> Result<()> {
        self.control
            .try_send(value)
            .context("Local browser control queue unavailable")
    }
    pub fn stop(&self) {
        self.stop.cancel();
    }
    pub fn finished(&self) -> bool {
        self.job.as_ref().is_none_or(|job| job.is_finished())
    }
    pub async fn finish(&mut self) -> Result<()> {
        self.stop.cancel();
        if let Some(job) = self.job.take() {
            job.await.context("Local browser cleanup task failed")??;
        }
        Ok(())
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // The owning task observes cleanup; dropping the UI is not permission to abandon the helper.
        self.stop.cancel();
    }
}

pub(super) fn launcher(root: &std::path::Path, url: &str) -> Result<PathBuf> {
    let url = reqwest::Url::parse(url).context("Invalid local companion address")?;
    ensure!(
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.path() == "/"
            && url.fragment().is_some(),
        "Invalid local companion address"
    );
    let escaped = url
        .as_str()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let text = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"refresh\" content=\"0;url={escaped}\"><title>Local Helm browser</title><a href=\"{escaped}\">Open local browser companion</a>"
    );
    let private = crate::attachment::local_actor::storage::Directory::open_existing(root)?;
    private.publish_new("open.html", text.as_bytes())?;
    Ok(root.join("open.html"))
}
pub(crate) async fn open_launcher(path: PathBuf) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "Local browser launcher must be absolute"
    );
    #[cfg(target_os = "linux")]
    {
        let mut child = tokio::process::Command::new("xdg-open")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Could not open companion; open its private local launcher file manually")?;
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(result) => ensure!(
                result?.success(),
                "Could not open companion; open its private local launcher file manually"
            ),
            Err(_) => {
                let _ = child.start_kill();
                child.wait().await?;
                anyhow::bail!("Companion opener did not finish; local browser remains unshared");
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        anyhow::bail!(
            "Local browser desktop integration currently requires Linux; native support is not verified"
        )
    }
}

/// Optional standalone human frontend; it shares the same Client socket and consent gates as F6.
pub async fn run_connected(client: Client, session: Uuid) -> Result<()> {
    use voyage_protocol::vessel::{ProcessInfo, VesselCommand};
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect {
                session_id: session,
            })
            .await?,
    )?;
    let mut handle = Handle::start(
        client.clone(),
        session,
        process.incarnation,
        format!("Voyage {session} — {}", client.label()),
    );
    let mut opened = false;
    println!(
        "Local browser companion. Sharing is off until you explicitly enable it locally. Ctrl+C closes this browser, not the Voyage."
    );
    let result: Result<()> = async {
        loop {
            let state = handle.state.borrow_and_update().clone();
            println!("{}", state.summary);
            if !opened && let Some(path) = state.launcher {
                println!("Local launcher: {}", path.display());
                let _ = open_launcher(path).await;
                opened = true;
            }
            if state.finished {
                break;
            }
            tokio::select! {
                _=tokio::signal::ctrl_c()=>break,
                changed=handle.state.changed()=>if changed.is_err(){break;},
            }
        }
        Ok(())
    }
    .await;
    result.and(handle.finish().await)
}
