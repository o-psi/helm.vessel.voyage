//! Explicit foreground presence. No provider, session or command dispatcher exists here.
use super::{CliError, EnrollmentClient};
use crate::attachment::transport;
use std::{io::Write, time::Duration};
use tokio_util::sync::CancellationToken;
use voyage_protocol::events::Features;

pub(super) async fn run(
    client: EnrollmentClient,
    cancel: CancellationToken,
) -> Result<(), CliError> {
    let mut connection = transport::connect(client, Features::default(), cancel.clone()).await?;
    let context = connection.context();
    let notice = serde_json::json!({
        "event": "attachment_connected", "mode": "presence_only",
        "machine_id": context.machine_id, "connection_id": context.connection_id,
    });
    let notice = tokio::select! { biased;
        _ = cancel.cancelled() => Err(transport::TransportError::Closed.into()),
        result = write_notice(notice.to_string()) => result,
    };
    if let Err(error) = notice {
        connection.close().await;
        return Err(error);
    }
    // Heartbeats run inside the connection. An application request is never
    // dispatched, acknowledged or retained by this presence-only frontend.
    let unexpected = connection.receive().await.is_some();
    connection.close().await;
    Err(if unexpected {
        transport::TransportError::Invalid
    } else {
        transport::TransportError::Closed
    }
    .into())
}
async fn write_notice(notice: String) -> Result<(), CliError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // A blocked output worker owns only this bounded, content-free notice, never
    // enrollment or its file lease. Runtime shutdown need not join this thread.
    std::thread::Builder::new()
        .name("attachment-output".into())
        .spawn(move || {
            let mut stdout = std::io::stdout().lock();
            let written = writeln!(stdout, "{notice}")
                .and_then(|()| stdout.flush())
                .is_ok();
            let _ = sender.send(written);
        })
        .map_err(|_| CliError::Output)?;
    if matches!(
        tokio::time::timeout(Duration::from_secs(2), receiver).await,
        Ok(Ok(true))
    ) {
        Ok(())
    } else {
        Err(CliError::Output)
    }
}
