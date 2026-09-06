//! Slow observers cannot block the terminal input loop or another Vessel.
use super::state::{Snapshot, Target};
use crate::process_client::transport::Client;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

pub enum Update {
    Catalogue {
        route: usize,
        processes: Vec<ProcessInfo>,
    },
    Snapshot {
        target: Target,
        incarnation: uuid::Uuid,
        result: Result<Snapshot, String>,
    },
    RouteError {
        route: usize,
        error: String,
    },
    Command {
        target: Target,
        command_id: uuid::Uuid,
        result: Result<serde_json::Value, String>,
    },
    Created {
        route: usize,
        result: Result<ProcessInfo, String>,
    },
}

pub fn spawn(clients: &[Client], sender: mpsc::Sender<Update>) -> Vec<tokio::task::JoinHandle<()>> {
    clients
        .iter()
        .cloned()
        .enumerate()
        .map(|(route, client)| {
            let sender = sender.clone();
            tokio::spawn(async move {
                let limit = Arc::new(Semaphore::new(8));
                loop {
                    let result = client
                        .request(VesselCommand::Catalogue)
                        .await
                        .and_then(|value| Ok(serde_json::from_value::<Vec<ProcessInfo>>(value)?));
                    match result {
                        Ok(processes) => {
                            if sender
                                .send(Update::Catalogue {
                                    route,
                                    processes: processes.clone(),
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                            let mut jobs = tokio::task::JoinSet::new();
                            for process in processes.into_iter().take(256) {
                                let Ok(permit) = limit.clone().acquire_owned().await else {
                                    break;
                                };
                                let client = client.clone();
                                let sender = sender.clone();
                                jobs.spawn(async move {
                                    let _permit = permit;
                                    let target = Target {
                                        route,
                                        session: process.session_id,
                                    };
                                    let result = client
                                        .forward(
                                            target.session,
                                            process.incarnation,
                                            RuntimeCommand::Snapshot,
                                        )
                                        .await
                                        .and_then(|value| Ok(serde_json::from_value(value)?))
                                        .map_err(|error| error.to_string());
                                    let _ = sender
                                        .send(Update::Snapshot {
                                            target,
                                            incarnation: process.incarnation,
                                            result,
                                        })
                                        .await;
                                });
                            }
                            while jobs.join_next().await.is_some() {}
                        }
                        Err(error) => {
                            if sender
                                .send(Update::RouteError {
                                    route,
                                    error: error.to_string(),
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(750)).await;
                }
            })
        })
        .collect()
}
