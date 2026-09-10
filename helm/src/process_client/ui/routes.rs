//! Stable activation identities. Historical clients are retained for exact recovery;
//! disconnected identities are never rebound to a different credential.
use super::{App, Client, Route, observe};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Index,
};
use uuid::Uuid;

pub(super) struct Routes {
    clients: BTreeMap<Route, Client>,
    active: BTreeMap<Uuid, Route>,
    order: Vec<Uuid>,
    unavailable: BTreeSet<Route>,
}
impl Routes {
    pub fn new(clients: Vec<Client>) -> Self {
        let mut routes = Self {
            clients: BTreeMap::new(),
            active: BTreeMap::new(),
            order: Vec::new(),
            unavailable: BTreeSet::new(),
        };
        for client in clients {
            routes.insert(client);
        }
        routes
    }
    pub fn insert(&mut self, client: Client) -> Route {
        let id = client.id();
        let generation = self
            .clients
            .keys()
            .filter(|r| r.id == id)
            .map(|r| r.generation)
            .max()
            .map_or(client.generation(), |g| g.saturating_add(1));
        let client = client.with_generation(generation);
        let route = Route::of(&client);
        if !self.order.contains(&id) {
            self.order.push(id);
        }
        self.clients.insert(route, client);
        self.active.insert(id, route);
        route
    }
    pub fn routes(&self) -> impl Iterator<Item = Route> + '_ {
        self.order
            .iter()
            .filter_map(|id| self.active.get(id).copied())
    }
    pub fn iter(&self) -> impl Iterator<Item = &Client> {
        self.routes().map(|r| &self.clients[&r])
    }
    pub fn local_client(&self) -> Option<Client> {
        self.clients
            .values()
            .rev()
            .find(|client| client.is_local())
            .cloned()
    }
    pub fn first_route(&self) -> Option<Route> {
        self.routes().next()
    }
    pub fn current(&self, route: Route) -> bool {
        self.active.get(&route.id) == Some(&route)
    }
    pub fn available(&self, route: Route) -> bool {
        self.current(route) && !self.unavailable.contains(&route)
    }
    pub fn mark_unavailable(&mut self, route: Route) {
        self.unavailable.insert(route);
    }
    pub fn mark_available(&mut self, route: Route) {
        self.unavailable.remove(&route);
    }
    pub fn deactivate(&mut self, id: Uuid) {
        self.active.remove(&id);
    }
    pub fn len(&self) -> usize {
        self.active.len()
    }
}
impl Index<Route> for Routes {
    type Output = Client;
    fn index(&self, route: Route) -> &Client {
        &self.clients[&route]
    }
}
impl App {
    pub(super) fn start_observers(&mut self) {
        for route in self.clients.routes() {
            if self.clients[route].is_local()
                && let Some(manager) = &self.vessels
            {
                manager.borrow_mut().set_local(route.id);
            }
            self.vessel_state(route, super::vessels::ConnectionState::Connecting);
            self.observers.insert(
                route.id,
                observe::spawn(self.clients[route].clone(), route, self.sender.clone()),
            );
        }
    }
    pub(super) fn disconnect_connection(&mut self, id: Uuid) {
        if let Some(route) = self.clients.routes().find(|route| route.id == id) {
            self.vessel_state(route, super::vessels::ConnectionState::Offline);
        }
        self.clients.deactivate(id);
        self.pending_disconnects.insert(id);
        self.pending_activations.remove(&id);
        if let Some(jobs) = self.route_tasks.remove(&id) {
            for job in jobs {
                job.abort();
                self.retired_observers.push(job);
            }
        }
        if let Some(job) = self.observers.remove(&id) {
            job.abort();
            self.retired_observers.push(job);
        }
        self.terminal_request = self
            .terminal_request
            .take()
            .filter(|(t, ..)| t.route.id != id);
        for (target, view) in &mut self.views {
            if target.route.id == id {
                view.connection_unavailable = true;
                view.error = Some(
                    "Disconnected · remote work continues; drafts and pending receipts retained"
                        .into(),
                );
            }
        }
        self.status =
            "Disconnected observation only. Remote work continues; no input will be replayed."
                .into();
    }
    pub(super) fn retry_client(&mut self, client: Client) {
        let route = self.clients.routes().find(|route| route.id == client.id());
        if let Some(route) = route
            && !self.clients.available(route)
        {
            // Keep the activation and in-flight command observations intact.
            // Retry only the catalogue observer, never a mutation.
            if self
                .observers
                .get(&route.id)
                .is_some_and(|job| !job.is_finished())
            {
                return;
            }
            if let Some(job) = self.observers.remove(&route.id) {
                self.retired_observers.push(job);
            }
            self.vessel_state(route, super::vessels::ConnectionState::Connecting);
            self.observers.insert(
                route.id,
                observe::spawn(self.clients[route].clone(), route, self.sender.clone()),
            );
            return;
        }
        self.activate_client(client);
    }
    pub(super) fn activate_client(&mut self, client: Client) {
        let id = client.id();
        if self.clients.len() >= 32 && !self.clients.routes().any(|route| route.id == id) {
            self.status = "At most 32 active Vessels. Disconnect another connection first; its recovery records will remain.".into();
            return;
        }
        self.disconnect_connection(id);
        self.pending_activations.insert(id, client);
        self.status = "Connecting · waiting for previous observation cleanup".into();
    }
    fn finish_activation(&mut self, client: Client) {
        let id = client.id();
        let route = self.clients.insert(client);
        self.vessel_state(route, super::vessels::ConnectionState::Connecting);
        if let Err(error) = self.reactivate_drafts(route) {
            self.status = format!(
                "Draft recovery retained: {}",
                super::safe(&error.to_string())
            );
        }
        // Rebind presentation only within the same immutable connection identity.
        // Exact pending command envelopes and draft contents are never changed.
        let old = self
            .views
            .keys()
            .filter(|t| t.route.id == id)
            .copied()
            .collect::<Vec<_>>();
        for target in old {
            if let Some(view) = self.views.remove(&target) {
                let next = super::Target {
                    route,
                    session: target.session,
                };
                self.views.insert(next, view);
                if self.selected == Some(target) {
                    self.selected = Some(next);
                }
            }
        }
        self.observers.insert(
            id,
            observe::spawn(self.clients[route].clone(), route, self.sender.clone()),
        );
        self.status = "Connecting · observing only; no command or terminal input replay".into();
    }
    pub(super) fn observe_retired(&mut self) {
        let mut pending = Vec::new();
        for job in self.retired_observers.drain(..) {
            if job.is_finished() {
                // Finished aborts have released their owned streams and nested JoinSets.
                use futures_util::FutureExt;
                let _ = job.now_or_never();
            } else {
                pending.push(job);
            }
        }
        self.retired_observers = pending;
        for jobs in self.route_tasks.values_mut() {
            let mut running = Vec::new();
            for job in jobs.drain(..) {
                if job.is_finished() {
                    self.retired_observers.push(job);
                } else {
                    running.push(job);
                }
            }
            *jobs = running;
        }
        if self.retired_observers.is_empty() {
            for id in std::mem::take(&mut self.pending_disconnects) {
                if let Err(error) = self.cancel_new_drafts(id) {
                    self.status = format!(
                        "Draft recovery retained: {}",
                        super::safe(&error.to_string())
                    );
                }
            }
            for (_, client) in std::mem::take(&mut self.pending_activations) {
                self.finish_activation(client);
            }
        }
    }
}

/// Retain authenticated mutation observations independently of ephemeral UI generations.
/// This is a recovery record, not an instruction to replay any request.
pub(super) fn retain_receipt(
    target: super::Target,
    command_id: Uuid,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::io::Write;
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-command-receipts");
    std::fs::create_dir_all(root.parent().context("receipt parent")?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    crate::process_client::local::check_private_directory(&root)?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "connection_id": target.route.id, "activation_generation": target.route.generation,
        "session_id": target.session, "command_id": command_id, "receipt": value,
    }))?;
    ensure!(
        bytes.len() <= voyage_protocol::vessel::MAX_VESSEL_BODY,
        "receipt exceeds retention limit"
    );
    let mut file = tempfile::NamedTempFile::new_in(&root)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(root.join(format!(
        "{}-{}-{command_id}.json",
        target.route.id, target.session
    )))?;
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
