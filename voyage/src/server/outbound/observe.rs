//! Fenced remote observations without starting or recovering an execution runtime.
use super::*;
use crate::policy::ExecutionAuthority;
use serde::{Deserialize, Serialize};
use voyage_protocol::process::{read_frame, write_frame};

#[derive(Serialize, Deserialize)]
pub(super) struct Request {
    pub context: proxy::ContextData,
    pub incarnation: Uuid,
    pub token: String,
    pub until: u64,
    pub frame: Frame,
}
#[derive(Debug)]
struct ObservationAuthority {
    until: u64,
    grant: Arc<RemoteGrantObserver>,
}
impl ExecutionAuthority for ObservationAuthority {
    fn check(&self) -> Result<()> {
        ensure!(
            proxy::now() < self.until,
            "remote observation lease expired"
        );
        self.grant.check()
    }
}
pub async fn run(directory: PathBuf) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let request: Request = read_frame(&mut tokio::io::stdin()).await?;
        let frame = observe(&directory, &request).await.ok();
        write_frame(&mut tokio::io::stdout(), &frame).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    if !matches!(result, Ok(Ok(()))) {
        std::process::exit(1);
    }
    Ok(())
}
async fn observe(directory: &std::path::Path, request: &Request) -> Result<Frame> {
    let startup = crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match startup.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(error) => return Err(anyhow::anyhow!("runtime startup owned: {error}")),
        }
    }
    let registration = super::super::transport::registration(directory)?;
    ensure!(
        request.incarnation == registration.incarnation && request.token == registration.token,
        "observation authentication failed"
    );
    super::super::suspended::check_suspended(directory, &registration)?;
    #[derive(Deserialize)]
    struct ActorFile {
        version: u32,
        actor: LocalActor,
    }
    let actor: ActorFile = serde_json::from_slice(
        &super::super::bootstrap::read_private_artifact(&directory.join("identity/actor.json"))?,
    )?;
    ensure!(actor.version == 1, "unsupported local actor identity");
    let identity = &request.context.identity;
    let binding = RemoteBinding {
        origin: identity.origin.clone(),
        machine_id: identity.machine_id,
        owner_id: identity.owner_id,
        epoch: identity.epoch,
        local_installation_id: actor.actor.installation_id,
        local_principal_id: actor.actor.principal_id,
    };
    let owner =
        super::super::suspended::open_owner(directory.join("journal"), registration.session_id)
            .await?;
    let authority = Arc::new(ObservationAuthority {
        until: request.until,
        grant: Arc::new(owner.remote_grant_observer(binding.clone()).await?),
    });
    authority.check()?;
    let connection_id = request.context.connection_id;
    let session_id = registration.session_id;
    let output = match &request.frame {
        Frame::Command { command } => {
            ensure!(
                command.machine_id == identity.machine_id
                    && command.principal_id == identity.owner_id
                    && command.connection_id == connection_id
                    && command.version == VERSION,
                "remote command identity mismatch"
            );
            let reply = if command
                .validate(SystemClock.now_ms().map_err(anyhow::Error::msg)?)
                .is_err()
            {
                Reply::Denied {
                    code: DenialCode::Expired,
                }
            } else {
                match &command.operation {
                    Operation::List { after, limit } if *limit > 0 => {
                        let Reply::ExecutionSnapshot { session, .. } = owner
                            .remote_snapshot(binding.clone(), authority.clone())
                            .await?
                        else {
                            bail!("remote snapshot unavailable")
                        };
                        Reply::Sessions {
                            sessions: if after.is_none_or(|after| session_id > after) {
                                vec![session]
                            } else {
                                vec![]
                            },
                        }
                    }
                    Operation::Inspect {
                        session_id: selected,
                    } if *selected == session_id => {
                        owner
                            .remote_snapshot(binding.clone(), authority.clone())
                            .await?
                    }
                    _ => Reply::Denied {
                        code: DenialCode::Unauthorized,
                    },
                }
            };
            let config = match owner.saved_configuration().await? {
                Some(settings) => {
                    serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
                        .resolve(&registration.workspace)?
                }
                None => super::super::bootstrap::load_config(
                    registration.config_path.as_deref(),
                    &registration.workspace,
                )?,
            };
            Frame::Result {
                connection_id,
                command_id: command.command_id,
                reply: public_reply(reply, &redactor(&config)),
            }
        }
        Frame::ReplayRequest {
            connection_id: requested_connection,
            request_id,
            session_id: selected,
            after,
            limit,
            ..
        } => {
            ensure!(
                *requested_connection == connection_id && *selected == session_id,
                "remote replay identity mismatch"
            );
            match owner
                .remote_replay(
                    binding.clone(),
                    after.get(),
                    usize::from(*limit),
                    authority.clone(),
                )
                .await?
            {
                RemoteReplay::Events { events, latest } => Frame::Replay {
                    connection_id,
                    request_id: *request_id,
                    session_id,
                    after: *after,
                    latest: voyage_protocol::events::EventCursor::new(latest)
                        .map_err(anyhow::Error::msg)?,
                    events,
                },
                RemoteReplay::InvalidCursor => Frame::Result {
                    connection_id,
                    command_id: *request_id,
                    reply: Reply::Denied {
                        code: DenialCode::InvalidRequest,
                    },
                },
                RemoteReplay::SnapshotRequired { latest } => Frame::SnapshotRequired {
                    connection_id,
                    request_id: *request_id,
                    session_id,
                    after: *after,
                    latest: voyage_protocol::events::EventCursor::new(latest)
                        .map_err(anyhow::Error::msg)?,
                },
            }
        }
        _ => bail!("not a remote observation"),
    };
    authority.check()?;
    super::super::suspended::check_suspended(directory, &registration)?;
    Ok(output)
}
