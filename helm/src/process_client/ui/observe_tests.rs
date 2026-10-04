use super::super::socket_support_tests::Server;
use super::*;
use serde_json::json;
use uuid::Uuid;

fn entity_page(
    generation: Uuid,
    session: Uuid,
    incarnation: Uuid,
    revision: u64,
    cursor: u64,
    model: &str,
) -> serde_json::Value {
    let fence = json!({"generation":generation,"session_id":session,"incarnation":incarnation});
    json!({"version":3,"revision":revision,"cursor":cursor,"next_offset":1,"has_more":false,"events":[{"kind":"begin","fence":fence,"cursor":cursor},{"kind":"entity","fence":fence,"sequence":0,"entity_kind":"session","entity_id":"session","value":{"session_id":session,"model":model,"revision":revision,"total_messages":0}},{"kind":"complete","fence":fence,"sequence":1,"cursor":cursor}]})
}

#[tokio::test]
async fn refresh_reports_snapshot_identity_cursor_and_terminal_parse_failure_independently() {
    for state in ["live", "suspended", "stopped"] {
        let incarnation = Uuid::new_v4();
        let server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else {
                unreachable!()
            };
            let result = match &request.command {
                VoyageCommand::InitializeEntities { generation, .. } => entity_page(
                    *generation,
                    request.session_id,
                    incarnation,
                    23,
                    91,
                    "synthetic-model",
                ),
                VoyageCommand::Controls { section, run_id } => {
                    assert_eq!(section, "terminals");
                    assert_eq!(*run_id, None);
                    json!(null)
                }
                _ => unreachable!(),
            };
            Ok(json!({"session_id":request.session_id,"incarnation":incarnation,"result":result}))
        })
        .await;
        let process: ProcessInfo = serde_json::from_value(json!({"session_id":server.target.session,"incarnation":incarnation,"workspace":"/synthetic","state":state})).unwrap();
        let (sender, mut receiver) = mpsc::channel(8);
        let cursor = refresh(&server.client, server.target.route, &process, &sender).await;
        assert_eq!(cursor, Some(91));
        match receiver.recv().await.unwrap() {
            Update::Snapshot {
                target,
                incarnation,
                result,
            } => {
                assert_eq!(target, server.target);
                assert_eq!(incarnation, process.incarnation);
                assert_eq!(result.unwrap().revision, 23);
            }
            _ => panic!("snapshot must arrive first"),
        }
        if state == "live" {
            match receiver.recv().await.unwrap() {
                Update::Terminals {
                    target,
                    incarnation,
                    result,
                    ..
                } => {
                    assert_eq!(target, server.target);
                    assert_eq!(incarnation, process.incarnation);
                    assert!(result.is_err());
                }
                _ => panic!("live process needs inventory"),
            }
        }
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn refresh_keeps_snapshot_failures_explicit_and_continues_live_inventory() {
    let server = Server::new(|_| Err("synthetic refusal".into())).await;
    let process = serde_json::from_value(json!({"session_id":server.target.session,"incarnation":server.incarnation,"workspace":"/synthetic","state":"live"})).unwrap();
    let (sender, mut receiver) = mpsc::channel(8);
    assert_eq!(
        refresh(&server.client, server.target.route, &process, &sender).await,
        None
    );
    match receiver.recv().await.unwrap() {
        Update::Snapshot { result, .. } => assert!(result.is_err()),
        _ => panic!("snapshot"),
    }
    match receiver.recv().await.unwrap() {
        Update::Terminals { result, .. } => assert!(result.is_err()),
        _ => panic!("inventory"),
    }
}

#[tokio::test]
async fn observer_publishes_catalogue_and_hydrated_selection_without_blocking_input() {
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let server = Server::new(move |command| {
        Ok(match command {
            VesselCommand::Catalogue => json!([{"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"suspended"}]),
            VesselCommand::Capabilities => json!({"features":[]}),
            VesselCommand::Voyage(request) => json!({"session_id":request.session_id,"incarnation":incarnation,"result":match &request.command {
                VoyageCommand::InitializeEntities { generation, .. } => entity_page(*generation,session,incarnation,7,12,"synthetic-model"),
                _ => json!({}),
            }}),
            _ => json!({}),
        })
    }).await;
    let target = Target {
        route: server.target.route,
        session,
    };
    let (sender, mut receiver) = mpsc::channel(32);
    let (_selection, selected) = tokio::sync::watch::channel(Some(target));
    let job = spawn(server.client.clone(), target.route, sender, selected);
    let observed = tokio::time::timeout(Duration::from_secs(3), async {
        let mut catalogue = false;
        let mut snapshot = false;
        while !(catalogue && snapshot) {
            match receiver.recv().await.unwrap() {
                Update::Catalogue { route, processes } => {
                    assert_eq!(route, target.route);
                    assert_eq!(processes.len(), 1);
                    catalogue = true;
                }
                Update::Snapshot {
                    target: actual,
                    result,
                    ..
                } => {
                    assert_eq!(actual, target);
                    assert_eq!(result.unwrap().revision, 7);
                    snapshot = true;
                }
                _ => {}
            }
        }
    })
    .await;
    job.abort();
    let _ = job.await;
    observed.unwrap();
}

#[test]
fn catalogue_probe_ignores_refresh_time_but_detects_owner_and_process_transition() {
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let mut original: ProcessInfo = serde_json::from_value(json!({
        "session_id":session,"incarnation":incarnation,"workspace":"/synthetic",
        "state":"live","catalogue":{"summary":null,"observed_at_ms":1,"stale":false,"error_code":null}
    })).unwrap();
    let mut next = original.clone();
    next.catalogue.as_mut().unwrap().observed_at_ms = Some(2);
    assert!(!catalogue_changed(&[original.clone()], &[next.clone()]));
    assert!(!catalogue_roster_changed(
        &[original.clone()],
        &[next.clone()]
    ));
    next.state = voyage_protocol::process::ProcessState::Suspended;
    assert!(catalogue_changed(&[original.clone()], &[next.clone()]));
    assert!(catalogue_roster_changed(
        &[original.clone()],
        &[next.clone()]
    ));
    next = original.clone();
    next.incarnation = Uuid::new_v4();
    assert!(catalogue_changed(&[original.clone()], &[next]));
    original.catalogue.as_mut().unwrap().stale = true;
    assert!(catalogue_changed(&[original], &[]));
}

#[tokio::test]
async fn catalogue_watcher_checkpoints_before_hydration_and_recovers_gaps() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let stage = Arc::new(AtomicUsize::new(0));
    let server_stage = stage.clone();
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let mut server = Server::new(move |command| {
        let stage = server_stage.load(Ordering::SeqCst);
        let entry = |name| json!({"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"suspended","name":name});
        Ok(match command {
            VesselCommand::Capabilities => json!({"features":["catalogue_changes"]}),
            VesselCommand::Catalogue => json!([entry(if stage < 2 {"Initial"} else {"Recovered"})]),
            VesselCommand::CatalogueChanges { after:None,.. } => json!({"cursor":10,"latest_cursor":10,"has_more":false,"replay_gap":true,"entries":[]}),
            VesselCommand::CatalogueChanges { after:Some(after),.. } => {
                let (cursor,gap,entries) = if stage == 1 && *after < 11 {(11,false,vec![entry("Renamed")])}
                    else if stage == 2 && *after < 20 {(20,true,vec![])} else {(*after,false,vec![])};
                json!({"cursor":cursor,"latest_cursor":cursor,"has_more":false,"replay_gap":gap,"entries":entries})
            }
            _ => panic!("unexpected catalogue observer command"),
        })
    }).await;
    let (observer, mut updates) = catalogue_watch::spawn(server.client.clone());
    tokio::time::timeout(Duration::from_secs(3), updates.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updates
            .borrow_and_update()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()[0]
            .name
            .as_deref(),
        Some("Initial")
    );
    assert!(matches!(
        server.requests.recv().await,
        Some(VesselCommand::Capabilities)
    ));
    assert!(matches!(
        server.requests.recv().await,
        Some(VesselCommand::CatalogueChanges { after: None, .. })
    ));
    assert!(matches!(
        server.requests.recv().await,
        Some(VesselCommand::Catalogue)
    ));
    stage.store(1, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), updates.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updates
            .borrow_and_update()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()[0]
            .name
            .as_deref(),
        Some("Renamed")
    );
    stage.store(2, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), updates.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updates
            .borrow_and_update()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()[0]
            .name
            .as_deref(),
        Some("Recovered")
    );
    drop(observer);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), updates.changed())
            .await
            .unwrap()
            .is_err()
    );
}

// The legacy (no catalogue metadata/event stream) path used to observe every
// saved voyage every 750 ms, twice during its first fallback round.
#[tokio::test]
async fn fallback_hydrates_257_saved_voyages_once_even_without_snapshot_cursors() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let entries = (0..257)
        .map(|_| (Uuid::new_v4(), Uuid::new_v4()))
        .collect::<Vec<_>>();
    let snapshots = Arc::new(AtomicUsize::new(0));
    let count = snapshots.clone();
    let server = Server::new(move |command| Ok(match command {
        VesselCommand::Capabilities => json!({"features":[]}),
        VesselCommand::Catalogue => json!(entries.iter().map(|(session, incarnation)| json!({"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"suspended"})).collect::<Vec<_>>()),
        VesselCommand::Voyage(request) => {
            let incarnation = entries.iter().find(|(id,_)| *id == request.session_id).unwrap().1;
            if let VoyageCommand::InitializeEntities { generation, .. } = request.command { count.fetch_add(1, Ordering::SeqCst); return Ok(json!({"session_id":request.session_id,"incarnation":incarnation,"result":entity_page(generation,request.session_id,incarnation,1,1,"synthetic")})); }
            json!({"session_id":request.session_id,"incarnation":incarnation,"result":{"session_id":request.session_id,"model":"synthetic","revision":1,"messages":[]}})
        },
        _ => json!({}),
    })).await;
    let (sender, mut receiver) = mpsc::channel(1024);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let (_selection, selected) = tokio::sync::watch::channel(None);
    let job = spawn(server.client.clone(), server.target.route, sender, selected);
    tokio::time::timeout(Duration::from_secs(10), async {
        while snapshots.load(Ordering::SeqCst) < 257 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    job.abort();
    let _ = job.await;
    drain.abort();
    let _ = drain.await;
    assert_eq!(snapshots.load(Ordering::SeqCst), 257);
}

#[tokio::test]
async fn fallback_observation_failures_obey_backoff_instead_of_double_polling() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let snapshots = Arc::new(AtomicUsize::new(0));
    let count = snapshots.clone();
    let server = Server::new(move |command| match command {
        VesselCommand::Capabilities => Ok(json!({"features":[]})),
        VesselCommand::Catalogue => Ok(json!([{"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"suspended"}])),
        VesselCommand::Voyage(request) if matches!(request.command, VoyageCommand::InitializeEntities { .. }) => { count.fetch_add(1, Ordering::SeqCst); Err("unavailable".into()) },
        _ => Ok(json!({})),
    }).await;
    let (sender, mut receiver) = mpsc::channel(32);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let (_selection, selected) = tokio::sync::watch::channel(None);
    let job = spawn(server.client.clone(), server.target.route, sender, selected);
    tokio::time::sleep(Duration::from_millis(1700)).await;
    job.abort();
    let _ = job.await;
    drain.abort();
    let _ = drain.await;
    assert_eq!(snapshots.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fallback_invalidates_saved_hydration_on_rename_and_owner_transition() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let session = Uuid::new_v4();
    let owners = [Uuid::new_v4(), Uuid::new_v4()];
    let phase = Arc::new(AtomicUsize::new(0));
    let current = phase.clone();
    let snapshots = Arc::new(AtomicUsize::new(0));
    let count = snapshots.clone();
    let server = Server::new(move |command| {
        let stage = current.load(Ordering::SeqCst);
        let incarnation = owners[usize::from(stage == 2)];
        Ok(match command {
            VesselCommand::Capabilities => json!({"features":[]}),
            VesselCommand::Catalogue => json!([{"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"suspended","name":if stage == 0 {"old"} else {"new"}}]),
            VesselCommand::Voyage(request) => {
                if let VoyageCommand::InitializeEntities { generation, .. } = request.command { count.fetch_add(1, Ordering::SeqCst); return Ok(json!({"session_id":session,"incarnation":incarnation,"result":entity_page(generation,session,incarnation,(stage+1) as u64,(stage+1) as u64,"synthetic")})); }
                json!({"session_id":session,"incarnation":incarnation,"result":{"session_id":session,"model":"synthetic","revision":stage+1,"messages":[],"observation_cursor":stage+1}})
            },
            _ => json!({}),
        })
    }).await;
    let (sender, mut receiver) = mpsc::channel(32);
    let drain = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
    let (_selection, selected) = tokio::sync::watch::channel(None);
    let job = spawn(server.client.clone(), server.target.route, sender, selected);
    for stage in 0..3 {
        phase.store(stage, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(15), async {
            while snapshots.load(Ordering::SeqCst) < stage + 1 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
    job.abort();
    let _ = job.await;
    drain.abort();
    let _ = drain.await;
    assert_eq!(snapshots.load(Ordering::SeqCst), 3);
}
