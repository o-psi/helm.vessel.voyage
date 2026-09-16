use super::super::socket_support_tests::{Server, voyage};
use super::*;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn refresh_reports_snapshot_identity_cursor_and_terminal_parse_failure_independently() {
    for state in ["live", "suspended", "stopped"] {
        let server = Server::new(|command| {
            let VesselCommand::Voyage(request) = command else { unreachable!() };
            Ok(voyage(command, match &request.command {
                VoyageCommand::Snapshot => json!({"session_id":request.session_id,"model":"synthetic-model","revision":23,"messages":[],"observation_cursor":91}),
                VoyageCommand::Controls { section, run_id } => { assert_eq!(section,"terminals"); assert_eq!(*run_id,None); json!(null) },
                _ => unreachable!(),
            }))
        }).await;
        let process: ProcessInfo = serde_json::from_value(json!({"session_id":server.target.session,"incarnation":server.incarnation,"workspace":"/synthetic","state":state})).unwrap();
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
                assert_eq!(incarnation, server.incarnation);
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
                    assert_eq!(incarnation, server.incarnation);
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
            VesselCommand::Voyage(request) => voyage(command, match &request.command {
                VoyageCommand::Snapshot => json!({"session_id":session,"model":"synthetic-model","revision":7,"messages":[],"observation_cursor":12}),
                _ => json!({}),
            }),
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
