//! Enrollment cancellation belongs to the interface, independently of voyages.
use anyhow::Result;
pub(crate) async fn attachment_notice(message: &'static str) {
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        tokio::task::spawn_blocking(move || eprintln!("{message}")),
    )
    .await;
}

pub(crate) async fn attachment_connect(args: helm::attachment::cli::AttachmentArgs) -> Result<()> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let operation = helm::attachment::cli::run_cancellable(args, cancel.clone());
    tokio::pin!(operation);
    tokio::select! { biased;
        _ = attachment_interrupt() => {
            cancel.cancel();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), &mut operation).await;
            attachment_notice("attachment connection interrupted; enrollment unchanged").await;
            std::process::exit(130)
        }
        result = &mut operation => result.map_err(anyhow::Error::from),
    }
}

pub(crate) async fn attachment_interrupt() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let (Ok(mut term), Ok(mut hangup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) {
            tokio::select! { _=tokio::signal::ctrl_c()=>(), _=term.recv()=>(), _=hangup.recv()=>() }
            return;
        }
    }
    #[cfg(windows)]
    {
        if let Ok(mut interrupt) = tokio::signal::windows::ctrl_break() {
            tokio::select! { _=tokio::signal::ctrl_c()=>(), _=interrupt.recv()=>() }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
