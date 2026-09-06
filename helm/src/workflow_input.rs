//! Private workflow input restoration and bounded cancellation notices.
use crate::attachment_ui::attachment_notice;
use anyhow::Result;
// A stalled terminal (or full stderr pipe) must not obstruct cancellation after
// native mode restoration. The process exits after this bounded best effort.
pub(crate) async fn prepare_workflow(
    args: helm::workflow::WorkflowArgs,
    workspace: &std::path::Path,
) -> Result<Option<helm::workflow::Prepared>> {
    match helm::workflow::prepare(args, workspace).await {
        Err(error)
            if error
                .downcast_ref::<helm::workflow::InputFailure>()
                .is_some() =>
        {
            let (message, status) = match error
                .downcast_ref::<helm::workflow::InputFailure>()
                .unwrap()
            {
                helm::workflow::InputFailure::Cancelled => ("workflow input cancelled", 130),
                helm::workflow::InputFailure::TimedOut => ("workflow input timed out", 1),
                helm::workflow::InputFailure::Restoration => {
                    ("workflow terminal restoration failed", 1)
                }
            };
            // A stalled terminal may hold stderr's lock. Restoration has already
            // completed; bound the notice and never await a blocked input thread.
            attachment_notice(message).await;
            std::process::exit(status)
        }
        result => result,
    }
}
