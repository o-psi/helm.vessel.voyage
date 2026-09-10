use super::*;
use sha2::{Digest, Sha256};
pub(super) async fn run(
    state: &Arc<State>,
    run: &mut crate::attachment::runtime::RunOwner,
    context: crate::tools::ToolContext,
    words: Vec<String>,
    cancel: CancellationToken,
) -> Result<()> {
    let id = run.record().await?.id;
    let reservation = match crate::host_resources::Reservation::acquire("executors", id, 1) {
        Ok(reservation) => reservation,
        Err(error) => {
            run.fail_before_execution_reason(crate::host_resources::startup_failure(&error))
                .await?;
            run.confirm_local_cleanup_observed().await?;
            return Ok(());
        }
    };
    run.start_operator().await?;
    let stopped = CancellationToken::new();
    let _stop = stopped.clone().drop_guard();
    let watcher = {
        let owner = state.owner.clone();
        let stopped = stopped.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {_=stopped.cancelled()=>return Ok::<(),anyhow::Error>(()),_=tokio::time::sleep(std::time::Duration::from_millis(50))=>{}}
                match owner.local_cancel_requested(id).await {
                    Ok(false) => {}
                    Ok(true) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    Err(error) => {
                        cancel.cancel();
                        return Err(error);
                    }
                }
            }
        })
    };
    let native_possible = words.iter().any(|word| word == "remotes");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        execute(state, run, &context, words),
    )
    .await;
    stopped.cancel();
    let watched = matches!(watcher.await, Ok(Ok(())));
    // Returned HTTP/storage failures have no owned child. A dropped operation or
    // failed native discovery does not establish process cleanup.
    let observed =
        (matches!(&result, Ok(Ok(_))) || (!native_possible && result.is_ok())) && watched;
    let outcome = match result {
        Ok(Ok(text)) => Ok(context.redactor.redact(text)),
        Ok(Err(_)) => {
            Err("GitHub operator failed; inspect its publication receipt before repeating".into())
        }
        Err(_) => {
            cancel.cancel();
            Err(
                "GitHub operator timed out; cleanup unconfirmed; inspect its publication receipt"
                    .into(),
            )
        }
    };
    run.finish_operator(outcome, cancel.is_cancelled()).await?;
    if observed {
        run.confirm_local_cleanup_observed().await?;
        reservation.release_observed()?;
    }
    Ok(())
}
async fn execute(
    state: &Arc<State>,
    run: &mut crate::attachment::runtime::RunOwner,
    context: &crate::tools::ToolContext,
    words: Vec<String>,
) -> Result<String> {
    if words.as_slice() == ["references"] {
        return Ok(serde_json::to_string_pretty(
            &state.owner.snapshot().await?.session.github_references,
        )?);
    }
    if words.first().is_some_and(|word| word == "unreference") {
        ensure!(words.len() == 2, "unreference needs one URL");
        context.policy.check_current()?;
        ensure!(
            context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
            "reference removal denied in read-only mode"
        );
        let removed = run
            .github_reference(
                None,
                Some(crate::github::repository::Object::parse(&words[1])?),
            )
            .await?;
        return Ok(format!("reference removed: {removed}"));
    }
    let mut result = crate::github::operator::execute(
        context.clone(),
        Some(state.registration.session_id),
        words,
    )
    .await?;
    if let Some(feedback) = result.feedback.take() {
        context.policy.check_current()?;
        let workspace = context.policy.workspace();
        let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
        let root = crate::build::resource_root().join("completion");
        crate::attachment::journal::prepare_directory(root.clone())?;
        let coordinator = crate::completion::runtime::Coordinator::open(root.join(key), workspace)?;
        let todo = crate::build::todo_tool(workspace, coordinator)
            .store()
            .import_github_feedback(
                feedback,
                context.policy.clone(),
                context.cancellation.clone(),
            )
            .await?;
        result
            .display
            .push_str(&format!("\nLocal task {} · {:?}", todo.id.0, todo.status));
    }
    if let Some(reference) = result.reference {
        context.policy.check_current()?;
        ensure!(
            context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
            "reference update denied in read-only mode"
        );
        run.github_reference(Some(reference), None).await?;
    }
    Ok(result.display)
}
