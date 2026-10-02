//! Real public socket/observer/App acknowledgement journeys, scripted peers only.
//! No executing Vessel/Voyage, provider, browser, credential or native host.
use super::super::{
    account_test_support::Fixture, accounts::app_tests, routes::Routes, state::View,
};
use super::*;
use crate::process_client::loopback_tests::{Peer, response, send};
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use voyage_protocol::{
    duplex::{ClientFrame, ServerFrame},
    vessel::{VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VoyageRequest},
};

const WAIT: Duration = Duration::from_secs(6);
fn process(id: uuid::Uuid, owner: uuid::Uuid) -> ProcessInfo {
    serde_json::from_value(json!({"session_id":id,"incarnation":owner,"workspace":"/synthetic","state":"live","name":"Observer fixture"})).unwrap()
}
fn snapshot(id: uuid::Uuid, cursor: u64) -> Value {
    json!({"session_id":id,"revision":17,"model":"observer-model","observation_cursor":cursor,"total_messages":1,"messages":[{"message_index":0,"role":"user","content":"Canonical observer Ω"}],"decisions":[]})
}
fn entry(id: uuid::Uuid, cursor: u64, kind: &str, payload: Value) -> Value {
    json!({"session_id":id,"cursor":cursor,"revision":18,"kind":kind,"payload":payload})
}
#[derive(Default)]
struct State {
    processes: Vec<ProcessInfo>,
    snapshots: HashMap<uuid::Uuid, Value>,
    reads: Vec<VesselCommand>,
    subscriptions: Vec<(uuid::Uuid, VesselEventRequest)>,
    active: HashSet<uuid::Uuid>,
    retired: Vec<uuid::Uuid>,
    held: Vec<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    hold_snapshots: bool,
}
enum Signal {
    Event(uuid::Uuid, VesselEvent),
    Release,
}
struct Wire {
    client: Client,
    state: Arc<std::sync::Mutex<State>>,
    signals: mpsc::Sender<Signal>,
    job: Option<tokio::task::JoinHandle<()>>,
}
impl Wire {
    async fn new(count: usize) -> Self {
        let peer = Peer::open().await;
        let client = peer.client.clone();
        let processes: Vec<_> = (0..count)
            .map(|_| process(uuid::Uuid::new_v4(), uuid::Uuid::new_v4()))
            .collect();
        let snapshots = processes
            .iter()
            .map(|p| (p.session_id, snapshot(p.session_id, 10)))
            .collect();
        let state = Arc::new(std::sync::Mutex::new(State {
            processes,
            snapshots,
            ..Default::default()
        }));
        let (signals, mut receiver) = mpsc::channel(64);
        let held = state.clone();
        let job = tokio::spawn(async move {
            let mut peer = peer;
            loop {
                tokio::select! {
                    signal=receiver.recv()=>match signal {
                        Some(Signal::Event(id,event))=>{ assert!(held.lock().unwrap().active.contains(&id));send(&mut peer.socket,ServerFrame::Event {subscription_id:id,event}).await; }
                        Some(Signal::Release)=>{
                            let pending=std::mem::take(&mut held.lock().unwrap().held);
                            held.lock().unwrap().hold_snapshots=false;
                            for (id,session,owner) in pending {
                                let value=held.lock().unwrap().snapshots[&session].clone();peer.voyage_reply(id,session,owner,value).await;
                            }
                        }
                        None=>break,
                    },
                    frame=peer.socket.next()=>{
                        let Some(Ok(frame))=frame else {break;};
                        let frame=match frame {
                            tokio_tungstenite::tungstenite::Message::Text(text)=>serde_json::from_str::<ClientFrame>(&text).unwrap(),
                            tokio_tungstenite::tungstenite::Message::Ping(bytes)=>{peer.socket.send(tokio_tungstenite::tungstenite::Message::Pong(bytes)).await.unwrap();continue;},
                            tokio_tungstenite::tungstenite::Message::Pong(_)=>continue,
                            tokio_tungstenite::tungstenite::Message::Close(_)=>break,
                            _=>panic!("only public text metadata frames are permitted"),
                        };
                        match frame {
                            ClientFrame::Command {request_id,request}=>{
                                let result={
                                    let mut state=held.lock().unwrap();assert!(state.reads.len()<4096);state.reads.push(request.command.clone());
                                    match &request.command {
                                        VesselCommand::Capabilities=>Some(json!({"features":["duplex_socket"]})),
                                        VesselCommand::Catalogue=>Some(serde_json::to_value(&state.processes).unwrap()),
                                        VesselCommand::Notifications {operation}=>{assert!(matches!(operation,voyage_protocol::notifications::NotificationOperation::Attention));Some(json!({"available":0}))},
                                        VesselCommand::Voyage(VoyageRequest {session_id,incarnation,command})=>{
                                            assert!(incarnation.is_none(),"Snapshot/Controls keep their session-named wire contract");
                                            let owner=state.processes.iter().find(|p|p.session_id==*session_id).unwrap().incarnation;
                                            let value=match command {
                                                VoyageCommand::Snapshot if state.hold_snapshots=>{state.held.push((request_id,*session_id,owner));None},
                                                VoyageCommand::Snapshot=>Some(state.snapshots[session_id].clone()),
                                                VoyageCommand::Controls {run_id,section}=>{assert!(run_id.is_none());assert_eq!(section,"terminals");Some(json!({"run_id":null,"value":[]}))},
                                                _=>panic!("observer cannot emit a mutative or unrelated Voyage command"),
                                            };
                                            value.map(|value|json!({"session_id":session_id,"incarnation":owner,"result":value}))
                                        }
                                        _=>panic!("observer cannot emit an unrelated or mutative Vessel command"),
                                    }
                                };
                                if let Some(result)=result {send(&mut peer.socket,ServerFrame::Reply {request_id,response:response(result)}).await;}
                            }
                            ClientFrame::Subscribe {request_id,request}=>{
                                assert_eq!(request.protocol,VESSEL_API_VERSION);assert!(!request.subscriptions.is_empty()&&request.subscriptions.len()<=32);
                                {let mut state=held.lock().unwrap();assert!(state.active.insert(request_id));state.subscriptions.push((request_id,request));}
                                send(&mut peer.socket,ServerFrame::Subscribed {request_id}).await;
                            }
                            ClientFrame::Unsubscribe {subscription_id}=>{let mut state=held.lock().unwrap();assert!(state.active.remove(&subscription_id),"only one owned unsubscribe");state.retired.push(subscription_id);},
                            ClientFrame::ReverseReply {..}=>panic!("no reverse work belongs to metadata observation"),
                        }
                    }
                }
            }
        });
        Self {
            client,
            state,
            signals,
            job: Some(job),
        }
    }
    fn catalogue(&self) -> Vec<ProcessInfo> {
        self.state.lock().unwrap().processes.clone()
    }
    fn subscriptions(&self) -> Vec<(uuid::Uuid, VesselEventRequest)> {
        self.state.lock().unwrap().subscriptions.clone()
    }
    fn snapshot_reads(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .reads
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    VesselCommand::Voyage(VoyageRequest {
                        command: VoyageCommand::Snapshot,
                        ..
                    })
                )
            })
            .count()
    }
    async fn event(&self, id: uuid::Uuid, process: &ProcessInfo, result: Value) {
        self.signals
            .send(Signal::Event(
                id,
                VesselEvent {
                    protocol: VESSEL_API_VERSION,
                    session_id: process.session_id,
                    incarnation: process.incarnation,
                    result,
                    error: None,
                    outcome_unknown: false,
                },
            ))
            .await
            .unwrap();
    }
    async fn finish(&mut self) {
        self.client.disconnect();
        tokio::time::timeout(WAIT, self.job.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(self.client.connection_state().borrow().socket_id.is_none());
    }
}
impl Drop for Wire {
    fn drop(&mut self) {
        self.client.disconnect();
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}

struct Journey {
    wire: Wire,
    app: super::super::App,
    target: Target,
    selected: tokio::sync::watch::Sender<Option<Target>>,
    updates: mpsc::Receiver<Update>,
    held: VecDeque<Update>,
    hold_events: bool,
    observer: Option<tokio::task::JoinHandle<()>>,
    _fixture: Fixture,
}
impl Journey {
    async fn new(count: usize) -> Self {
        let fixture = Fixture::new();
        let wire = Wire::new(count).await;
        let mut app = app_tests::app(fixture.0.path());
        app.clients = Routes::new(vec![wire.client.clone()]);
        let route = super::super::state::Route::of(&wire.client);
        for process in wire.catalogue() {
            let target = Target {
                route,
                session: process.session_id,
            };
            let mut view = View::new(process);
            view.snapshot = Some(serde_json::from_value(snapshot(target.session, 10)).unwrap());
            view.draft.insert_str("retained observer draft Δ");
            app.views.insert(target, view);
        }
        let target = Target {
            route,
            session: wire.catalogue()[0].session_id,
        };
        app.selected = Some(target);
        let (sender, updates) = mpsc::channel(64);
        app.sender = sender.clone();
        let (selected, receiver) = tokio::sync::watch::channel(Some(target));
        let observer = spawn(wire.client.clone(), route, sender, receiver);
        Self {
            wire,
            app,
            target,
            selected,
            updates,
            held: VecDeque::new(),
            hold_events: false,
            observer: Some(observer),
            _fixture: fixture,
        }
    }
    async fn until(&mut self, predicate: impl Fn(&Self) -> bool) {
        tokio::time::timeout(WAIT,async {
            loop {
                if predicate(self){break;}
                tokio::select! {
                    update=self.updates.recv()=>{
                        let update=update.expect("owned observer update channel");
                        if self.hold_events&&matches!(&update,Update::Event {..}){self.held.push_back(update);}else{self.app.update(update);}
                    }
                    _=tokio::time::sleep(Duration::from_millis(10))=>{},
                }
            }
        }).await.expect("bounded owned observation stage");
    }
    async fn boot(&mut self) {
        self.until(|j| !j.wire.subscriptions().is_empty()).await;
    }
    fn preserved(&self) {
        for target in self
            .app
            .views
            .keys()
            .filter(|t| t.route == self.target.route)
        {
            let view = &self.app.views[target];
            assert_eq!(view.draft.text, "retained observer draft Δ");
            assert!(view.pending.is_none());
            if let Some(snapshot) = &view.snapshot {
                assert_eq!(snapshot.messages[0].message_index, 0);
                assert_eq!(snapshot.messages[0].role, "user");
                assert_eq!(snapshot.messages[0].content, "Canonical observer Ω");
            }
        }
        assert!(
            self.app.command_checks.is_empty()
                && self.app.first_send_checks.is_empty()
                && self.app.route_tasks.is_empty()
                && self.app.browsers.is_empty()
        );
        assert!(super::super::account_test_support::browsers().is_empty());
    }
    async fn finish(mut self) {
        self.preserved();
        self.shutdown().await;
    }
    async fn shutdown(&mut self) {
        let observer = self.observer.take().unwrap();
        observer.abort();
        assert!(
            tokio::time::timeout(WAIT, observer)
                .await
                .unwrap()
                .unwrap_err()
                .is_cancelled()
        );
        // A read-only actor barrier observes any subscribe acknowledgement that
        // raced task cancellation; deferred retirement cannot hide after an
        // early empty-set observation.
        tokio::time::timeout(WAIT, self.wire.client.request(VesselCommand::Capabilities))
            .await
            .unwrap()
            .unwrap();
        // Stream guards emit actual socket-local unsubscribe; no creation,
        // cancellation, input or lifecycle effect is inferred from retirement.
        tokio::time::timeout(WAIT, async {
            while !self.wire.state.lock().unwrap().active.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        self.wire.finish().await;
    }
}

#[tokio::test]
async fn foreign_inner_snapshot_returns_no_cursor_and_preserves_app_before_valid_refresh() {
    let mut peer = Peer::open().await;
    let session = uuid::Uuid::new_v4();
    let owner = uuid::Uuid::new_v4();
    let foreign = uuid::Uuid::new_v4();
    let command = uuid::Uuid::new_v4();
    let process = process(session, owner);
    let (_fixture, mut app, old) = super::super::coverage_support::app();
    app.clients = Routes::new(vec![peer.client.clone()]);
    let target = Target {
        route: super::super::state::Route::of(&peer.client),
        session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process = process.clone();
    view.snapshot = Some(serde_json::from_value(snapshot(session, 10)).unwrap());
    view.pending = Some(super::super::state::Pending {
        account_host: None,
        command_id: command,
        incarnation: owner,
        draft: "exact frozen pending text".into(),
        preserve_draft: true,
        original: None,
        receipt_only: true,
    });
    let before = view.snapshot.clone().unwrap();
    app.views.insert(target, view);
    app.selected = Some(target);
    for rejected in [true, false] {
        let (sender, mut receiver) = mpsc::channel(8);
        let client = peer.client.clone();
        let info = process.clone();
        let task =
            tokio::spawn(async move { refresh(&client, target.route, &info, &sender).await });
        let (id, request) = peer.command().await;
        match request {
            VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation,
                command: VoyageCommand::Snapshot,
            }) => {
                assert_eq!(session_id, session);
                assert!(incarnation.is_none());
            }
            _ => panic!("only the expected session-named snapshot read"),
        }
        peer.voyage_reply(
            id,
            session,
            owner,
            if rejected {
                snapshot(foreign, 999)
            } else {
                snapshot(session, 21)
            },
        )
        .await;
        let (id, request) = peer.command().await;
        match request {
            VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation,
                command: VoyageCommand::Controls { run_id, section },
            }) => {
                assert_eq!(session_id, session);
                assert!(incarnation.is_none() && run_id.is_none());
                assert_eq!(section, "terminals");
            }
            _ => panic!("only the expected terminal inventory read"),
        }
        peer.voyage_reply(id, session, owner, json!({"run_id":null,"value":[]}))
            .await;
        assert_eq!(
            tokio::time::timeout(WAIT, task).await.unwrap().unwrap(),
            if rejected { None } else { Some(21) }
        );
        let update = receiver.recv().await.unwrap();
        match &update {
            Update::Snapshot { result, .. } if rejected => {
                assert_eq!(
                    result.as_ref().as_ref().err().unwrap(),
                    "Snapshot observation session changed"
                );
            }
            Update::Snapshot { result, .. } => assert!(result.is_ok()),
            _ => panic!("snapshot precedes inventory"),
        }
        app.update(update);
        if rejected {
            assert!(app.views[&target].snapshot.as_ref() == Some(&before));
            assert_eq!(
                app.views[&target].error.as_deref(),
                Some("Snapshot observation session changed")
            );
        } else {
            assert!(
                app.views[&target].snapshot.as_ref()
                    == Some(&serde_json::from_value::<Snapshot>(snapshot(session, 21)).unwrap())
            );
            assert!(app.views[&target].error.is_none());
        }
        let update = receiver.recv().await.unwrap();
        assert!(matches!(&update, Update::Terminals { result, .. } if result.is_ok()));
        app.update(update);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        let pending = app.views[&target].pending.as_ref().unwrap();
        assert_eq!(pending.command_id, command);
        assert_eq!(pending.incarnation, owner);
        assert_eq!(pending.draft, "exact frozen pending text");
        assert!(
            pending.account_host.is_none()
                && pending.original.is_none()
                && pending.preserve_draft
                && pending.receipt_only
        );
        assert!(
            app.command_checks.is_empty()
                && app.first_send_checks.is_empty()
                && app.route_tasks.is_empty()
                && app.browsers.is_empty()
        );
    }
    peer.client.disconnect();
    tokio::time::timeout(WAIT, async {
        while peer.client.connection_state().borrow().socket_id.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn foreign_inner_snapshot_cannot_seed_subscription_and_valid_resnapshot_recovers() {
    let mut j = Journey::new(1).await;
    let observer = j.observer.take().unwrap();
    observer.abort();
    assert!(
        tokio::time::timeout(WAIT, observer)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    let p = j.wire.catalogue()[0].clone();
    let command = uuid::Uuid::new_v4();
    let before = j.app.views[&j.target].snapshot.clone().unwrap();
    j.app.views.get_mut(&j.target).unwrap().pending = Some(super::super::state::Pending {
        account_host: None,
        command_id: command,
        incarnation: p.incarnation,
        draft: "exact frozen pending text".into(),
        preserve_draft: true,
        original: None,
        receipt_only: true,
    });
    j.wire
        .state
        .lock()
        .unwrap()
        .snapshots
        .insert(p.session_id, snapshot(uuid::Uuid::new_v4(), 999));
    j.observer = Some(spawn(
        j.wire.client.clone(),
        j.target.route,
        j.app.sender.clone(),
        j.selected.subscribe(),
    ));
    j.until(|j| {
        j.app.views[&j.target].error.as_deref() == Some("Snapshot observation session changed")
    })
    .await;
    assert_eq!(j.wire.snapshot_reads(), 1);
    assert!(j.wire.subscriptions().is_empty());
    assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
    assert_eq!(
        j.app.views[&j.target].draft.text,
        "retained observer draft Δ"
    );
    let pending = j.app.views[&j.target].pending.as_ref().unwrap();
    assert_eq!(pending.command_id, command);
    assert_eq!(pending.incarnation, p.incarnation);
    assert_eq!(pending.draft, "exact frozen pending text");
    assert!(pending.original.is_none() && pending.preserve_draft && pending.receipt_only);
    // Supply a correct later reply. The production retry delay is unchanged;
    // no selection change, injected cursor or mutative retry drives recovery.
    j.wire
        .state
        .lock()
        .unwrap()
        .snapshots
        .insert(p.session_id, snapshot(p.session_id, 21));
    j.until(|j| {
        !j.wire.subscriptions().is_empty()
            && j.app.views[&j.target]
                .snapshot
                .as_ref()
                .unwrap()
                .observation_cursor
                == Some(21)
    })
    .await;
    assert_eq!(j.wire.snapshot_reads(), 2);
    let subscriptions = j.wire.subscriptions();
    assert_eq!(subscriptions.len(), 1);
    assert_eq!(subscriptions[0].1.subscriptions.len(), 1);
    let subscription = &subscriptions[0].1.subscriptions[0];
    assert_eq!(subscription.session_id, p.session_id);
    assert_eq!(subscription.incarnation, p.incarnation);
    assert_eq!(subscription.after, 21);
    assert!(j.app.views[&j.target].error.is_none());
    assert_eq!(
        j.app.views[&j.target].draft.text,
        "retained observer draft Δ"
    );
    let pending = j.app.views[&j.target].pending.as_ref().unwrap();
    assert_eq!(pending.command_id, command);
    assert_eq!(pending.incarnation, p.incarnation);
    assert_eq!(pending.draft, "exact frozen pending text");
    assert!(
        pending.account_host.is_none()
            && pending.original.is_none()
            && pending.preserve_draft
            && pending.receipt_only
    );
    assert_eq!(
        j.app.views[&j.target].snapshot.as_ref().unwrap().messages[0].content,
        "Canonical observer Ω"
    );
    assert!(
        j.app.command_checks.is_empty()
            && j.app.first_send_checks.is_empty()
            && j.app.route_tasks.is_empty()
            && j.app.browsers.is_empty()
    );
    assert!(super::super::account_test_support::browsers().is_empty());
    j.shutdown().await;
}

#[tokio::test]
async fn held_snapshot_and_terminal_reply_owner_changes_cannot_install_old_catalogue_context() {
    for changed_snapshot in [true, false] {
        let mut peer = Peer::open().await;
        let session = uuid::Uuid::new_v4();
        let owner = uuid::Uuid::new_v4();
        let process = process(session, owner);
        let (_fixture, mut app, old) = super::super::coverage_support::app();
        app.clients = Routes::new(vec![peer.client.clone()]);
        let target = Target {
            route: super::super::state::Route::of(&peer.client),
            session,
        };
        let mut view = app.views.remove(&old).unwrap();
        view.process = process.clone();
        view.snapshot = Some(serde_json::from_value(snapshot(session, 10)).unwrap());
        app.views.insert(target, view);
        app.selected = Some(target);
        let (sender, mut receiver) = mpsc::channel(8);
        let client = peer.client.clone();
        let info = process.clone();
        let task =
            tokio::spawn(async move { refresh(&client, target.route, &info, &sender).await });
        let (id, command) = peer.command().await;
        assert!(matches!(
            command,
            VesselCommand::Voyage(VoyageRequest {
                incarnation: None,
                command: VoyageCommand::Snapshot,
                ..
            })
        ));
        peer.voyage_reply(
            id,
            session,
            if changed_snapshot {
                uuid::Uuid::new_v4()
            } else {
                owner
            },
            snapshot(session, 90),
        )
        .await;
        let (id, command) = peer.command().await;
        assert!(matches!(
            command,
            VesselCommand::Voyage(VoyageRequest {
                incarnation: None,
                command: VoyageCommand::Controls { .. },
                ..
            })
        ));
        peer.voyage_reply(
            id,
            session,
            if changed_snapshot {
                owner
            } else {
                uuid::Uuid::new_v4()
            },
            json!({"run_id":null,"value":[]}),
        )
        .await;
        assert_eq!(
            task.await.unwrap(),
            if changed_snapshot { None } else { Some(90) }
        );
        let first = receiver.recv().await.unwrap();
        match &first {
            Update::Snapshot { result, .. } => assert_eq!(result.is_err(), changed_snapshot),
            _ => panic!("snapshot first"),
        };
        app.update(first);
        let second = receiver.recv().await.unwrap();
        match &second {
            Update::Terminals { result, .. } => assert_eq!(result.is_err(), !changed_snapshot),
            _ => panic!("inventory second"),
        };
        app.update(second);
        assert_eq!(app.views[&target].process.incarnation, owner);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert_eq!(
            app.views[&target]
                .snapshot
                .as_ref()
                .unwrap()
                .observation_cursor,
            Some(if changed_snapshot { 10 } else { 90 })
        );
        assert!(
            app.views[&target].pending.is_none()
                && app.command_checks.is_empty()
                && app.route_tasks.is_empty()
        );
        peer.client.disconnect();
    }
}

#[tokio::test]
async fn actual_app_acknowledgement_orders_events_and_resume_cursor_without_duplicate_history_or_effects()
 {
    let mut j = Journey::new(1).await;
    j.boot().await;
    j.hold_events = true;
    let p = j.wire.catalogue()[0].clone();
    let id = j.wire.subscriptions()[0].0;
    let first = entry(
        p.session_id,
        11,
        "message_finalized",
        json!({"message_index":1,"message":{"role":"assistant","content":"Observed once"}}),
    );
    let second = entry(
        p.session_id,
        12,
        "session",
        json!({"name":"Acknowledged metadata"}),
    );
    j.wire
        .event(
            id,
            &p,
            json!({"projection":"public-v2","events":[first.clone(),second]}),
        )
        .await;
    j.until(|j| j.held.len() == 1).await;
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor,
        Some(10)
    );
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .len(),
        1
    );
    j.app.update(j.held.pop_front().unwrap());
    j.until(|j| j.held.len() == 1).await;
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor,
        Some(11)
    );
    j.app.update(j.held.pop_front().unwrap());
    j.hold_events = false;
    j.selected.send(None).unwrap();
    j.until(|j| j.wire.subscriptions().len() >= 2).await;
    assert_eq!(j.wire.subscriptions()[1].1.subscriptions[0].after, 12);
    assert_eq!(j.wire.state.lock().unwrap().retired, vec![id]);
    let next = j.wire.subscriptions()[1].0;
    j.wire
        .event(next, &p, json!({"projection":"public-v2","events":[first,entry(p.session_id,13,"session",json!({"name":"Duplicate passed once"}))]}))
        .await;
    j.until(|j| {
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(13)
    })
    .await;
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.message_index == 1)
            .count(),
        1
    );
    j.finish().await;
}

#[tokio::test]
async fn rejected_multifield_session_event_has_false_ack_without_partial_metadata_and_refreshes_exact_snapshot()
 {
    for payload in [
        json!({"total_messages":2,"name":17,"model":"must-not-install"}),
        json!({"total_messages":2,"name":"must-not-stick","model":17}),
    ] {
        let mut j = Journey::new(1).await;
        j.boot().await;
        j.hold_events = true;
        let p = j.wire.catalogue()[0].clone();
        let id = j.wire.subscriptions()[0].0;
        let before = j.app.views[&j.target].snapshot.clone().unwrap();
        let reads = j.wire.snapshot_reads();
        j.wire.event(id,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",payload)]})).await;
        j.until(|j| j.held.len() == 1).await;
        j.app.update(j.held.pop_front().unwrap());
        assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
        j.hold_events = false;
        j.until(|j| j.wire.snapshot_reads() > reads && j.wire.subscriptions().len() >= 2)
            .await;
        assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
        j.finish().await;
    }
}

#[tokio::test]
async fn rejected_delta_offset_does_not_initialize_partial_text_or_advance_cursor() {
    let mut j = Journey::new(1).await;
    j.boot().await;
    j.hold_events = true;
    let p = j.wire.catalogue()[0].clone();
    let id = j.wire.subscriptions()[0].0;
    let run = uuid::Uuid::new_v4();
    let view = j.app.views.get_mut(&j.target).unwrap();
    view.snapshot.as_mut().unwrap().run = Some(
        serde_json::from_value(json!({"run_id":run,"state":"running","live_text":null})).unwrap(),
    );
    let before = view.snapshot.clone().unwrap();
    let mut bad = entry(
        p.session_id,
        11,
        "text_delta",
        json!({"offset":1,"text":"not accepted"}),
    );
    bad["run_id"] = json!(run);
    j.wire
        .event(id, &p, json!({"projection":"public-v2","events":[bad]}))
        .await;
    j.until(|j| j.held.len() == 1).await;
    j.app.update(j.held.pop_front().unwrap());
    assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
    assert!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .run
            .as_ref()
            .unwrap()
            .live_text
            .is_none()
    );
    j.finish().await;
}

#[tokio::test]
async fn supplied_invalid_total_is_rejected_atomically_while_omitted_total_keeps_existing_count() {
    for total in [json!(-1), json!("2"), Value::Null, json!(1.5)] {
        let mut j = Journey::new(1).await;
        j.boot().await;
        j.hold_events = true;
        let p = j.wire.catalogue()[0].clone();
        let id = j.wire.subscriptions()[0].0;
        let before = j.app.views[&j.target].snapshot.clone().unwrap();
        let reads = j.wire.snapshot_reads();
        j.wire.event(id,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"total_messages":total,"name":"must-not-stick","model":"must-not-stick"}))]})).await;
        j.until(|j| j.held.len() == 1).await;
        j.app.update(j.held.pop_front().unwrap());
        assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
        j.hold_events = false;
        j.until(|j| j.wire.snapshot_reads() > reads && j.wire.subscriptions().len() >= 2)
            .await;
        assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
        let fresh = j.wire.subscriptions()[1].0;
        j.wire.event(fresh,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"name":"Omitted total is accepted"}))]})).await;
        j.until(|j| {
            j.app.views[&j.target]
                .snapshot
                .as_ref()
                .unwrap()
                .observation_cursor
                == Some(11)
        })
        .await;
        assert_eq!(
            j.app.views[&j.target]
                .snapshot
                .as_ref()
                .unwrap()
                .total_messages,
            1
        );
        j.finish().await;
    }
}

#[tokio::test]
async fn thirty_three_healthy_legacy_sessions_receive_real_rotated_subscriptions_without_new_snapshots_or_mutations()
 {
    let mut j = Journey::new(33).await;
    j.boot().await;
    let baseline = j.wire.snapshot_reads();
    let p = j.wire.catalogue()[0].clone();
    let first = j.wire.subscriptions()[0].0;
    j.wire.event(first,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"name":"Rotation retains applied cursor"}))]})).await;
    j.until(|j| {
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(11)
    })
    .await;
    assert_eq!(j.wire.subscriptions()[0].1.subscriptions.len(), 32);
    j.until(|j| j.wire.subscriptions().len() >= 2).await;
    let subscriptions = j.wire.subscriptions();
    let served: HashSet<_> = subscriptions
        .iter()
        .take(2)
        .flat_map(|(_, r)| r.subscriptions.iter().map(|s| s.session_id))
        .collect();
    assert_eq!(
        served,
        j.wire.catalogue().iter().map(|p| p.session_id).collect()
    );
    assert_eq!(served.len(), 33);
    assert_eq!(j.wire.snapshot_reads(), baseline);
    assert_eq!(
        j.wire.state.lock().unwrap().retired,
        vec![subscriptions[0].0]
    );
    assert!(subscriptions[1].1.subscriptions.iter().all(|s| s.after
        == (if s.session_id == p.session_id { 11 } else { 10 })
        && s.projection.as_deref() == Some("public-v2")));
    j.finish().await;
}

#[tokio::test]
async fn healthy_at_most_thirty_two_subscriptions_are_not_retired_by_the_legacy_rotation_timer() {
    let mut j = Journey::new(2).await;
    j.boot().await;
    let start = Instant::now();
    j.until(|_| start.elapsed() > Duration::from_millis(2300))
        .await;
    assert_eq!(j.wire.subscriptions().len(), 1);
    assert!(j.wire.state.lock().unwrap().retired.is_empty());
    j.finish().await;
}

#[tokio::test]
async fn replay_gap_and_legacy_invalidation_refresh_without_treating_payload_as_typed_events() {
    for legacy in [false, true] {
        let mut j = Journey::new(1).await;
        j.boot().await;
        let p = j.wire.catalogue()[0].clone();
        let id = j.wire.subscriptions()[0].0;
        let before = j.wire.snapshot_reads();
        j.wire
            .state
            .lock()
            .unwrap()
            .snapshots
            .insert(p.session_id, snapshot(p.session_id, 21));
        let result = if legacy {
            json!({"projection":"public-v1","events":[entry(p.session_id,90,"session",json!({"model":"must-not-apply"}))]})
        } else {
            json!({"projection":"public-v2","replay_gap":true,"events":[]})
        };
        j.wire.event(id, &p, result).await;
        j.until(|j| {
            j.wire.snapshot_reads() > before
                && j.app.views[&j.target]
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .observation_cursor
                    == Some(21)
        })
        .await;
        assert_eq!(
            j.app.views[&j.target].snapshot.as_ref().unwrap().model,
            "observer-model"
        );
        j.finish().await;
    }
}

#[tokio::test]
async fn malformed_page_missing_cursor_and_wrong_inner_session_never_mutate_or_advance_old_view() {
    for bad in [
        json!({"projection":"public-v2","events":{}}),
        json!({"projection":"public-v2","events":[{"session_id":uuid::Uuid::new_v4(),"revision":18,"kind":"session","payload":{"model":"foreign"}}]}),
        json!({"projection":"public-v2","events":[entry(uuid::Uuid::new_v4(),11,"session",json!({"model":"foreign"}))]}),
    ] {
        let mut j = Journey::new(1).await;
        j.boot().await;
        let p = j.wire.catalogue()[0].clone();
        let id = j.wire.subscriptions()[0].0;
        let before = j.app.views[&j.target].snapshot.clone().unwrap();
        let reads = j.wire.snapshot_reads();
        j.wire.event(id, &p, bad).await;
        j.until(|j| j.wire.snapshot_reads() > reads && j.wire.subscriptions().len() >= 2)
            .await;
        assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
        assert!(j.held.is_empty());
        j.finish().await;
    }
}

#[tokio::test]
async fn rejected_current_owner_or_retired_route_app_ack_cannot_advance_stream_resume_cursor() {
    for replaced_route in [false, true] {
        let mut j = Journey::new(1).await;
        j.boot().await;
        j.hold_events = true;
        let p = j.wire.catalogue()[0].clone();
        let id = j.wire.subscriptions()[0].0;
        j.wire.event(id,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"model":"stale"}))]})).await;
        j.until(|j| j.held.len() == 1).await;
        if replaced_route {
            j.app.clients.insert(j.wire.client.clone());
        } else {
            j.app.views.get_mut(&j.target).unwrap().process.incarnation = uuid::Uuid::new_v4();
        }
        j.app.update(j.held.pop_front().unwrap());
        j.hold_events = false;
        j.until(|j| j.wire.subscriptions().len() >= 2).await;
        // The observer refetches after a false/drop ACK; it never claims cursor11
        // was applied to a different current activation/owner.
        assert_eq!(j.wire.subscriptions()[1].1.subscriptions[0].after, 10);
        assert_eq!(
            j.app.views[&j.target].snapshot.as_ref().unwrap().model,
            "observer-model"
        );
        j.finish().await;
    }
}

#[tokio::test]
async fn eight_read_refresh_bound_keeps_composer_responsive_until_owned_replies_are_released() {
    let mut j = Journey::new(1).await;
    // Stop the first observer before replacing its catalogue; no hidden old
    // task is allowed to overlap the measured initial refresh fan-out.
    let first = j.observer.take().unwrap();
    first.abort();
    let _ = first.await;
    let route = j.target.route;
    let mut processes = Vec::new();
    for _ in 0..12 {
        let p = process(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let target = Target {
            route,
            session: p.session_id,
        };
        let mut view = View::new(p.clone());
        view.snapshot = Some(serde_json::from_value(snapshot(p.session_id, 10)).unwrap());
        view.draft.insert_str("retained observer draft Δ");
        j.app.views.insert(target, view);
        processes.push(p);
    }
    {
        let mut state = j.wire.state.lock().unwrap();
        state.processes = processes.clone();
        state.snapshots = processes
            .iter()
            .map(|p| (p.session_id, snapshot(p.session_id, 10)))
            .collect();
        state.hold_snapshots = true;
    }
    j.observer = Some(spawn(
        j.wire.client.clone(),
        route,
        j.app.sender.clone(),
        j.selected.subscribe(),
    ));
    j.until(|j| j.wire.state.lock().unwrap().held.len() == 8)
        .await;
    let before = j.wire.snapshot_reads();
    let start = Instant::now();
    j.until(|_| start.elapsed() > Duration::from_millis(120))
        .await;
    assert_eq!(j.wire.snapshot_reads(), before);
    j.app.selected = Some(Target {
        route,
        session: processes[0].session_id,
    });
    j.app.sidebar.focus = super::super::sidebar::Focus::Composer;
    j.app
        .input(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('x'),
                crossterm::event::KeyModifiers::NONE,
            ),
        ))
        .unwrap();
    assert!(
        j.app.views[&j.app.selected.unwrap()]
            .draft
            .text
            .ends_with('x')
    );
    j.app
        .views
        .get_mut(&j.app.selected.unwrap())
        .unwrap()
        .draft
        .set_text("retained observer draft Δ".into());
    j.wire.signals.send(Signal::Release).await.unwrap();
    j.until(|j| !j.wire.subscriptions().is_empty()).await;
    assert_eq!(
        j.wire.subscriptions().last().unwrap().1.subscriptions.len(),
        12
    );
    j.finish().await;
}

#[tokio::test]
async fn two_real_peer_observers_keep_other_vessel_events_live_while_one_snapshot_is_pending() {
    let mut j = Journey::new(1).await;
    let old = j.observer.take().unwrap();
    old.abort();
    let _ = old.await;
    j.wire.state.lock().unwrap().hold_snapshots = true;
    j.observer = Some(spawn(
        j.wire.client.clone(),
        j.target.route,
        j.app.sender.clone(),
        j.selected.subscribe(),
    ));
    j.until(|j| j.wire.state.lock().unwrap().held.len() == 1)
        .await;
    let mut other = Wire::new(1).await;
    let route = j.app.clients.insert(other.client.clone());
    let p = other.catalogue()[0].clone();
    let target = Target {
        route,
        session: p.session_id,
    };
    let mut view = View::new(p.clone());
    view.snapshot = Some(serde_json::from_value(snapshot(p.session_id, 10)).unwrap());
    view.draft.insert_str("retained other-vessel draft");
    j.app.views.insert(target, view);
    let job = spawn(
        other.client.clone(),
        route,
        j.app.sender.clone(),
        j.selected.subscribe(),
    );
    j.until(|_| !other.subscriptions().is_empty()).await;
    let id = other.subscriptions()[0].0;
    other.event(id,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"name":"Other vessel remains live"}))]})).await;
    j.until(|j| {
        j.app.views[&target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(11)
    })
    .await;
    assert_eq!(j.app.selected, Some(j.target));
    assert_eq!(
        j.app.views[&target].draft.text,
        "retained other-vessel draft"
    );
    assert_eq!(j.wire.state.lock().unwrap().held.len(), 1);
    job.abort();
    assert!(
        tokio::time::timeout(WAIT, job)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    tokio::time::timeout(WAIT, other.client.request(VesselCommand::Capabilities))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(WAIT, async {
        while !other.state.lock().unwrap().active.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    other.finish().await;
    j.wire.signals.send(Signal::Release).await.unwrap();
    j.boot().await;
    j.finish().await;
}

#[tokio::test]
async fn authoritative_catalogue_hydrates_only_selected_voyage_and_selection_retires_exact_old_stream()
 {
    let mut j = Journey::new(3).await;
    let old = j.observer.take().unwrap();
    old.abort();
    let _ = old.await;
    {
        let mut state = j.wire.state.lock().unwrap();
        for p in &mut state.processes {
            p.catalogue = Some(Box::new(voyage_protocol::process::CatalogueMetadata {
                summary: Some(voyage_protocol::process::CatalogueSummary {
                    session_id: p.session_id,
                    revision: 17,
                    observation_cursor: 10,
                    name: Some("Authoritative sidebar".into()),
                    model: "observer-model".into(),
                    created_at: None,
                    last_turn_end: None,
                    total_messages: 1,
                    run_id: None,
                    run_state: None,
                    archived: false,
                    deleted: false,
                    pending_cleanup_run: None,
                }),
                observed_at_ms: Some(1),
                stale: false,
                error_code: None,
            }));
        }
    }
    j.observer = Some(spawn(
        j.wire.client.clone(),
        j.target.route,
        j.app.sender.clone(),
        j.selected.subscribe(),
    ));
    j.boot().await;
    assert_eq!(j.wire.snapshot_reads(), 1);
    let first = j.wire.subscriptions()[0].clone();
    assert_eq!(first.1.subscriptions.len(), 1);
    assert_eq!(first.1.subscriptions[0].session_id, j.target.session);
    let started = Instant::now();
    j.until(|_| started.elapsed() > Duration::from_millis(2300))
        .await;
    assert_eq!(j.wire.subscriptions().len(), 1);
    let other = j.wire.catalogue()[1].clone();
    let next = Target {
        route: j.target.route,
        session: other.session_id,
    };
    j.app.selected = Some(next);
    j.selected.send(Some(next)).unwrap();
    j.until(|j| j.wire.subscriptions().len() >= 2).await;
    assert_eq!(j.wire.subscriptions()[1].1.subscriptions.len(), 1);
    assert_eq!(
        j.wire.subscriptions()[1].1.subscriptions[0].session_id,
        next.session
    );
    assert_eq!(j.wire.snapshot_reads(), 2);
    assert_eq!(j.wire.state.lock().unwrap().retired, vec![first.0]);
    assert_eq!(j.app.selected, Some(next));
    assert_eq!(
        j.app.views[&next].snapshot.as_ref().unwrap().messages[0].content,
        "Canonical observer Ω"
    );
    for view in j.app.views.values() {
        assert_eq!(view.draft.text, "retained observer draft Δ");
        assert!(view.pending.is_none());
    }
    assert!(j.app.command_checks.is_empty() && j.app.route_tasks.is_empty());
    assert!(
        j.wire
            .state
            .lock()
            .unwrap()
            .snapshots
            .values()
            .all(|s| s["messages"][0]["content"] == "Canonical observer Ω")
    );
    j.shutdown().await;
}

#[tokio::test]
async fn accepted_run_utf8_deltas_session_and_lifecycle_use_actual_app_ack_without_any_tool_execution()
 {
    let mut j = Journey::new(1).await;
    j.boot().await;
    let p = j.wire.catalogue()[0].clone();
    let id = j.wire.subscriptions()[0].0;
    let run = uuid::Uuid::new_v4();
    let mut run_event = entry(
        p.session_id,
        11,
        "run_state",
        json!({"run_id":run,"state":"running"}),
    );
    run_event["run_id"] = json!(run);
    let mut delta = entry(
        p.session_id,
        12,
        "text_delta",
        json!({"offset":0,"text":"Δ"}),
    );
    delta["run_id"] = json!(run);
    let mut next = entry(
        p.session_id,
        13,
        "text_delta",
        json!({"offset":2,"text":"β"}),
    );
    next["run_id"] = json!(run);
    j.wire.event(id,&p,json!({"projection":"public-v2","events":[run_event,delta,next,entry(p.session_id,14,"session",json!({"name":null,"model":"observed-new-model","total_messages":1})),entry(p.session_id,15,"lifecycle",json!({"archived":false,"deleted":false}))]})).await;
    j.until(|j| {
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(15)
    })
    .await;
    let snapshot = j.app.views[&j.target].snapshot.as_ref().unwrap();
    let observed = snapshot.run.as_ref().unwrap();
    assert_eq!(observed.run_id, run);
    assert_eq!(observed.live_text.as_deref(), Some("Δβ"));
    assert_eq!(observed.partial_text_bytes, 4);
    assert_eq!(snapshot.model, "observed-new-model");
    assert!(snapshot.name.is_none());
    assert_eq!(snapshot.messages.len(), 1);
    j.finish().await;
}

#[tokio::test]
async fn owner_transition_event_retires_old_stream_and_never_relabels_new_owner_payload_as_old() {
    let mut j = Journey::new(1).await;
    j.boot().await;
    let mut p = j.wire.catalogue()[0].clone();
    let id = j.wire.subscriptions()[0].0;
    let reads = j.wire.snapshot_reads();
    let before = j.app.views[&j.target].snapshot.clone().unwrap();
    p.incarnation = uuid::Uuid::new_v4();
    j.wire.event(id,&p,json!({"owner_changed":true,"replay_gap":true,"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"model":"new-owner-must-not-install"}))]})).await;
    j.until(|j| j.wire.state.lock().unwrap().retired.contains(&id))
        .await;
    assert!(j.app.views[&j.target].snapshot.as_ref() == Some(&before));
    assert_eq!(j.wire.snapshot_reads(), reads);
    j.finish().await;
}

#[tokio::test]
async fn event_and_snapshot_recovery_preserve_frozen_pending_receipt_identity_and_unsent_draft() {
    let mut j = Journey::new(1).await;
    j.boot().await;
    let p = j.wire.catalogue()[0].clone();
    let id = j.wire.subscriptions()[0].0;
    let command = uuid::Uuid::new_v4();
    j.app.views.get_mut(&j.target).unwrap().pending = Some(super::super::state::Pending {
        account_host: None,
        command_id: command,
        incarnation: p.incarnation,
        draft: "exact frozen pending text".into(),
        preserve_draft: true,
        original: None,
        receipt_only: true,
    });
    j.wire.event(id,&p,json!({"projection":"public-v2","events":[entry(p.session_id,11,"session",json!({"name":"Read-only metadata"}))]})).await;
    j.until(|j| {
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(11)
    })
    .await;
    let pending = j.app.views[&j.target].pending.as_ref().unwrap();
    assert_eq!(pending.command_id, command);
    assert_eq!(pending.incarnation, p.incarnation);
    assert_eq!(pending.draft, "exact frozen pending text");
    assert!(pending.preserve_draft && pending.receipt_only);
    j.wire
        .state
        .lock()
        .unwrap()
        .snapshots
        .insert(p.session_id, snapshot(p.session_id, 21));
    j.wire
        .event(
            id,
            &p,
            json!({"projection":"public-v2","replay_gap":true,"events":[]}),
        )
        .await;
    j.until(|j| {
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .observation_cursor
            == Some(21)
    })
    .await;
    let pending = j.app.views[&j.target].pending.as_ref().unwrap();
    assert_eq!(pending.command_id, command);
    assert_eq!(pending.incarnation, p.incarnation);
    assert_eq!(pending.draft, "exact frozen pending text");
    assert!(pending.original.is_none());
    assert_eq!(
        j.app.views[&j.target].draft.text,
        "retained observer draft Δ"
    );
    assert_eq!(
        j.app.views[&j.target].snapshot.as_ref().unwrap().messages[0].content,
        "Canonical observer Ω"
    );
    assert!(j.app.command_checks.is_empty() && j.app.route_tasks.is_empty());
    j.shutdown().await;
}
