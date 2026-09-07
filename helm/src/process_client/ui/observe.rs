//! Slow observers cannot block the terminal input loop or another Vessel.
use super::state::{Snapshot, Target};
use crate::process_client::transport::Client;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, mpsc};
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

pub enum Update {
    FirstSend {
        saved: Box<super::new_draft::Saved>,
        result: Result<Option<serde_json::Value>, String>,
    },
    Live {
        target: Target,
        incarnation: uuid::Uuid,
        run: uuid::Uuid,
        offset: u64,
        total: u64,
        result: Result<String, String>,
    },
    History {
        target: Target,
        incarnation: uuid::Uuid,
        revision: u64,
        result: Result<Vec<super::state::Message>, String>,
    },
    Completion {
        target: Target,
        incarnation: uuid::Uuid,
        section: &'static str,
        value: Option<serde_json::Value>,
    },
    Terminals {
        target: Target,
        incarnation: uuid::Uuid,
        result: Result<super::terminals::Inventory, String>,
        observed: Instant,
    },
    Control {
        target: Target,
        incarnation: uuid::Uuid,
        result: Result<String, String>,
    },
    Catalogue {
        route: usize,
        processes: Vec<ProcessInfo>,
    },
    Snapshot {
        target: Target,
        incarnation: uuid::Uuid,
        result: Box<Result<Snapshot, String>>,
    },
    RouteError {
        route: usize,
        error: String,
    },
    Command {
        target: Target,
        command_id: uuid::Uuid,
        refused: bool,
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
                let cursors = Arc::new(tokio::sync::Mutex::new(HashMap::<
                    (uuid::Uuid, uuid::Uuid),
                    u64,
                >::new()));
                loop {
                    let result = client
                        .request(VesselCommand::Catalogue)
                        .await
                        .and_then(|value| Ok(serde_json::from_value::<Vec<ProcessInfo>>(value)?));
                    match result {
                        Ok(processes) => {
                            cursors.lock().await.retain(|(id, incarnation), _| {
                                processes
                                    .iter()
                                    .any(|p| p.session_id == *id && p.incarnation == *incarnation)
                            });
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
                                if process.archive.is_some() || process.deletion.is_some() {
                                    continue;
                                }
                                let Ok(permit) = limit.clone().acquire_owned().await else {
                                    break;
                                };
                                let client = client.clone();
                                let cursors = cursors.clone();
                                let sender = sender.clone();
                                jobs.spawn(async move {
                                    let _permit = permit;
                                    let target = Target {
                                        route,
                                        session: process.session_id,
                                    };
                                    let key = (process.session_id, process.incarnation);
                                    let after = cursors.lock().await.get(&key).copied();
                                    if let Some(after) = after {
                                        // Events are durable invalidations. A gap or unsupported
                                        // capability is recovered by the snapshot below, never replay.
                                        let _ = client
                                            .forward(
                                                target.session,
                                                process.incarnation,
                                                RuntimeCommand::Events {
                                                    after,
                                                    limit: 128,
                                                    wait_ms: 0,
                                                },
                                            )
                                            .await;
                                    }
                                    let result = client
                                        .forward(
                                            target.session,
                                            process.incarnation,
                                            RuntimeCommand::Snapshot,
                                        )
                                        .await
                                        .and_then(|value| {
                                            Ok(serde_json::from_value::<Snapshot>(value)?)
                                        })
                                        .map_err(|error| error.to_string());
                                    if let Ok(snapshot) = &result
                                        && let Some(cursor) = snapshot.observation_cursor
                                    {
                                        cursors.lock().await.insert(key, cursor);
                                    }
                                    let _ = sender
                                        .send(Update::Snapshot {
                                            target,
                                            incarnation: process.incarnation,
                                            result: Box::new(result),
                                        })
                                        .await;
                                    let inventory = client
                                        .forward(
                                            target.session,
                                            process.incarnation,
                                            RuntimeCommand::Controls {
                                                run_id: None,
                                                section: "terminals".into(),
                                            },
                                        )
                                        .await
                                        .and_then(|value| Ok(serde_json::from_value(value)?))
                                        .map_err(|error| error.to_string());
                                    let _ = sender
                                        .send(Update::Terminals {
                                            target,
                                            incarnation: process.incarnation,
                                            result: inventory,
                                            observed: Instant::now(),
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
