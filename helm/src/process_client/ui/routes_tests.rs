use super::super::coverage_support;
use super::*;

#[test]
fn activation_generations_preserve_historical_clients_and_stable_display_order() {
    let fixture = tempfile::tempdir().unwrap();
    let local = Client::local(fixture.path().join("local"));
    let remote = Client::access(fixture.path().join("grant"));
    let mut routes = Routes::new(vec![local.clone(), remote.clone()]);
    let first = routes.first_route().unwrap();
    let second = routes.routes().nth(1).unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(
        routes.iter().map(Client::id).collect::<Vec<_>>(),
        vec![local.id(), remote.id()]
    );
    assert!(routes.available(first));
    routes.mark_unavailable(first);
    assert!(routes.current(first));
    assert!(!routes.available(first));
    routes.mark_available(first);
    assert!(routes.available(first));
    let replacement = routes.insert(local.clone());
    assert_eq!(replacement.id, first.id);
    assert_eq!(replacement.generation, first.generation + 1);
    assert!(!routes.current(first));
    assert!(!routes.available(first));
    assert_eq!(routes[first].generation(), first.generation);
    assert_eq!(routes[replacement].generation(), replacement.generation);
    assert_eq!(
        routes.routes().collect::<Vec<_>>(),
        vec![replacement, second]
    );
    routes.deactivate(first.id);
    assert_eq!(routes.first_route(), Some(second));
    assert_eq!(
        routes.local_client().unwrap().generation(),
        replacement.generation
    );
    routes.deactivate(Uuid::new_v4());
    let next = routes.insert(local);
    assert_eq!(next.generation, replacement.generation + 1);
    assert_eq!(routes.first_route(), Some(next));
    routes.deactivate(second.id);
    routes.deactivate(next.id);
    assert_eq!(routes.len(), 0);
    assert_eq!(routes.first_route(), None);
    assert_eq!(routes.iter().count(), 0);
}

#[tokio::test]
async fn disconnect_aborts_only_owned_observers_and_retains_composer_and_views() {
    let (_fixture, mut app, target) = coverage_support::app();
    let other = Client::local(std::path::PathBuf::from("/synthetic-other-route"));
    let other_route = app.clients.insert(other);
    let owned = tokio::spawn(std::future::pending::<()>());
    let unrelated = tokio::spawn(std::future::pending::<()>());
    app.observers.insert(target.route.id, owned);
    app.observers.insert(other_route.id, unrelated);
    app.route_tasks.insert(
        target.route.id,
        vec![tokio::spawn(std::future::pending::<()>())],
    );
    app.disconnect_connection(target.route.id);
    assert!(!app.clients.current(target.route));
    assert!(app.clients.current(other_route));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.views[&target].connection_unavailable);
    assert!(
        app.views[&target]
            .error
            .as_ref()
            .unwrap()
            .contains("remote work continues")
    );
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.retired_observers.len(), 2);
    assert!(app.pending_disconnects.contains(&target.route.id));
    assert!(app.observers.contains_key(&other_route.id));
    for job in app.retired_observers.drain(..) {
        assert!(job.await.unwrap_err().is_cancelled());
    }
    app.observers.remove(&other_route.id).unwrap().abort();
}

#[tokio::test]
async fn retry_unavailable_route_keeps_identity_and_does_not_duplicate_live_observer() {
    let (_fixture, mut app, target) = coverage_support::app();
    let client = app.clients[target.route].clone();
    app.clients.mark_unavailable(target.route);
    app.retry_client(client.clone());
    let id = app.observers[&target.route.id].id();
    app.retry_client(client);
    assert_eq!(app.observers[&target.route.id].id(), id);
    assert_eq!(app.clients.first_route(), Some(target.route));
    assert!(app.pending_activations.is_empty());
    let job = app.observers.remove(&target.route.id).unwrap();
    job.abort();
    let _ = job.await;
}

#[tokio::test]
async fn activation_limit_refuses_new_connection_without_disturbing_existing_routes() {
    let (_fixture, mut app, target) = coverage_support::app();
    for index in 0..31 {
        app.clients
            .insert(Client::local(format!("/synthetic-route-{index}").into()));
    }
    app.activate_client(Client::local("/synthetic-route-over-limit".into()));
    assert!(app.status.contains("At most 32"));
    assert_eq!(app.clients.len(), 32);
    assert!(app.clients.current(target.route));
    assert!(app.pending_activations.is_empty());
    let client = app.clients[target.route].clone();
    app.activate_client(client);
    assert_eq!(app.clients.len(), 31);
    assert!(app.pending_activations.contains_key(&target.route.id));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}
