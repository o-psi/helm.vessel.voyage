//! Shared synthetic App fixture for the coverage campaign. No real host discovery.
use super::{
    App,
    state::{Target, View},
};
use serde_json::json;
use uuid::Uuid;

pub(super) fn app() -> (super::account_test_support::Fixture, App, Target) {
    let fixture = super::account_test_support::Fixture::new();
    let mut app = super::accounts::app_tests::app(fixture.0.path());
    let target = Target {
        route: app.clients.first_route().unwrap(),
        session: Uuid::new_v4(),
    };
    let mut view = View::new(serde_json::from_value(json!({"session_id":target.session,"incarnation":Uuid::new_v4(),"workspace":"/synthetic-workspace","state":"live","name":"Synthetic voyage"})).unwrap());
    view.snapshot = Some(serde_json::from_value(json!({"session_id":target.session,"revision":17,"model":"synthetic-model","messages":[],"decisions":[]})).unwrap());
    view.draft.insert_str("preserved draft");
    app.views.insert(target, view);
    app.selected = Some(target);
    (fixture, app, target)
}
