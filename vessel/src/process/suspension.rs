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
        if info.state == ProcessState::Unavailable {
            self.recover_abandoned(session, registration.incarnation)
                .await?;
        } else {
            ensure!(
                info.state == ProcessState::Suspended,
                "owner cannot be resumed from its current lifecycle state"
            );
        }
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
        // A long-lived SSE observer must never hold the lifecycle lock and delay
        // admission, suspension or recovery. Events are read-only, bounded and
        // incarnation checked. A concurrent lifecycle transition can end this
        // observation; Helm reconnects from its unchanged durable cursor.
        if let RuntimeCommand::Events {
            after,
            limit,
            wait_ms,
        } = command
        {
            let registration = self.registration(session).await?;
            ensure!(
                registration.incarnation == incarnation,
                "stale runtime incarnation"
            );
            let directory = registry::directory(&self.directory, session);
            if super::recovery::suspended(&directory, &registration) {
                let read = || {
                    observe(
                        &directory,
                        &registration,
                        RuntimeCommand::Events {
                            after,
                            limit,
                            wait_ms: 0,
                        },
                        authorization.clone(),
                    )
                };
                let response = read().await?;
                if wait_ms > 0
                    && response.error.is_none()
                    && response.result["replay_gap"] != true
                    && response.result["events"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                {
                    tokio::time::sleep(Duration::from_millis(u64::from(wait_ms))).await;
                    let current = self.registration(session).await?;
                    ensure!(
                        current.incarnation == incarnation
                            && super::recovery::suspended(&directory, &current),
                        "runtime changed during event observation"
                    );
                    return read().await;
                }
                return Ok(response);
            }
            return routing::forward_authorized(
                &directory,
                &registration,
                RuntimeCommand::Events {
                    after,
                    limit,
                    wait_ms,
                },
                authorization,
            )
            .await;
        }
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
            if matches!(command, RuntimeCommand::Resolve { .. })
                && !directory.join("runtime.sock").exists()
                && registration.state != ProcessState::Relinquished
                && super::recovery::clean_stop(&directory, &registration)
            {
                // An upgraded supervisor must resolve old saved deliveries even
                // when the retired executable predates Resolve. The current
                // one-shot runtime takes the existing execution/startup fences;
                // it never starts an agent or changes the incarnation.
                let mut observer = registration.clone();
                observer.executable = Some(self.binary.clone());
                let mut response = observe(&directory, &observer, command, authorization).await?;
                if resumed {
                    response.resumed_from = Some(incarnation);
                }
                return Ok(response);
            }
            if !directory.join("runtime.sock").exists()
                && !super::recovery::clean_stop(&directory, &registration)
                && registration.state != ProcessState::Relinquished
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
                    "the addressed runtime is gone; inspect the recovered voyage before acting on live resources"
                );
                self.recover_abandoned(session, registration.incarnation)
                    .await?;
                self.restart(Uuid::new_v4(), session, registration.incarnation)
                    .await?;
                registration = self.registration(session).await?;
                resumed = true;
            }
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
                    let observed_name = if response.error.is_none() {
                        match &command {
                            RuntimeCommand::Snapshot => response.result["name"].as_str(),
                            RuntimeCommand::Rename { name, .. } => Some(name.as_str()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(name) = observed_name {
                        self.remember_name(session, registration.incarnation, name)
                            .await?;
                    }
                    if resumed {
                        response.resumed_from = Some(incarnation);
                    }
                    return Ok(response);
                }
                Err(error)
                    if error.downcast_ref::<routing::NotConnected>().is_some()
                        && !super::recovery::clean_stop(&directory, &registration)
                        && registration.state != ProcessState::Relinquished
                        && tokio::time::Instant::now() < deadline =>
                {
                    // Connecting failed before any request bytes were sent. A stale
                    // endpoint can survive an unclean exit, so absence alone is not
                    // the only abandoned-owner signal.
                    ensure!(
                        !matches!(
                            command,
                            RuntimeCommand::ExecuteTool { .. }
                                | RuntimeCommand::Terminal { .. }
                                | RuntimeCommand::Steer { .. }
                                | RuntimeCommand::Respond { .. }
                                | RuntimeCommand::Cancel { .. }
                        ),
                        "the addressed runtime is gone; inspect the recovered voyage before acting on live resources"
                    );
                    self.recover_abandoned(session, registration.incarnation)
                        .await?;
                    self.restart(Uuid::new_v4(), session, registration.incarnation)
                        .await?;
                    registration = self.registration(session).await?;
                    resumed = true;
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

    async fn remember_name(&self, session: Uuid, incarnation: Uuid, name: &str) -> Result<()> {
        ensure!(
            !name.is_empty()
                && name.len() <= 256
                && !name.chars().any(|character| character.is_control()),
            "invalid public voyage name"
        );
        let mut registrations = self.registrations.lock().await;
        let registration = registrations.get_mut(&session).context("unknown session")?;
        ensure!(
            registration.incarnation == incarnation,
            "runtime changed while saving voyage name"
        );
        if registration.name.as_deref() != Some(name) {
            registration.name = Some(name.to_owned());
            registry::save(&registry::directory(&self.directory, session), registration)?;
        }
        Ok(())
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
