//! Scripted public Vessel exchanges through the real Client socket. No providers.
use super::*;
use crate::process_client::ui::{
    account_test_support, accounts, routes::Routes, socket_support_tests::Server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Store {
    records: Vec<Value>,
    requests: Vec<Value>,
    reject_put: bool,
}
async fn server(store: Arc<Mutex<Store>>) -> Server {
    Server::new(move |command| {
        let wire = serde_json::to_value(command).unwrap();
        assert_eq!(wire["op"], "drafts");
        let op = &wire["operation"];
        let mut store = store.lock().unwrap();
        store.requests.push(op.clone());
        match op["op"].as_str().unwrap() {
            "list" => Ok(json!({"drafts":store.records})),
            "put" => {
                if store.reject_put { return Err("revision conflict".into()); }
                let index = store.records.iter().position(|r| r["draft_id"] == op["draft_id"]);
                let revision = index.map_or(0, |i| store.records[i]["revision"].as_u64().unwrap());
                if op["expected_revision"] != revision { return Err("revision conflict".into()); }
                let record = json!({"draft_id":op["draft_id"],"revision":revision+1,"document":op["document"]});
                if let Some(index) = index { store.records[index] = record.clone(); }
                else { store.records.push(record.clone()); }
                Ok(record)
            }
            "delete" => {
                let index = store.records.iter().position(|r| r["draft_id"] == op["draft_id"]).ok_or("missing draft")?;
                if store.records[index]["revision"] != op["expected_revision"] { return Err("revision conflict".into()); }
                store.records.remove(index);
                Ok(json!({"deleted":true}))
            }
            unexpected => panic!("unexpected draft operation {unexpected}"),
        }
    }).await
}
fn entry(local: Destination, target: Value, text: &str) -> Entry {
    let mut composer = composer::Composer::default();
    composer.insert_str(text);
    let document = document(target, &composer, &[]).unwrap();
    Entry {
        local,
        link: Link::default(),
        authored: document.clone(),
        document,
        composer,
        images: vec![],
        frozen: false,
        clearing: false,
    }
}
fn record(id: Uuid, revision: u64, entry: &Entry) -> Value {
    json!({"draft_id":id,"revision":revision,"document":entry.document})
}
fn message(target: Target) -> Value {
    json!({"type":"message","session_id":target.session})
}
fn edit(entry: &mut Entry, text: &str) {
    entry.composer.take();
    entry.composer.insert_str(text);
    entry.document = document(entry.document["target"].clone(), &entry.composer, &[]).unwrap();
    entry.authored = entry.document.clone();
}
async fn exchange(server: &Server, entries: Vec<Entry>) -> Batch {
    tokio::time::timeout(
        Duration::from_secs(3),
        sync(server.client.clone(), server.target.route, entries),
    )
    .await
    .expect("bounded socket exchange")
    .unwrap()
}
fn app(server: &Server, root: &std::path::Path) -> App {
    let mut app = accounts::app_tests::app(root);
    app.clients = Routes::new(vec![server.client.clone()]);
    let view = state::View::new(
        serde_json::from_value(json!({"session_id":server.target.session,
        "incarnation":server.incarnation,"workspace":"/synthetic-workspace","state":"live"}))
        .unwrap(),
    );
    app.views.insert(server.target, view);
    app.selected = Some(server.target);
    app
}
async fn apply(app: &mut App, batch: Batch) {
    // Hold a completed network batch until after the test's intervening UI edit.
    app.shared_drafts.busy = true;
    app.shared_drafts.job = Some(tokio::spawn(async move { vec![Ok(batch)] }));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !app.shared_drafts.job.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.poll_shared_drafts();
    assert!(!app.shared_drafts.busy);
}

#[tokio::test]
async fn two_devices_restore_existing_and_multiple_new_chats_without_rediscovery() {
    let store = Arc::new(Mutex::new(Store::default()));
    let a = server(store.clone()).await;
    let b = server(store.clone()).await;
    let existing = entry(
        Destination::Live(a.target),
        message(a.target),
        "from device A 界",
    );
    let new_one = entry(
        Destination::New(Uuid::new_v4()),
        json!({"type":"new_chat","workspace":"/one"}),
        "first new",
    );
    let new_two = entry(
        Destination::New(Uuid::new_v4()),
        json!({"type":"new_chat","workspace":"/two"}),
        "second new",
    );
    let saved = exchange(&a, vec![existing, new_one, new_two]).await;
    assert!(
        saved
            .entries
            .iter()
            .all(|e| e.link.revision == 2 && !e.link.conflict)
    );
    let mut target = b.target;
    target.session = a.target.session;
    let restored = exchange(
        &b,
        vec![entry(Destination::Live(target), message(target), "")],
    )
    .await;
    assert_eq!(restored.entries[0].composer.text, "from device A 界");
    assert_eq!(restored.entries[0].link.id, saved.entries[0].link.id);
    assert_eq!(restored.discovered.len(), 2);
    let fixture = account_test_support::Fixture::new();
    let mut ui = app(&b, fixture.0.path());
    // Restore both into real new-chat storage, then exercise discovery with their saved links.
    for discovered in restored.discovered {
        let Destination::New(id) = discovered.local else {
            panic!("new destination")
        };
        ui.apply_shared_new(id, b.target.route, discovered).unwrap();
    }
    assert_eq!(ui.new_drafts.len(), 2);
    let entries = ui.shared_new_entries(b.target.route);
    assert_eq!(entries.len(), 2);
    let repeated = exchange(&b, entries).await;
    assert!(repeated.discovered.is_empty());
    assert!(repeated.entries.iter().all(|e| e.link.revision == 2));
    assert_eq!(store.lock().unwrap().records.len(), 3);
}

#[tokio::test]
async fn empty_remote_bootstrap_keeps_its_identity_and_can_be_edited() {
    for local_text in ["", "new local content"] {
        let store = Arc::new(Mutex::new(Store::default()));
        let server = server(store.clone()).await;
        let remote = entry(Destination::Live(server.target), message(server.target), "");
        let id = Uuid::new_v4();
        store.lock().unwrap().records.push(record(id, 4, &remote));
        let mut local = entry(remote.local, message(server.target), local_text);
        // poll_shared_drafts allocates an id before searching an existing session.
        if !local_text.is_empty() {
            local.link.id = Some(Uuid::new_v4());
        }
        let batch = exchange(&server, vec![local]).await;
        let mut restored = batch.entries.into_iter().next().unwrap();
        assert_eq!(restored.link.id, Some(id));
        assert!(!restored.link.conflict);
        assert_eq!(restored.composer.text, local_text);
        edit(&mut restored, "next edit");
        let next = exchange(&server, vec![restored]).await;
        assert!(!next.entries[0].link.conflict);
        let store = store.lock().unwrap();
        assert_eq!(store.records.len(), 1);
        assert_eq!(
            store.records[0]["document"]["parts"][0]["text"],
            "next edit"
        );
    }
}

#[tokio::test]
async fn local_deletion_is_an_edit_not_permission_to_restore_remote_changes() {
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let saved = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            message(server.target),
            "base",
        )],
    )
    .await;
    let mut local = saved.entries.into_iter().next().unwrap();
    edit(&mut local, "");
    {
        let mut store = store.lock().unwrap();
        store.records[0]["revision"] = json!(3);
        store.records[0]["document"]["parts"][0]["text"] = json!("other device");
        store.requests.clear();
    }
    let batch = exchange(&server, vec![local]).await;
    assert!(batch.entries[0].link.conflict);
    assert!(batch.entries[0].composer.text.is_empty());
    assert_eq!(
        store.lock().unwrap().requests.len(),
        1,
        "conflict must not put"
    );
}

#[tokio::test]
async fn cas_failure_retains_authored_content_and_last_acknowledged_base() {
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let batch = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            message(server.target),
            "base",
        )],
    )
    .await;
    let mut local = batch.entries.into_iter().next().unwrap();
    let base = local.link.base.clone();
    edit(&mut local, "local edit");
    store.lock().unwrap().reject_put = true;
    let batch = exchange(&server, vec![local]).await;
    assert!(batch.entries[0].link.conflict);
    assert_eq!(batch.entries[0].composer.text, "local edit");
    assert_eq!(batch.entries[0].link.base, base);
    assert_eq!(batch.entries[0].link.revision, 2);
    let store = store.lock().unwrap();
    assert_eq!(store.requests.last().unwrap()["expected_revision"], 2);
    assert_eq!(store.records[0]["document"]["parts"][0]["text"], "base");
}

#[tokio::test]
async fn save_acknowledgement_does_not_clobber_in_flight_ui_edits() {
    let fixture = account_test_support::Fixture::new();
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let mut ui = app(&server, fixture.0.path());
    let sent = entry(
        Destination::Live(server.target),
        message(server.target),
        "captured",
    );
    let batch = exchange(&server, vec![sent]).await;
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .draft
        .insert_str("newer edit");
    apply(&mut ui, batch).await;
    let view = &ui.views[&server.target];
    assert_eq!(view.draft.text, "newer edit");
    assert_eq!(view.shared.revision, 2);
    assert!(!view.shared.conflict);
    assert!(
        ui.shared_send_guard(server.target).is_err(),
        "newer edit is not yet acknowledged"
    );
    let mut next = entry(
        Destination::Live(server.target),
        message(server.target),
        &view.draft.text,
    );
    next.link = view.shared.clone();
    let batch = exchange(&server, vec![next]).await;
    apply(&mut ui, batch).await;
    assert!(ui.shared_send_guard(server.target).is_ok());
    assert_eq!(
        store.lock().unwrap().records[0]["document"]["parts"][0]["text"],
        "newer edit"
    );
}

#[tokio::test]
async fn pending_freezes_exchange_and_discards_late_acknowledgements() {
    let fixture = account_test_support::Fixture::new();
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let mut frozen = entry(
        Destination::Live(server.target),
        message(server.target),
        "pending content",
    );
    frozen.frozen = true;
    let batch = exchange(&server, vec![frozen]).await;
    assert_eq!(store.lock().unwrap().requests.len(), 1);
    assert!(store.lock().unwrap().records.is_empty());
    assert_eq!(batch.entries[0].composer.text, "pending content");
    let mut ui = app(&server, fixture.0.path());
    let pending = json!({"command_id":Uuid::new_v4(),"incarnation":server.incarnation,"draft":"pending content"});
    let view = ui.views.get_mut(&server.target).unwrap();
    view.draft.insert_str("private pending");
    view.pending = Some(serde_json::from_value(pending).unwrap());
    let batch = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            message(server.target),
            "late save",
        )],
    )
    .await;
    apply(&mut ui, batch).await;
    let view = &ui.views[&server.target];
    assert_eq!(view.draft.text, "private pending");
    assert!(view.shared.id.is_none());
    assert!(view.pending.is_some());
}

#[tokio::test]
async fn steering_intents_match_exact_run_and_guard_rejects_changed_run_or_incarnation() {
    let fixture = account_test_support::Fixture::new();
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let run = Uuid::new_v4();
    let intent = json!({"type":"steer","session_id":server.target.session,"run_id":run,"incarnation":server.incarnation});
    let saved = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            intent.clone(),
            "steer",
        )],
    )
    .await;
    // An untouched second device restores the pinned steering target, not a new run.
    let restored = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            message(server.target),
            "",
        )],
    )
    .await;
    assert_eq!(restored.entries[0].link.target, Some(intent.clone()));
    assert_eq!(restored.entries[0].composer.text, "steer");
    // An explicitly selected message intent must not bind to a steering draft.
    let mut explicit_message = entry(Destination::Live(server.target), message(server.target), "");
    explicit_message.link.target = Some(message(server.target));
    let message_batch = exchange(&server, vec![explicit_message]).await;
    assert!(message_batch.entries[0].link.id.is_none());
    assert!(message_batch.entries[0].composer.text.is_empty());
    let mut ui = app(&server, fixture.0.path());
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .draft
        .insert_str("steer");
    apply(&mut ui, saved).await;
    let view = ui.views.get_mut(&server.target).unwrap();
    view.snapshot = Some(serde_json::from_value(json!({"session_id":server.target.session,"revision":1,"model":"synthetic","messages":[],"decisions":[],"run":{"run_id":run,"state":"running"}})).unwrap());
    assert!(ui.shared_send_guard(server.target).is_ok());
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run
        .as_mut()
        .unwrap()
        .run_id = Uuid::new_v4();
    assert!(ui.shared_send_guard(server.target).is_err());
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run
        .as_mut()
        .unwrap()
        .run_id = run;
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .process
        .incarnation = Uuid::new_v4();
    assert!(ui.shared_send_guard(server.target).is_err());
}

#[tokio::test]
async fn post_send_clear_preserves_newer_edits_and_refuses_remote_revision_races() {
    for steering in [false, true] {
        for race in [false, true] {
            let fixture = account_test_support::Fixture::new();
            let store = Arc::new(Mutex::new(Store::default()));
            let server = server(store.clone()).await;
            let intent = if steering {
                json!({"type":"steer","session_id":server.target.session,"run_id":Uuid::new_v4(),"incarnation":server.incarnation})
            } else {
                message(server.target)
            };
            let saved = exchange(
                &server,
                vec![entry(Destination::Live(server.target), intent, "sent")],
            )
            .await;
            let mut clearing = saved.entries.into_iter().next().unwrap();
            clearing.link.admitted_revision = Some(clearing.link.revision);
            clearing.link.send_revision = Some(clearing.link.revision);
            clearing.clearing = true;
            edit(&mut clearing, "");
            let mut ui = app(&server, fixture.0.path());
            ui.views.get_mut(&server.target).unwrap().shared = clearing.link.clone();
            if race {
                let mut store = store.lock().unwrap();
                store.records[0]["revision"] = json!(3);
                store.records[0]["document"]["parts"][0]["text"] = json!("remote newer");
            }
            let batch = exchange(&server, vec![clearing]).await;
            ui.views
                .get_mut(&server.target)
                .unwrap()
                .draft
                .insert_str("typed after send");
            apply(&mut ui, batch).await;
            let view = &ui.views[&server.target];
            assert_eq!(view.draft.text, "typed after send");
            assert_eq!(view.shared.conflict, race);
            if race {
                assert_eq!(view.shared.admitted_revision, Some(2));
                assert_eq!(
                    store.lock().unwrap().records[0]["document"]["parts"][0]["text"],
                    "remote newer"
                );
            } else {
                assert!(view.shared.admitted_revision.is_none());
                assert_eq!(view.shared.fork, steering);
                assert_eq!(
                    store.lock().unwrap().records[0]["document"]["parts"],
                    json!([])
                );
            }
        }
    }
}

#[tokio::test]
async fn new_chat_save_ack_keeps_newer_editor_text_and_pending_launch_is_frozen() {
    let fixture = account_test_support::Fixture::new();
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let seed = entry(
        Destination::New(Uuid::new_v4()),
        json!({"type":"new_chat","workspace":"/new"}),
        "original",
    );
    exchange(&server, vec![seed]).await;
    let discovered = exchange(&server, vec![]).await.discovered.pop().unwrap();
    let Destination::New(id) = discovered.local else {
        panic!("new chat")
    };
    let mut ui = app(&server, fixture.0.path());
    ui.apply_shared_new(id, server.target.route, discovered)
        .unwrap();
    ui.select_draft(id);
    assert!(ui.new_draft_input(&Event::Paste(" saved".into())).unwrap());
    let captured = ui.shared_new_entries(server.target.route);
    let batch = exchange(&server, captured).await;
    assert!(ui.new_draft_input(&Event::Paste(" newer".into())).unwrap());
    apply(&mut ui, batch).await;
    assert_eq!(
        ui.shared_new_entries(server.target.route)[0].composer.text,
        "original saved newer"
    );
    let entries = ui.shared_new_entries(server.target.route);
    assert!(!entries[0].link.conflict);
    assert_eq!(
        entries[0].link.base.as_ref().unwrap()["parts"][0]["text"],
        "original saved"
    );
    ui.new_drafts.get_mut(&id).unwrap().busy = true;
    let entries = ui.shared_new_entries(server.target.route);
    assert!(entries[0].frozen);
    store.lock().unwrap().requests.clear();
    let batch = exchange(&server, entries).await;
    apply(&mut ui, batch).await;
    assert_eq!(store.lock().unwrap().requests.len(), 1);
    assert_eq!(
        ui.shared_new_entries(server.target.route)[0].composer.text,
        "original saved newer"
    );
    assert_eq!(
        ui.shared_new_entries(server.target.route)[0].link.revision,
        3
    );
}

#[tokio::test]
async fn remote_restore_racing_local_typing_retains_both_versions_as_conflict() {
    let fixture = account_test_support::Fixture::new();
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let saved = exchange(
        &server,
        vec![entry(
            Destination::Live(server.target),
            message(server.target),
            "base",
        )],
    )
    .await;
    let mut local = saved.entries.into_iter().next().unwrap();
    // A new poll captures the now-acknowledged document.
    local.authored = local.document.clone();
    {
        let mut store = store.lock().unwrap();
        store.records[0]["revision"] = json!(3);
        store.records[0]["document"]["parts"][0]["text"] = json!("remote edit");
    }
    let batch = exchange(&server, vec![local]).await;
    assert_eq!(batch.entries[0].composer.text, "remote edit");
    let mut ui = app(&server, fixture.0.path());
    ui.views
        .get_mut(&server.target)
        .unwrap()
        .draft
        .insert_str("local edit during restore");
    apply(&mut ui, batch).await;
    let view = &ui.views[&server.target];
    assert!(view.shared.conflict);
    assert_eq!(view.draft.text, "local edit during restore");
    assert_eq!(
        view.shared.base.as_ref().unwrap()["parts"][0]["text"],
        "remote edit"
    );
    assert!(ui.shared_send_guard(server.target).is_err());
}

#[tokio::test]
async fn explicit_discard_requires_exact_revision_and_retains_conflicts() {
    let store = Arc::new(Mutex::new(Store::default()));
    let server = server(store.clone()).await;
    let mut local = entry(
        Destination::New(Uuid::new_v4()),
        json!({"type":"new_chat","workspace":"/synthetic-workspace"}),
        "discard me",
    );
    let id = Uuid::new_v4();
    local.link.id = Some(id);
    local.link.revision = 1;
    local.link.discard_requested = true;
    store.lock().unwrap().records.push(record(id, 2, &local));
    let failed = exchange(&server, vec![local.clone()]).await;
    assert!(failed.entries[0].link.conflict);
    assert!(!failed.entries[0].link.discarded);
    assert_eq!(store.lock().unwrap().records.len(), 1);
    local.link.revision = 2;
    let removed = exchange(&server, vec![local]).await;
    assert!(removed.entries[0].link.discarded);
    assert!(store.lock().unwrap().records.is_empty());
}
