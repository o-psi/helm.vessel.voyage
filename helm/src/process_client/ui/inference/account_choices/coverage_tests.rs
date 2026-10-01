use super::*;
use serde_json::json;
#[tokio::test]
async fn account_load_publication_requires_same_target_and_incarnation() {
    let (_fixture, mut app, target) = crate::process_client::ui::coverage_support::app();
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .inference = Some(Settings {
        model: "fixture".into(),
        provider: "openai-responses".into(),
        ..Default::default()
    });
    let original = app.inference_settings(Destination::Live(target)).unwrap();
    let incarnation = app.views[&target].process.incarnation;
    let mut draft = chooser::Draft::new(&original);
    draft.accounts_open = true;
    draft.accounts_loading = true;
    app.inference.picker = Some(Picker {
        id: Uuid::new_v4(),
        chooser: draft,
        incarnation: Some(incarnation),
        destination: Destination::Live(target),
        original,
        models: vec![],
        field: Field::Model,
        query: String::new(),
        selected: 0,
        options: vec![],
        loading: false,
        models_loaded: false,
        notice: String::new(),
        confirmation: None,
        command_text: String::new(),
        preserve_draft: true,
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.inference.account_load = Some(Load {
        destination: Destination::Live(target),
        incarnation: Some(incarnation),
        receiver: rx,
        task: tokio::spawn(async {}),
    });
    app.poll_chooser_accounts();
    assert!(app.inference.account_load.is_some());
    let host = Uuid::new_v4();
    tx.send(Ok((
        host,
        serde_json::from_value(json!({"accounts":[],"connections":[]})).unwrap(),
        None,
    )))
    .ok()
    .unwrap();
    app.poll_chooser_accounts();
    assert!(app.inference.account_load.is_none());
    assert_eq!(app.account_host(target.route), Some(host));
    assert!(
        !app.inference
            .picker
            .as_ref()
            .unwrap()
            .chooser
            .accounts_loading
    );
    assert!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("No accounts")
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.inference.account_load = Some(Load {
        destination: Destination::Live(target),
        incarnation: Some(Uuid::new_v4()),
        receiver: rx,
        task: tokio::spawn(async {}),
    });
    drop(tx);
    app.poll_chooser_accounts();
    assert!(app.inference.account_load.is_none());
    assert!(app.views[&target].pending.is_none());
}
#[test]
fn catalogue_cross_product_keeps_availability_and_default_binding_explicit() {
    let a = Uuid::new_v4();
    let c = Uuid::new_v4();
    let catalogue:Catalogue=serde_json::from_value(json!({"accounts":[{"id":a,"connection_id":c,"alias":"fixture","label":"fixture","metadata_revision":1,"identity_generation":2,"credential_revision":1,"capability_revision":1,"availability":"available","state":"ready"}],"connections":[{"id":c,"revision":3,"label":"fixture","endpoint":"https://example.invalid","transports":["openai_responses","openai_chat","anthropic","chatgpt_oauth"]}],"default_account":{"account_id":a,"connection_id":c,"identity_generation":2,"connection_revision":3,"transport":"openai_responses"}})).unwrap();
    let choices = catalogue.choices().unwrap();
    assert_eq!(choices.len(), 4);
    assert!(choices.iter().all(|c| c.ready));
    assert_eq!(
        choices
            .iter()
            .filter(|c| c.label.contains("Default"))
            .count(),
        1
    );
    assert_eq!(choices[0].binding.connection_revision, 3);
}

fn staged_picker() -> (
    crate::process_client::ui::account_test_support::Fixture,
    App,
    Target,
) {
    let (fixture, mut app, target) = crate::process_client::ui::coverage_support::app();
    let original = Settings {
        model: "manual-model".into(),
        provider: "openai-responses".into(),
        ..Default::default()
    };
    let mut chooser = chooser::Draft::new(&original);
    chooser.accounts_open = true;
    chooser.accounts_loading = true;
    app.inference.picker = Some(Picker {
        id: Uuid::new_v4(),
        chooser,
        incarnation: Some(app.views[&target].process.incarnation),
        destination: Destination::Live(target),
        original,
        models: vec![],
        field: Field::Model,
        query: String::new(),
        selected: 0,
        options: vec![],
        loading: false,
        models_loaded: false,
        notice: "retained notice".into(),
        confirmation: None,
        command_text: String::new(),
        preserve_draft: true,
    });
    (fixture, app, target)
}

#[tokio::test]
async fn private_account_result_is_discarded_after_every_live_review_context_change() {
    for variant in 0..6 {
        let (_fixture, mut app, target) = staged_picker();
        let owner = app.views[&target].process.incarnation;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.account_load = Some(Load {
            destination: Destination::Live(target),
            incarnation: Some(owner),
            receiver,
            task: tokio::spawn(async {}),
        });
        sender
            .send(Ok((
                Uuid::new_v4(),
                serde_json::from_value(json!({"accounts":[],"connections":[]})).unwrap(),
                None,
            )))
            .ok()
            .unwrap();
        match variant {
            0 => app.selected = None,
            1 => {
                app.inference.picker = None;
            }
            2 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            3 => app.inference.picker.as_mut().unwrap().chooser.accounts_open = false,
            4 => app.clients.mark_unavailable(target.route),
            _ => {
                let client = app.clients[target.route].clone();
                app.clients.insert(client);
            }
        };
        app.poll_chooser_accounts();
        assert!(app.inference.account_load.is_none());
        assert_eq!(app.retired_observers.len(), 1);
        assert!(app.account_host(target.route).is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none());
        assert!(app.route_tasks.is_empty());
        if let Some(picker) = &app.inference.picker {
            assert_eq!(picker.notice, "retained notice");
            assert_eq!(picker.original.model, "manual-model");
        }
    }
}

#[tokio::test]
async fn private_account_channel_close_and_oversized_catalogue_preserve_editor_without_model_dispatch()
 {
    for oversized in [false, true] {
        let (_fixture, mut app, target) = staged_picker();
        let owner = app.views[&target].process.incarnation;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.account_load = Some(Load {
            destination: Destination::Live(target),
            incarnation: Some(owner),
            receiver,
            task: tokio::spawn(async {}),
        });
        if oversized {
            let descriptor = json!({"id":Uuid::new_v4(),"connection_id":Uuid::new_v4(),"alias":"fixture","label":"fixture","metadata_revision":1,"identity_generation":1,"credential_revision":1,"capability_revision":1,"availability":"available","state":"ready"});
            sender
                .send(Ok((
                    Uuid::new_v4(),
                    serde_json::from_value(
                        json!({"accounts":vec![descriptor;129],"connections":[]}),
                    )
                    .unwrap(),
                    None,
                )))
                .ok()
                .unwrap();
        } else {
            drop(sender);
        }
        app.poll_chooser_accounts();
        let picker = app.inference.picker.as_ref().unwrap();
        assert!(!picker.chooser.accounts_loading);
        assert!(picker.chooser.accounts.is_empty());
        assert_eq!(picker.original.model, "manual-model");
        assert!(picker.notice.contains(if oversized {
            "exceeds limits"
        } else {
            "interrupted"
        }));
        assert!(app.inference.catalog_job.is_none());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[tokio::test]
async fn private_host_default_initialization_never_overwrites_explicit_model_or_provider() {
    for matching_provider in [false, true] {
        let (_fixture, mut app, target) = staged_picker();
        let owner = app.views[&target].process.incarnation;
        app.inference.picker.as_mut().unwrap().chooser.initializing = true;
        let account = AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 2,
            connection_revision: 3,
            transport: voyage_protocol::accounts::Transport::OpenaiResponses,
        };
        let catalogue:Catalogue=serde_json::from_value(json!({"accounts":[{"id":account.account_id,"connection_id":account.connection_id,"alias":"unavailable","label":"Unavailable","metadata_revision":1,"identity_generation":2,"credential_revision":1,"capability_revision":1,"availability":"missing","state":"sign_in_required"}],"connections":[{"id":account.connection_id,"revision":3,"label":"fixture","endpoint":"https://example.invalid","transports":["openai_responses"]}],"default_account":account})).unwrap();
        let defaults = Settings {
            account: Some(account.clone()),
            provider: if matching_provider {
                "openai-responses"
            } else {
                "anthropic"
            }
            .into(),
            model: "host-default-model".into(),
            ..Default::default()
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.account_load = Some(Load {
            destination: Destination::Live(target),
            incarnation: Some(owner),
            receiver,
            task: tokio::spawn(async {}),
        });
        sender
            .send(Ok((Uuid::new_v4(), catalogue, Some(defaults))))
            .ok()
            .unwrap();
        app.poll_chooser_accounts();
        let picker = app.inference.picker.as_ref().unwrap();
        assert!(!picker.chooser.initializing);
        assert_eq!(picker.original.model, "manual-model");
        assert_eq!(picker.original.provider, "openai-responses");
        assert_eq!(
            picker.original.account,
            matching_provider.then_some(account)
        );
        assert!(!picker.chooser.accounts[0].ready);
        assert!(picker.chooser.accounts_open);
        assert!(app.inference.catalog_job.is_none());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}

#[tokio::test]
async fn private_configuration_draft_account_result_requires_current_available_route() {
    for retired in [false, true] {
        let (fixture, mut app, target) = staged_picker();
        app.create_on_route(target.route, Some(fixture.0.path().to_str().unwrap()))
            .unwrap();
        let id = app.active_draft.unwrap();
        let original = Settings {
            model: "explicit-draft-model".into(),
            provider: "openai-responses".into(),
            ..Default::default()
        };
        let mut chooser = chooser::Draft::new(&original);
        chooser.accounts_open = true;
        chooser.accounts_loading = true;
        app.inference.picker = Some(Picker {
            id: Uuid::new_v4(),
            chooser,
            incarnation: None,
            destination: Destination::Draft(id),
            original,
            models: vec![],
            field: Field::Model,
            query: String::new(),
            selected: 0,
            options: vec![],
            loading: false,
            models_loaded: false,
            notice: "retained draft review".into(),
            confirmation: None,
            command_text: String::new(),
            preserve_draft: true,
        });
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.inference.account_load = Some(Load {
            destination: Destination::Draft(id),
            incarnation: None,
            receiver,
            task: tokio::spawn(async {}),
        });
        sender
            .send(Ok((
                Uuid::new_v4(),
                serde_json::from_value(json!({"accounts":[],"connections":[]})).unwrap(),
                None,
            )))
            .ok()
            .unwrap();
        if retired {
            let client = app.clients[target.route].clone();
            app.clients.insert(client);
        } else {
            app.clients.mark_unavailable(target.route);
        }
        app.poll_chooser_accounts();
        assert!(app.inference.account_load.is_none());
        assert!(app.account_host(target.route).is_none());
        let picker = app.inference.picker.as_ref().unwrap();
        assert_eq!(picker.original.model, "explicit-draft-model");
        assert_eq!(picker.notice, "retained draft review");
        let saved = &app.new_drafts[&id].saved;
        assert!(saved.account_host.is_none());
        assert!(saved.account_settings.is_none());
        assert!(saved.start.is_none());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
}
