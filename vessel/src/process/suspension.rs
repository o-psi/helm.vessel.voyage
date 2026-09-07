//! Clean automatic resume and bounded observations; never retry uncertain effects.
use super::{registry, routing, service::Supervisor};
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;
use voyage_protocol::process::*;

impl Supervisor {
    pub(super) async fn stop(
        &self,
        session: Uuid,
        incarnation: Uuid,
        authorization: Option<GrantBinding>,
    ) -> Result<RuntimeResponse> {
        let lock = self.lifecycle_lock(session).await?;
        let _guard = lock.lock().await;
        let mut registration = self.registration(session).await?;
        ensure!(
            registration.incarnation == incarnation,
            "stale runtime incarnation"
        );
        ensure!(
            registration.state != ProcessState::Relinquished,
            "source ownership has been permanently relinquished"
        );
        let directory = registry::directory(&self.directory, session);
        if super::recovery::suspended(&directory, &registration) {
            if authorization.is_some() {
                let checked = observe(
                    &directory,
                    &registration,
                    RuntimeCommand::Stop,
                    authorization.clone(),
                )
                .await?;
                ensure!(
                    checked.error.is_none(),
                    "suspended lifecycle authority refused"
                );
            }
            // No process or owned resource remains to cancel. Preserve the runtime's
            // cleanup proof while recording the explicit stop in supervisor metadata.
            let mut registrations = self.registrations.lock().await;
            ensure!(
                registrations
                    .get(&session)
                    .is_some_and(|r| r.incarnation == incarnation),
                "runtime changed during stop"
            );
            registration.state = ProcessState::Stopped;
            registry::save(&directory, &registration)?;
            registrations.insert(session, registration);
            return Ok(RuntimeResponse {
                protocol: PROCESS_PROTOCOL,
                session_id: session,
                incarnation,
                resumed_from: None,
                error: None,
                outcome_unknown: false,
                result: serde_json::json!({"status":"stopped","cleanup":"observed"}),
            });
        }
        let mut registrations = self.registrations.lock().await;
        ensure!(
            registrations
                .get(&session)
                .is_some_and(|r| r.incarnation == incarnation),
            "runtime changed during stop"
        );
        registration.state = ProcessState::CleanupUnconfirmed;
        registry::save(&directory, &registration)?;
        registrations.insert(session, registration.clone());
        drop(registrations);
        routing::forward_authorized(
            &directory,
            &registration,
            RuntimeCommand::Stop,
            authorization,
        )
        .await
    }
    async fn lifecycle_lock(&self, session: Uuid) -> Result<Arc<Mutex<()>>> {
        self.registration(session).await?;
        Ok(self
            .lifecycle_locks
            .lock()
            .await
            .entry(session)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }

    pub(super) async fn wake(&self, session: Uuid) -> Result<serde_json::Value> {
        let lock = self.lifecycle_lock(session).await?;
        let _guard = lock.lock().await;
        let registration = self.registration(session).await?;
        let directory = registry::directory(&self.directory, session);
        let info = routing::inspect(&directory, &registration).await;
        if info.state == ProcessState::Live {
            return Ok(serde_json::to_value(info)?);
        }
        ensure!(
            info.state == ProcessState::Suspended,
            "owner is not cleanly suspended; explicit recovery required"
        );
        self.restart(Uuid::new_v4(), session, registration.incarnation)
            .await
    }

    pub(super) async fn forward_resuming(
        &self,
        session: Uuid,
        incarnation: Uuid,
        command: RuntimeCommand,
        authorization: Option<GrantBinding>,
    ) -> Result<RuntimeResponse> {
        let lock = self.lifecycle_lock(session).await?;
        let _guard = lock.lock().await;
        let mut registration = self.registration(session).await?;
        ensure!(
            registration.incarnation == incarnation,
            "stale runtime incarnation"
        );
        let directory = registry::directory(&self.directory, session);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        let mut resumed = false;
        loop {
            if super::recovery::suspended(&directory, &registration)
                && !command.observes_suspended()
            {
                ensure!(
                    !matches!(
                        command,
                        RuntimeCommand::ExecuteTool { .. }
                            | RuntimeCommand::Terminal { .. }
                            | RuntimeCommand::Steer { .. }
                            | RuntimeCommand::Respond { .. }
                            | RuntimeCommand::Cancel { .. }
                    ),
                    "turn has finished; inspect its receipt before acting on live resources"
                );
                self.restart(Uuid::new_v4(), session, registration.incarnation)
                    .await?;
                registration = self.registration(session).await?;
                resumed = true;
            }
            let response = routing::forward_authorized(
                &directory,
                &registration,
                command.clone(),
                authorization.clone(),
            )
            .await;
            match response {
                Ok(response)
                    if response.error.is_none()
                        && response.result["status"] == "suspending"
                        && response.result["not_dispatched"] == true =>
                {
                    // This explicit response is proof that dispatch never ran. No I/O
                    // failure or unknown command outcome reaches this retry branch.
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "runtime is still completing suspension; command was not dispatched"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok(mut response) => {
                    if resumed {
                        response.resumed_from = Some(incarnation);
                    }
                    return Ok(response);
                }
                Err(_)
                    if command.observes_suspended() && tokio::time::Instant::now() < deadline =>
                {
                    // A read can lose its socket while the completed owner retires.
                    // Re-observe under the same identity; this branch never admits
                    // a turn, replays a mutation, or treats absence as cleanup proof.
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error)
                    if error.downcast_ref::<routing::NotConnected>().is_some()
                        && super::recovery::clean_stop(&directory, &registration)
                        && tokio::time::Instant::now() < deadline =>
                {
                    // No request bytes were sent, and clean stop evidence exists.
                    // Only a suspension marker can enable the wake on the next pass.
                    if !super::recovery::suspended(&directory, &registration) {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }
}

pub(super) async fn observe(
    directory: &Path,
    registration: &ProcessRegistration,
    command: RuntimeCommand,
    authorization: Option<GrantBinding>,
) -> Result<RuntimeResponse> {
    ensure!(
        command.observes_suspended(),
        "command requires execution owner"
    );
    let binary = registration
        .executable
        .as_ref()
        .context("suspended observation binary missing")?;
    let mut child = tokio::process::Command::new(binary)
        .arg("observe-suspended")
        .arg("--directory")
        .arg(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut input = child.stdin.take().context("observer input unavailable")?;
        write_frame(
            &mut input,
            &RuntimeRequest {
                protocol: PROCESS_PROTOCOL,
                session_id: registration.session_id,
                incarnation: registration.incarnation,
                token: registration.token.clone(),
                authorization,
                command,
            },
        )
        .await?;
        drop(input);
        let response: RuntimeResponse =
            read_frame(&mut child.stdout.take().context("observer output unavailable")?).await?;
        ensure!(
            child.wait().await?.success(),
            "suspended observation failed"
        );
        ensure!(
            response.protocol == PROCESS_PROTOCOL
                && response.session_id == registration.session_id
                && response.incarnation == registration.incarnation
                && response.resumed_from.is_none(),
            "observer identity mismatch"
        );
        Ok(response)
    })
    .await;
    match result {
        Ok(value) => value,
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!("suspended observation deadline elapsed")
        }
    }
}
