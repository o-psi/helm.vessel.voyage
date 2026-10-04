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
#[cfg(test)]
use voyage_protocol::vessel::VesselCommand;
use voyage_protocol::vessel::{ProcessInfo, VesselEventSubscription, VoyageCommand};

#[path = "catalogue_watch.rs"]
mod catalogue_watch;

pub enum Update {
    Execution {
        target: Target,
        incarnation: uuid::Uuid,
        result: Result<serde_json::Value, String>,
    },
    GoalOwner {
        target: Target,
        id: uuid::Uuid,
        owner: bool,
    },
    Inspection(Box<super::inspection_bridge::Loaded>),
    InboxAttention {
        route: Route,
        count: u64,
    },
    Operator(Box<super::operator_bridge::Loaded>),
    Browser {
        target: Target,
        result: Result<String, String>,
    },
    Coordination {
        origin: Target,
        request: uuid::Uuid,
        result: Result<Box<super::transcript::navigation::Located>, String>,
    },
    Vessels(super::vessels::Event),
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
        request_id: uuid::Uuid,
        target: Target,
        incarnation: uuid::Uuid,
        section: &'static str,
        context: Box<super::completion::LookupContext>,
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
    Event {
        target: Target,
        incarnation: uuid::Uuid,
        event: serde_json::Value,
        applied: tokio::sync::oneshot::Sender<bool>,
    },
    Snapshot {
        target: Target,
        incarnation: uuid::Uuid,
        result: Box<Result<Snapshot, String>>,
    },
    RouteUnavailable {
        route: Route,
        error: String,
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
        origin: Target,
        route: Route,
        result: Result<ProcessInfo, String>,
    },
}

pub fn spawn(
    client: Client,
    route: Route,
    sender: mpsc::Sender<Update>,
    mut selected: tokio::sync::watch::Receiver<Option<Target>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (_catalogue_observer, mut catalogue_updates) = catalogue_watch::spawn(client.clone());
        let mut cursors = HashMap::<(uuid::Uuid, uuid::Uuid), u64>::new();
        let mut retry_after = HashMap::<(uuid::Uuid, uuid::Uuid), (u32, Instant)>::new();
        let mut hydrated_selection = None;
        let mut hydrated = HashMap::<(uuid::Uuid, uuid::Uuid), (ProcessInfo, Instant)>::new();
        let mut route_failures = 0u32;
        let mut stream_round = 0usize;
        let mut inbox_count = 0u64;
        let mut inbox_probe = Instant::now() - Duration::from_secs(15);
        loop {
            let current = catalogue_updates.borrow_and_update().clone();
            let result = match current {
                Some(result) => result,
                None => {
                    if catalogue_updates.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            match result {
                Ok(processes) => {
                    route_failures = 0;
                    if inbox_probe.elapsed() >= Duration::from_secs(15) {
                        inbox_probe = Instant::now();
                        if let Some(count) = super::inbox::attention(&client).await {
                            if count > 0 && count != inbox_count {
                                let _ = sender.send(Update::InboxAttention { route, count }).await;
                            }
                            inbox_count = count;
                        }
                    }
                    hydrated.retain(|key, (previous, observed)| {
                        processes
                            .iter()
                            .find(|p| (p.session_id, p.incarnation) == *key)
                            .is_some_and(|current| {
                                !catalogue_changed(
                                    std::slice::from_ref(previous),
                                    std::slice::from_ref(current),
                                ) && (current.catalogue.is_some()
                                    || observed.elapsed() < Duration::from_secs(60))
                            })
                    });
                    retry_after.retain(|(id, incarnation), _| {
                        processes
                            .iter()
                            .any(|p| p.session_id == *id && p.incarnation == *incarnation)
                    });
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
                    let streaming = client.supports_events().await;
                    // New Vessels supply durable sidebar metadata. Only the
                    // selected voyage needs transcript/configuration observations.
                    let selected_target = *selected.borrow_and_update();
                    if selected_target != hydrated_selection {
                        // An unselected view may have shed its full transcript
                        // for a catalogue projection while retaining its cursor.
                        if let Some(target) = selected_target.filter(|t| t.route == route) {
                            cursors.retain(|(session, _), _| *session != target.session);
                            hydrated.retain(|(session, _), _| *session != target.session);
                        }
                        hydrated_selection = selected_target;
                    }
                    for process in &processes {
                        if let Some(summary) =
                            process.catalogue.as_ref().and_then(|c| c.summary.as_ref())
                        {
                            let key = (process.session_id, process.incarnation);
                            if cursors
                                .get(&key)
                                .is_some_and(|cursor| *cursor > summary.observation_cursor)
                            {
                                cursors.remove(&key);
                                hydrated.remove(&key);
                            }
                        }
                    }
                    let mut catalogue_baseline = processes.clone();
                    let processes = processes
                        .into_iter()
                        .filter(|process| {
                            process.catalogue.is_none()
                                || selected_target
                                    == Some(Target {
                                        route,
                                        session: process.session_id,
                                    })
                        })
                        .filter(|process| process.archive.is_none() && process.deletion.is_none())
                        .collect::<Vec<_>>();
                    let limit = Arc::new(Semaphore::new(8));
                    let mut initial = tokio::task::JoinSet::new();
                    for process in &processes {
                        let key = (process.session_id, process.incarnation);
                        if (!hydrated.contains_key(&key)
                            || (process.state == voyage_protocol::vessel::ProcessState::Live
                                && (!streaming || !cursors.contains_key(&key))))
                            && retry_after
                                .get(&key)
                                .is_none_or(|(_, when)| Instant::now() >= *when)
                        {
                            let client = client.clone();
                            let process = process.clone();
                            let sender = sender.clone();
                            let limit = limit.clone();
                            initial.spawn(async move {
                                let Ok(_permit) = limit.acquire_owned().await else {
                                    return (key, process, None);
                                };
                                let result =
                                    refresh_observation(&client, route, &process, &sender).await;
                                (key, process, result)
                            });
                        }
                    }
                    while let Some(result) = initial.join_next().await {
                        match result {
                            Ok((key, process, Some(cursor))) => {
                                hydrated.insert(key, (process, Instant::now()));
                                if let Some(cursor) = cursor {
                                    cursors.insert(key, cursor);
                                }
                                retry_after.remove(&key);
                            }
                            Ok((key, _, None)) => {
                                let attempts = retry_after
                                    .get(&key)
                                    .map_or(1, |(attempts, _)| attempts.saturating_add(1))
                                    .min(6);
                                retry_after.insert(
                                    key,
                                    (
                                        attempts,
                                        Instant::now() + Duration::from_secs(1 << attempts),
                                    ),
                                );
                            }
                            Err(_) => {}
                        }
                    }
                    if !streaming {
                        // Hydration above owns polling, invalidation and backoff.
                        // Saved voyages have no running event loop to poll.
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
                                projection: Some("public-v2".into()),
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
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                            changed = selected.changed() => { if changed.is_err() { return; } },
                            changed = catalogue_updates.changed() => { if changed.is_err() { return; } },
                        }
                        continue;
                    }
                    match client.events(subscriptions).await {
                        Ok(mut events) => {
                            let mut ended_unexpectedly = false;
                            // Only eligible legacy/catalogue-less groups larger
                            // than the stream limit need timed rotation. A healthy
                            // unchanged first group must not starve later owners.
                            let rotate = tokio::time::sleep(Duration::from_secs(2));
                            tokio::pin!(rotate);
                            // Metadata arrives independently; transcript reads never
                            // wait for a catalogue request or a catalogue timer.
                            loop {
                                tokio::select! {
                                    _ = &mut rotate, if count > 32 => { break; },
                                    changed = catalogue_updates.changed() => {
                                        if changed.is_err() { return; }
                                        let current = catalogue_updates.borrow_and_update().clone();
                                        match current {
                                            Some(Ok(current)) => {
                                                if catalogue_changed(&catalogue_baseline, &current) {
                                                    let relevant = |entries: &[ProcessInfo]| entries.iter().filter(|process| process.catalogue.is_none() || selected_target == Some(Target {route, session:process.session_id})).cloned().collect::<Vec<_>>();
                                                    let retire = catalogue_roster_changed(&relevant(&catalogue_baseline), &relevant(&current));
                                                    if sender.send(Update::Catalogue { route, processes: current.clone() }).await.is_err() { return; }
                                                    catalogue_baseline = current;
                                                    if retire { break; }
                                                }
                                            }
                                            Some(Err(error)) if sender.send(Update::RouteError { route, error: crate::process_client::safe(&error) }).await.is_err() => { return; }
                                            _ => {}
                                        }
                                    },
                                    changed = selected.changed() => { if changed.is_err() { return; } break; },
                                    event = events.next() => {
                                        let Some(event) = event else { ended_unexpectedly = true; break; };
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
                                                }) {
                                                    let key = (process.session_id, process.incarnation);
                                                    // Older peers send v1 invalidations. Never interpret
                                                    // an unnegotiated result as a typed mutation.
                                                    let page = &event.result;
                                                    let v2 = page.get("projection").and_then(serde_json::Value::as_str) == Some("public-v2");
                                                    if v2 {
                                                        if page.get("replay_gap").and_then(serde_json::Value::as_bool) == Some(true) {
                                                            if let Some(cursor) = refresh(&client, route, process, &sender).await {
                                                                cursors.insert(key, cursor);
                                                            } else { cursors.remove(&key); }
                                                            break;
                                                        }
                                                        let Some(entries) = page.get("events").and_then(serde_json::Value::as_array) else {
                                                            cursors.remove(&key);
                                                            break;
                                                        };
                                                        let mut failed = false;
                                                        for entry in entries {
                                                            let Some(cursor) = entry.get("cursor").and_then(serde_json::Value::as_u64) else { failed = true; break };
                                                            if cursor <= cursors.get(&key).copied().unwrap_or(0) { continue }
                                                            if entry.get("session_id").and_then(serde_json::Value::as_str) != Some(process.session_id.to_string().as_str()) {
                                                                failed = true; break;
                                                            }
                                                            let (applied, received) = tokio::sync::oneshot::channel();
                                                            if sender.send(Update::Event { target: Target { route, session: process.session_id }, incarnation: process.incarnation, event: entry.clone(), applied }).await.is_err() { return }
                                                            if received.await != Ok(true) { failed = true; break; }
                                                            cursors.insert(key, cursor);
                                                        }
                                                        if failed {
                                                            cursors.remove(&key);
                                                            break;
                                                        }
                                                    } else if let Some(cursor) = refresh(&client, route, process, &sender).await {
                                                        cursors.insert(key, cursor);
                                                    } else { cursors.remove(&key); }
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
                            if ended_unexpectedly {
                                cursors.clear();
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
                    // Keep successfully applied cursors across planned catalogue probes.
                }
                Err(error) => {
                    let _ = sender
                        .send(Update::RouteUnavailable {
                            route,
                            error: error.to_string(),
                        })
                        .await;
                    // Only repeat the read-only catalogue probe on this same
                    // activation. Explicit disconnect aborts this observer; no
                    // creation, submission, or uncertain command is replayed.
                    route_failures = route_failures.saturating_add(1).min(5);
                    tokio::time::sleep(Duration::from_secs((1 << route_failures).min(30))).await;
                }
            }
        }
    })
}

// Only a changed roster or owner requires retiring subscriptions. Metadata
// changes update the sidebar in place, avoiding normal timed churn.
fn catalogue_roster_changed(old: &[ProcessInfo], current: &[ProcessInfo]) -> bool {
    old.len() != current.len()
        || old.iter().zip(current).any(|(a, b)| {
            a.session_id != b.session_id
                || a.incarnation != b.incarnation
                || a.state != b.state
                || a.archive.is_some() != b.archive.is_some()
                || a.deletion.is_some() != b.deletion.is_some()
        })
}

// Ignore observation timestamps; changed identity, lifecycle or summary requires
// updating the sidebar projection.
fn catalogue_changed(old: &[ProcessInfo], current: &[ProcessInfo]) -> bool {
    if old.len() != current.len() {
        return true;
    }
    old.iter().zip(current).any(|(a, b)| {
        a.session_id != b.session_id
            || a.incarnation != b.incarnation
            || a.state != b.state
            || a.name != b.name
            || a.archive.is_some() != b.archive.is_some()
            || a.deletion.is_some() != b.deletion.is_some()
            || a.catalogue.as_ref().and_then(|c| c.summary.as_ref())
                != b.catalogue.as_ref().and_then(|c| c.summary.as_ref())
            || a.catalogue.as_ref().map(|c| (&c.stale, &c.error_code))
                != b.catalogue.as_ref().map(|c| (&c.stale, &c.error_code))
    })
}

pub(super) async fn refresh(
    client: &Client,
    route: Route,
    process: &ProcessInfo,
    sender: &mpsc::Sender<Update>,
) -> Option<u64> {
    refresh_observation(client, route, process, sender)
        .await
        .flatten()
}

// Success without a cursor is hydrated saved metadata, not a failed read.
async fn refresh_observation(
    client: &Client,
    route: Route,
    process: &ProcessInfo,
    sender: &mpsc::Sender<Update>,
) -> Option<Option<u64>> {
    let target = Target {
        route,
        session: process.session_id,
    };
    let result = client
        .initialize_entities(target.session, process.incarnation)
        .await
        .and_then(|scope| {
            let mut value = scope
                .entities
                .get("session:session")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing canonical session entity"))?;
            let mut messages = scope
                .entities
                .iter()
                .filter(|(key, _)| key.starts_with("message:"))
                .map(|(_, value)| value.clone())
                .collect::<Vec<_>>();
            messages.sort_by_key(|message| message["message_index"].as_u64());
            value["messages"] = serde_json::json!(messages);
            value["run"] = scope
                .entities
                .get("run:run")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            value["goal"] = scope
                .entities
                .get("goal:goal")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            value["lifecycle"] = scope
                .entities
                .get("lifecycle:lifecycle")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            value["cleanup"] = scope
                .entities
                .get("resource:cleanup")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            value["turns"] = scope
                .entities
                .get("run:turns")
                .cloned()
                .unwrap_or(serde_json::json!([]));
            value["recovery_notice"] = scope
                .entities
                .get("resource:recovery_notice")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            value["observation_cursor"] = serde_json::json!(scope.cursor);
            if let Some(settings) = scope
                .entities
                .get("settings:settings")
                .and_then(serde_json::Value::as_object)
            {
                for (key, field) in settings {
                    value[key] = field.clone();
                }
            }
            let snapshot: Snapshot = serde_json::from_value(value)?;
            anyhow::ensure!(
                snapshot.session_id == target.session,
                "entity initialization session changed"
            );
            Ok(snapshot)
        })
        .map_err(|error| error.to_string());
    let success = result.is_ok();
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
    if process.state != voyage_protocol::vessel::ProcessState::Live {
        return success.then_some(cursor);
    }
    let inventory = client
        .voyage_observed(
            target.session,
            process.incarnation,
            VoyageCommand::Controls {
                run_id: None,
                section: "terminals".into(),
            },
        )
        .await
        .and_then(|(value, owner)| {
            anyhow::ensure!(
                owner == process.incarnation,
                "Terminal observation owner changed"
            );
            Ok(serde_json::from_value(value)?)
        })
        .map_err(|error| error.to_string());
    let _ = sender
        .send(Update::Terminals {
            target,
            incarnation: process.incarnation,
            result: inventory,
            observed: Instant::now(),
        })
        .await;
    success.then_some(cursor)
}

#[cfg(all(test, unix))]
#[path = "observe_tests.rs"]
mod coverage_tests;

#[cfg(all(test, unix))]
#[path = "observation_stream_journey_tests.rs"]
mod stream_journey_tests;
