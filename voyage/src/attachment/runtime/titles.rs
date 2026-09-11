//! Borrowed, run-owned utility work: no detached tasks or restart replay.
use super::*;

impl ManagedRunCheckpoint {
    pub(super) async fn with_title_updates<T>(
        &self,
        agent: &Agent,
        cancel: CancellationToken,
        execution: impl std::future::Future<Output = T>,
    ) -> T {
        let done = CancellationToken::new();
        let stop = done.clone().drop_guard();
        let execution = async {
            let result = execution.await;
            drop(stop);
            result
        };
        let (result, ()) = tokio::join!(execution, self.update_titles(agent, cancel, done));
        result
    }

    async fn update_titles(
        &self,
        agent: &Agent,
        cancel: CancellationToken,
        done: CancellationToken,
    ) {
        let mut attempted = None;
        loop {
            if cancel.is_cancelled() {
                return;
            }
            let closing = done.is_cancelled();
            let token = self.token.clone();
            let input = self
                .storage(move |store| {
                    steering::authorize(store, &token)?;
                    if let Some(authority) = &token.execution_authority {
                        authority.check()?;
                    }
                    store.journal.title_input(store.session_id)
                })
                .await;
            let Ok(input) = input else { return };
            if let Some((id, session)) = input
                && attempted != Some(id)
            {
                attempted = Some(id);
                if let Some(result) = agent
                    .generate_title_for_session(&session, cancel.clone())
                    .await
                {
                    if cancel.is_cancelled() {
                        return;
                    }
                    let token = self.token.clone();
                    let cancellation = cancel.clone();
                    let _ = self
                        .storage(move |store| {
                            if cancellation.is_cancelled() {
                                return Ok(());
                            }
                            steering::authorize(store, &token)?;
                            if let Some(authority) = &token.execution_authority {
                                authority.check()?;
                            }
                            store
                                .journal
                                .apply_title(&store.guard, store.run_id, id, result)
                        })
                        .await;
                }
                // Drain the newest intent once after execution ends, never keep a
                // finished run alive indefinitely under continuous incoming input.
                if closing {
                    return;
                }
                continue;
            }
            if closing {
                return;
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = done.cancelled() => {},
                _ = self.token.title_input.notified() => {},
            }
        }
    }
}

#[cfg(test)]
mod tests;
