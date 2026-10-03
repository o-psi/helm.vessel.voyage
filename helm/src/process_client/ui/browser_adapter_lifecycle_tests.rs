//! Actual App/Handle lifecycle against scripted loopback sockets. No OS browser
//! opener, Chromium, provider, discovery, real credential or Voyage is executed.
use super::*;
use crate::process_client::loopback_tests::Peer;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;
use voyage_protocol::{
    host_browser::{HostBrowserBinding, HostBrowserOperation as Op},
    vessel::{VesselCommand, VoyageCommand, VoyageRequest},
};

const WAIT: Duration = Duration::from_secs(4);

fn connect(app: &mut App, old: Target, peer: &Peer) -> Target {
    let route = app.clients.insert(peer.client.clone());
    let target = Target {
        route,
        session: old.session,
    };
    let view = app.views.remove(&old).unwrap();
    app.views.insert(target, view);
    app.selected = Some(target);
    target
}
fn add_view(app: &mut App, route: super::super::state::Route) -> Target {
    let target = Target {
        route,
        session: Uuid::new_v4(),
    };
    let mut view = View::new(serde_json::from_value(json!({"session_id":target.session,"incarnation":Uuid::new_v4(),"workspace":"/synthetic-workspace","state":"live","name":"Other synthetic voyage"})).unwrap());
    view.snapshot = Some(
        serde_json::from_value(
            json!({"session_id":target.session,"revision":17,"model":"synthetic-model","messages":[],"decisions":[]}),
        )
        .unwrap(),
    );
    view.draft.insert_str("other retained draft");
    app.views.insert(target, view);
    target
}
async fn opened(app: &mut App, target: Target) -> PathBuf {
    app.browser_command(target, "/browser open").unwrap();
    let handle = app.browsers.get_mut(&target).unwrap();
    let path = tokio::time::timeout(WAIT, async {
        loop {
            let state = handle.state.borrow_and_update().clone();
            assert!(!state.finished, "synthetic adapter ended before readiness");
            if let Some(path) = state.launcher {
                break path;
            }
            handle.state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    // The test observes the real private launcher but never invokes a desktop
    // opener. Mark only this target's launcher observed before poll_browsers.
    app.browser_opened.insert(target);
    path
}
async fn stopped(app: &mut App, target: Target) {
    let handle = app.browsers.get_mut(&target).unwrap();
    tokio::time::timeout(WAIT, async {
        while !handle.state.borrow().finished {
            handle.state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
async fn no_command(peer: &mut Peer) {
    assert!(
        tokio::time::timeout(Duration::from_millis(60), peer.command())
            .await
            .is_err(),
        "unrequested owner/runtime operation crossed synthetic peer"
    );
}
async fn authenticated_browser(
    app: &App,
    target: Target,
) -> impl Fn(&str) -> reqwest::RequestBuilder + use<> {
    let path = app.browsers[&target]
        .state
        .borrow()
        .launcher
        .clone()
        .unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let url = reqwest::Url::parse(
        text.split_once("href=\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(WAIT)
        .build()
        .unwrap();
    let auth: serde_json::Value = http
        .post(url.join("/bootstrap").unwrap())
        .header("origin", url.origin().ascii_serialization())
        .body(url.fragment().unwrap().to_owned())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    move |path| {
        http.post(url.join(path).unwrap())
            .header("origin", url.origin().ascii_serialization())
            .header("authorization", auth["authorization"].as_str().unwrap())
            .header("x-helm-csrf", auth["csrf"].as_str().unwrap())
    }
}
async fn status(app: &mut App, target: Target, peer: &mut Peer) -> HostBrowserBinding {
    let browser = authenticated_browser(app, target).await;
    let request = browser("/operation").json(&Op::Status {});
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    let (id, command) = peer.command().await;
    let incarnation = app.views[&target].process.incarnation;
    assert!(
        matches!(command,VesselCommand::Voyage(VoyageRequest { session_id,incarnation:Some(owner),command:VoyageCommand::HostBrowser { operation:Op::Status {} } }) if session_id==target.session && owner==incarnation)
    );
    let binding = HostBrowserBinding {
        incarnation,
        browser_id: Uuid::new_v4(),
        attachment_id: Uuid::new_v4(),
        tab_id: Uuid::new_v4(),
        document_epoch: 1,
        viewport_epoch: 1,
        controller_epoch: 2,
        capture_epoch: 1,
    };
    peer.voyage_reply(
        id,
        target.session,
        incarnation,
        json!({"status":{"running":true,"mode":"private","binding":binding}}),
    )
    .await;
    let reply = tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(reply.status(), reqwest::StatusCode::OK);
    let value: serde_json::Value = reply.json().await.unwrap();
    assert_eq!(value["result"]["status"]["binding"], json!(binding));
    binding
}
async fn finish(app: &mut App, paths: &[PathBuf]) {
    tokio::time::timeout(WAIT, app.finish_browsers())
        .await
        .unwrap()
        .unwrap();
    assert!(app.browsers.is_empty() && app.browser_retired.is_empty());
    for path in paths {
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }
}

#[tokio::test]
async fn open_guards_require_available_current_target_lifecycle_and_snapshot_before_resources() {
    for variant in 0..5 {
        let (_fixture, mut app, target) = super::super::coverage_support::app();
        match variant {
            0 => app.clients.mark_unavailable(target.route),
            1 => {
                app.views.remove(&target);
            }
            2 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .lifecycle = json!({"archived":true})
            }
            3 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .lifecycle = json!({"deleted":true})
            }
            _ => app.views.get_mut(&target).unwrap().snapshot = None,
        }
        assert!(app.browser_command(target, "/browser open").is_err());
        assert!(
            app.browsers.is_empty()
                && app.browser_retired.is_empty()
                && app.browser_opened.is_empty()
        );
        assert!(
            app.clients[target.route]
                .connection_state()
                .borrow()
                .socket_id
                .is_none()
        );
        if let Some(view) = app.views.get(&target) {
            assert_eq!(view.draft.text, "preserved draft");
            assert!(view.pending.is_none());
        }
    }
}

#[tokio::test]
async fn open_and_repeated_open_reuse_exact_target_with_no_canonical_runtime_effect() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let incarnation = app.views[&target].process.incarnation;
    let path = opened(&mut app, target).await;
    app.browser_command(target, "/browser").unwrap();
    app.browser_command(target, "/browser open").unwrap();
    assert_eq!(app.browsers.len(), 1);
    assert_eq!(
        app.browsers[&target].state.borrow().launcher.as_ref(),
        Some(&path)
    );
    assert!(app.browsers[&target].accepts_incarnation(incarnation));
    let view = &app.views[&target];
    assert_eq!(view.draft.text, "preserved draft");
    assert!(view.pending.is_none());
    assert!(view.snapshot.as_ref().unwrap().messages.is_empty());
    let panel = view.panel.as_ref().unwrap();
    assert!(
        panel.contains(&format!("Voyage: {}", target.session))
            && panel.contains("F6 opens the shared page viewer")
    );
    assert!(!panel.contains("Bearer "));
    no_command(&mut peer).await;
    finish(&mut app, &[path]).await;
    no_command(&mut peer).await;
    assert!(peer.client.connection_state().borrow().socket_id.is_some());
}

#[tokio::test]
async fn actual_browser_panel_swallows_paste_enter_then_esc_retains_draft_and_f6_reuses_viewer() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let path = opened(&mut app, target).await;
    app.input(Event::Paste("/browser detach".into())).unwrap();
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert_eq!(app.browsers.len(), 1);
    assert!(app.browser_retired.is_empty());
    app.input(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
    assert!(app.views[&target].panel.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.input(Event::Key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)))
        .unwrap();
    assert_eq!(
        app.browsers[&target].state.borrow().launcher.as_ref(),
        Some(&path)
    );
    assert!(
        app.views[&target]
            .panel
            .as_ref()
            .unwrap()
            .contains(&target.session.to_string())
    );
    no_command(&mut peer).await;
    finish(&mut app, &[path]).await;
}

#[tokio::test]
async fn background_viewer_updates_only_its_panel_without_switching_target_or_private_drafts() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let path = opened(&mut app, target).await;
    let other = add_view(&mut app, target.route);
    app.selected = Some(other);
    app.views.get_mut(&other).unwrap().panel = Some("# Unrelated tool panel".into());
    let binding = status(&mut app, target, &mut peer).await;
    app.poll_browsers();
    assert_eq!(app.selected, Some(other));
    assert!(
        app.views[&target]
            .panel
            .as_ref()
            .unwrap()
            .contains("control: private")
    );
    assert_eq!(
        app.views[&other].panel.as_deref(),
        Some("# Unrelated tool panel")
    );
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert_eq!(app.views[&other].draft.text, "other retained draft");
    app.browsers[&target].stop();
    let (id, command) = peer.command().await;
    assert!(
        matches!(command,VesselCommand::Voyage(VoyageRequest { session_id,command:VoyageCommand::HostBrowser { operation:Op::Detach { binding:exact,.. } },.. }) if session_id==target.session && exact==binding)
    );
    peer.voyage_reply(
        id,
        target.session,
        binding.incarnation,
        json!({"detached":true}),
    )
    .await;
    finish(&mut app, &[path]).await;
    no_command(&mut peer).await;
}

#[tokio::test]
async fn catalogue_owner_can_lead_unbound_preparation_without_cancel_or_start_replay() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let launcher = opened(&mut app, target).await;
    let browser = authenticated_browser(&app, target).await;
    let original = app.views[&target].process.incarnation;
    let prepared = Uuid::new_v4();
    let command_id = Uuid::new_v4();
    let request = browser("/operation").json(&Op::Start {
        command_id,
        incarnation: original,
        expected_revision: 17,
    });
    let task = tokio::spawn(async move { request.send().await });
    let (id, command) = peer.command().await;
    assert!(matches!(command, VesselCommand::Voyage(VoyageRequest {
        incarnation: Some(owner), command: VoyageCommand::HostBrowser { operation: Op::Start { command_id: exact, .. } }, ..
    }) if owner == original && exact == command_id));
    // Catalogue/event delivery may precede the explicit not-dispatched reply.
    app.views.get_mut(&target).unwrap().process.incarnation = prepared;
    app.poll_browsers();
    assert_eq!(
        browser("/alive").send().await.unwrap().status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert!(!app.browsers[&target].accepts_incarnation(prepared));
    peer.voyage_reply(
        id,
        target.session,
        prepared,
        json!({"status":"prepared","not_dispatched":true}),
    )
    .await;
    let (revision_id, command) = peer.command().await;
    assert!(matches!(command, VesselCommand::Voyage(VoyageRequest {
        incarnation: Some(owner), command: VoyageCommand::Snapshot, ..
    }) if owner == prepared));
    // Even after explicit preparation, the exact revision observation is pending.
    app.poll_browsers();
    assert_eq!(
        browser("/alive").send().await.unwrap().status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert!(!app.browsers[&target].accepts_incarnation(prepared));
    peer.voyage_reply(
        revision_id,
        target.session,
        prepared,
        json!({"revision":23}),
    )
    .await;
    let (start_id, command) = peer.command().await;
    assert!(matches!(command, VesselCommand::Voyage(VoyageRequest {
        incarnation: Some(owner), command: VoyageCommand::HostBrowser { operation: Op::Start { command_id: exact, incarnation, expected_revision:23 } }, ..
    }) if owner == prepared && incarnation == prepared && exact == command_id));
    peer.voyage_reply(
        start_id,
        target.session,
        prepared,
        json!({"status":{"running":false,"binding":null}}),
    )
    .await;
    assert_eq!(
        task.await.unwrap().unwrap().status(),
        reqwest::StatusCode::OK
    );
    app.poll_browsers();
    assert!(app.browsers[&target].accepts_incarnation(prepared));
    assert_eq!(
        browser("/alive").send().await.unwrap().status(),
        reqwest::StatusCode::NO_CONTENT
    );
    no_command(&mut peer).await;
    // The catalogue is still not authority to accept an unrelated replacement.
    app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4();
    app.poll_browsers();
    stopped(&mut app, target).await;
    finish(&mut app, &[launcher]).await;
    no_command(&mut peer).await;
}

#[tokio::test]
async fn pending_unbound_preparation_cannot_preserve_unrelated_or_retired_catalogue_owner() {
    for variant in 0..5 {
        let mut peer = Peer::open().await;
        let (_fixture, mut app, old) = super::super::coverage_support::app();
        let target = connect(&mut app, old, &peer);
        let launcher = opened(&mut app, target).await;
        let browser = authenticated_browser(&app, target).await;
        let original = app.views[&target].process.incarnation;
        let request = browser("/operation").json(&Op::Status {});
        let task = tokio::spawn(async move { request.send().await });
        let (id, _) = peer.command().await;
        app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4();
        if variant < 2 {
            app.poll_browsers();
            assert_eq!(
                browser("/alive").send().await.unwrap().status(),
                reqwest::StatusCode::NO_CONTENT
            );
            let reply_owner = if variant == 0 {
                original
            } else {
                Uuid::new_v4()
            };
            peer.voyage_reply(
                id,
                target.session,
                reply_owner,
                json!({"status":{"running":false,"binding":null}}),
            )
            .await;
            let reply = task.await.unwrap();
            if variant == 0 {
                assert_eq!(reply.unwrap().status(), reqwest::StatusCode::OK);
            }
            app.poll_browsers();
        } else {
            match variant {
                2 => {
                    app.views
                        .get_mut(&target)
                        .unwrap()
                        .snapshot
                        .as_mut()
                        .unwrap()
                        .lifecycle = json!({"deleted":true})
                }
                3 => {
                    app.views
                        .get_mut(&target)
                        .unwrap()
                        .snapshot
                        .as_mut()
                        .unwrap()
                        .lifecycle = json!({"archived":true})
                }
                _ => app.stop_browser_route(target.route.id),
            }
            app.poll_browsers();
            // Retirement is immediate despite a held unbound request.
            assert!(!matches!(browser("/alive").send().await,
                Ok(reply) if reply.status() == reqwest::StatusCode::NO_CONTENT));
            peer.voyage_reply(
                id,
                target.session,
                original,
                json!({"status":{"running":false,"binding":null}}),
            )
            .await;
            let _ = task.await.unwrap();
        }
        stopped(&mut app, target).await;
        finish(&mut app, &[launcher]).await;
        no_command(&mut peer).await;
    }
}

#[tokio::test]
async fn poll_owner_replacement_or_retired_target_stops_only_its_viewer_not_other_voyage() {
    for variant in 0..4 {
        let mut peer = Peer::open().await;
        let (_fixture, mut app, old) = super::super::coverage_support::app();
        let target = connect(&mut app, old, &peer);
        let first = opened(&mut app, target).await;
        let other = add_view(&mut app, target.route);
        let second = opened(&mut app, other).await;
        match variant {
            0 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            1 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .lifecycle = json!({"archived":true})
            }
            2 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .lifecycle = json!({"deleted":true})
            }
            _ => {
                app.views.remove(&target);
            }
        }
        app.poll_browsers();
        stopped(&mut app, target).await;
        assert!(!first.exists());
        assert!(second.exists());
        assert!(!app.browsers[&other].finished());
        assert_eq!(app.views[&other].draft.text, "other retained draft");
        no_command(&mut peer).await;
        finish(&mut app, &[first, second]).await;
        no_command(&mut peer).await;
    }
}

#[tokio::test]
async fn route_viewer_cancellation_stops_only_matching_viewers_and_retains_other_connection() {
    let mut peer = Peer::open().await;
    let mut other_peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let first = opened(&mut app, target).await;
    let route = app.clients.insert(other_peer.client.clone());
    let other = add_view(&mut app, route);
    let second = opened(&mut app, other).await;
    app.stop_browser_route(target.route.id);
    stopped(&mut app, target).await;
    assert!(!first.exists());
    assert!(second.exists());
    assert!(!app.browsers[&other].finished());
    assert!(
        app.clients[other.route]
            .connection_state()
            .borrow()
            .socket_id
            .is_some()
    );
    no_command(&mut peer).await;
    no_command(&mut other_peer).await;
    finish(&mut app, &[first, second]).await;
}

#[tokio::test]
async fn ended_viewer_requires_explicit_detach_completion_before_one_fresh_launcher() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let first = opened(&mut app, target).await;
    app.browsers[&target].stop();
    stopped(&mut app, target).await;
    assert!(app.browser_command(target, "/browser open").is_err());
    assert_eq!(app.browsers.len(), 1);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    app.sender = sender;
    app.browser_command(target, "/browser detach").unwrap();
    assert!(app.browsers.is_empty());
    assert!(!app.browser_opened.contains(&target));
    let update = tokio::time::timeout(WAIT, receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&update,super::super::Update::Browser { target:exact,result:Ok(notice) } if *exact==target && notice=="Viewer detached; host browser remains owned by Voyage")
    );
    app.update(update);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    let second = opened(&mut app, target).await;
    assert_ne!(first, second);
    assert!(!first.exists());
    assert_eq!(app.browsers.len(), 1);
    no_command(&mut peer).await;
    finish(&mut app, &[first, second]).await;
}

#[tokio::test]
async fn viewer_capacity_counts_pending_cleanup_and_admits_only_after_observed_retirement() {
    let mut peer = Peer::open().await;
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    let target = connect(&mut app, old, &peer);
    let mut paths = vec![opened(&mut app, target).await];
    for _ in 0..2 {
        let next = add_view(&mut app, target.route);
        paths.push(opened(&mut app, next).await);
    }
    let fourth = add_view(&mut app, target.route);
    let fourth_path = opened(&mut app, fourth).await;
    paths.push(fourth_path.clone());
    let binding = status(&mut app, fourth, &mut peer).await;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    app.sender = sender;
    app.browser_command(fourth, "/browser detach").unwrap();
    let (detach_id, command) = peer.command().await;
    let VesselCommand::Voyage(VoyageRequest {
        session_id,
        incarnation,
        command:
            VoyageCommand::HostBrowser {
                operation:
                    Op::Detach {
                        command_id,
                        binding: exact,
                    },
            },
    }) = command
    else {
        panic!("only the fourth viewer's exact Detach is allowed");
    };
    assert_eq!(session_id, fourth.session);
    assert_eq!(incarnation, Some(binding.incarnation));
    assert_eq!(exact, binding);
    assert!(!command_id.is_nil());
    assert_eq!(app.browsers.len(), 3);
    assert!(!app.browsers.contains_key(&fourth));
    assert_eq!(app.browser_retired.len(), 1);
    assert!(!app.browser_retired[0].is_finished());
    assert!(fourth_path.exists());
    assert!(app.browsers.values().all(|handle| !handle.finished()));
    let fifth = add_view(&mut app, target.route);
    assert!(app.browser_command(fifth, "/browser open").is_err());
    assert_eq!(app.browsers.len(), 3);
    assert!(app.views[&fifth].panel.is_none());
    assert_eq!(app.views[&fifth].draft.text, "other retained draft");
    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    no_command(&mut peer).await;
    // Release the observed original Detach only after capacity refusal. A local
    // cancellation request alone has not freed this viewer's resource slot.
    peer.voyage_reply(
        detach_id,
        fourth.session,
        binding.incarnation,
        json!({"detached":true}),
    )
    .await;
    let update = tokio::time::timeout(WAIT, receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&update, super::super::Update::Browser { target:exact,result:Ok(notice) } if *exact==fourth && notice=="Viewer detached; host browser remains owned by Voyage")
    );
    app.update(update);
    tokio::time::timeout(WAIT, async {
        while !app.browser_retired.last().unwrap().is_finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!fourth_path.exists());
    assert!(!fourth_path.parent().unwrap().exists());
    assert_eq!(app.views[&fourth].draft.text, "other retained draft");
    paths.push(opened(&mut app, fifth).await);
    let sixth = add_view(&mut app, target.route);
    assert!(app.browser_command(sixth, "/browser open").is_err());
    assert_eq!(app.browsers.len(), 4);
    assert!(app.views[&sixth].panel.is_none());
    assert_eq!(app.views[&sixth].draft.text, "other retained draft");
    no_command(&mut peer).await;
    finish(&mut app, &paths).await;
    no_command(&mut peer).await;
}

#[tokio::test]
async fn invalid_relative_opener_path_retires_without_desktop_execution_or_draft_change() {
    let (_fixture, mut app, target) = super::super::coverage_support::app();
    app.open_browser_launcher(PathBuf::from("relative-untrusted-launcher"));
    finish(&mut app, &[]).await;
    assert!(super::super::account_test_support::browsers().is_empty());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(
        app.clients[target.route]
            .connection_state()
            .borrow()
            .socket_id
            .is_none()
    );
}
