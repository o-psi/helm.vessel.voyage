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
