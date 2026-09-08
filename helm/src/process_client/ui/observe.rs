//! Slow observers cannot block the terminal input loop or another Vessel.
use super::state::{Route, Snapshot, Target};
use crate::process_client::transport::Client;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, mpsc};
use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VesselEventSubscription, VoyageCommand};

pub enum Update {
    Vessels(super::vessels::Event),
    DraftInferenceModels {
        id: uuid::Uuid,
        provider: String,
        context: Option<[u8; 32]>,
        generation: uuid::Uuid,
        models: Option<Vec<crate::provider::ModelInfo>>,
    },
    InferenceModels {
        route: Option<Route>,
        id: uuid::Uuid,
        context: Option<[u8; 32]>,
        generation: Option<uuid::Uuid>,
        result: Result<serde_json::Value, String>,
    },
    FirstSend {
        route: Route,
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
        route: Route,
        processes: Vec<ProcessInfo>,
    },
    Snapshot {
        target: Target,
        incarnation: uuid::Uuid,
        result: Box<Result<Snapshot, String>>,
    },
    RouteError {
        route: Route,
        error: String,
    },
    Command {
        target: Target,
        command_id: uuid::Uuid,
        refused: bool,
        result: Result<serde_json::Value, String>,
    },
    Created {
        route: Route,
        result: Result<ProcessInfo, String>,
    },
}

pub fn spawn(
    client: Client,
    route: Route,
    sender: mpsc::Sender<Update>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut cursors = HashMap::<(uuid::Uuid, uuid::Uuid), u64>::new();
        let mut stream_round = 0usize;
        loop {
            let streaming = client.supports_events().await;
            let result = client
                .request(VesselCommand::Catalogue)
                .await
                .and_then(|value| Ok(serde_json::from_value::<Vec<ProcessInfo>>(value)?));
            match result {
                Ok(processes) => {
                    cursors.retain(|(id, incarnation), _| {
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
                    let processes = processes
                        .into_iter()
                        .filter(|process| process.archive.is_none() && process.deletion.is_none())
                        .take(256)
                        .collect::<Vec<_>>();
                    let limit = Arc::new(Semaphore::new(8));
                    let mut initial = tokio::task::JoinSet::new();
                    for process in &processes {
                        let key = (process.session_id, process.incarnation);
                        if !cursors.contains_key(&key) {
                            let client = client.clone();
                            let process = process.clone();
                            let sender = sender.clone();
                            let limit = limit.clone();
                            initial.spawn(async move {
                                let Ok(_permit) = limit.acquire_owned().await else {
                                    return (key, None);
                                };
                                (key, refresh(&client, route, &process, &sender).await)
                            });
                        }
                    }
                    while let Some(result) = initial.join_next().await {
                        if let Ok((key, Some(cursor))) = result {
                            cursors.insert(key, cursor);
                        }
                    }
                    if !streaming {
                        for process in &processes {
                            if let Some(cursor) = refresh(&client, route, process, &sender).await {
                                cursors.insert((process.session_id, process.incarnation), cursor);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(750)).await;
                        continue;
                    }
                    let subscriptions = processes
                        .iter()
                        .filter(|process| {
                            process.state == voyage_protocol::vessel::ProcessState::Live
                        })
                        .filter_map(|process| {
                            let key = (process.session_id, process.incarnation);
                            cursors.get(&key).map(|after| VesselEventSubscription {
                                session_id: process.session_id,
                                incarnation: process.incarnation,
                                after: *after,
                            })
                        })
                        .collect::<Vec<_>>();
                    // The gateway admits at most 32 distinct subscriptions. Rotate
                    // bounded groups; snapshot refresh above still observes every
                    // permitted catalogue entry, not just the first page of live work.
                    let count = subscriptions.len();
                    let subscriptions = if count > 32 {
                        let start = stream_round % count;
                        stream_round = start + 32;
                        subscriptions
                            .iter()
                            .cycle()
                            .skip(start)
                            .take(32)
                            .cloned()
                            .collect()
                    } else {
                        subscriptions
                    };
                    if subscriptions.is_empty() {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                    match client.events(subscriptions).await {
                        Ok(mut events) => {
                            let refresh_catalogue = tokio::time::sleep(Duration::from_secs(5));
                            tokio::pin!(refresh_catalogue);
                            loop {
                                tokio::select! {
                                    _ = &mut refresh_catalogue => break,
                                    event = events.next() => {
                                        let Some(event) = event else { break; };
                                        match event {
                                            Ok(event) => {
                                                if processes.iter().any(|process| process.session_id == event.session_id && process.incarnation != event.incarnation) {
                                                    // Refresh the catalogue/snapshot on a service-reported owner transition.
                                                    break;
                                                }
                                                if let Some(error) = event.error {
                                                    let _ = sender.send(Update::RouteError {
                                                        route,
                                                        error: crate::process_client::safe(&error),
                                                    }).await;
                                                    break;
                                                }
                                                if let Some(process) = processes.iter().find(|process| {
                                                    process.session_id == event.session_id
                                                        && process.incarnation == event.incarnation
                                                }) && let Some(cursor) = refresh(
                                                        &client,
                                                        route,
                                                        process,
                                                        &sender,
                                                    ).await {
                                                    cursors.insert(
                                                        (process.session_id, process.incarnation),
                                                        cursor,
                                                    );
                                                }
                                            }
                                            Err(error) => {
                                                let _ = sender.send(Update::RouteError {
                                                    route,
                                                    error: error.to_string(),
                                                }).await;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
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
                            tokio::time::sleep(Duration::from_millis(750)).await;
                        }
                    }
                    // A stream can end while its owner is suspending, after
                    // the final journal update but before we receive it.
                    // Re-establish snapshots before subscribing again,
                    // including owners that are no longer live.
                    cursors.clear();
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
                    tokio::time::sleep(Duration::from_millis(750)).await;
                }
            }
        }
    })
}

async fn refresh(
    client: &Client,
    route: Route,
    process: &ProcessInfo,
    sender: &mpsc::Sender<Update>,
) -> Option<u64> {
    let target = Target {
        route,
        session: process.session_id,
    };
    let result = client
        .voyage(target.session, process.incarnation, VoyageCommand::Snapshot)
        .await
        .and_then(|value| Ok(serde_json::from_value::<Snapshot>(value)?))
        .map_err(|error| error.to_string());
    let cursor = result
        .as_ref()
        .ok()
        .and_then(|snapshot| snapshot.observation_cursor);
    let _ = sender
        .send(Update::Snapshot {
            target,
            incarnation: process.incarnation,
            result: Box::new(result),
        })
        .await;
    let inventory = client
        .voyage(
            target.session,
            process.incarnation,
            VoyageCommand::Controls {
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
    cursor
}
