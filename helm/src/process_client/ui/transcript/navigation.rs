//! Read-only sender navigation. Never restores archives or launches an executor.
use super::super::{
    App, Client, drafts,
    observe::Update,
    state::{Message, Route, Snapshot, Target, View},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::{
    coordination::CoordinationSource,
    vessel::{ProcessInfo, VesselCommand, VoyageCommand},
};

pub(in crate::process_client::ui) struct Located {
    target: Target,
    process: ProcessInfo,
    snapshot: Snapshot,
    messages: Vec<Message>,
    call: String,
    group: usize,
}

async fn locate(
    clients: Vec<(Route, Client)>,
    source: CoordinationSource,
    origin: Target,
) -> Result<Located> {
    let mut matches = Vec::new();
    for (route, client) in clients {
        let identity = match client.managed() {
            Some(connection) => Some(connection.vessel_id),
            None => {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    client.request(VesselCommand::Capabilities),
                )
                .await;
                result
                    .ok()
                    .and_then(Result::ok)
                    .and_then(|v| serde_json::from_value::<Uuid>(v["vessel_id"].clone()).ok())
            }
        };
        if identity == Some(source.vessel_id) {
            matches.push((route, client));
        }
    }
    ensure!(
        !matches.is_empty(),
        "Sender Vessel is not connected or its identity is unavailable"
    );
    let chosen = matches.iter().position(|(route, _)| *route == origin.route);
    ensure!(
        chosen.is_some() || matches.len() == 1,
        "Multiple connections reach the sender Vessel; disconnect extra routes in Vessels and retry"
    );
    let (route, client) = matches.swap_remove(chosen.unwrap_or(0));
    let target = Target {
        route,
        session: source.session_id,
    };
    let value = client
        .request(VesselCommand::Inspect {
            session_id: source.session_id,
        })
        .await
        .context("Sender voyage is unavailable or access was denied")?;
    let process: ProcessInfo =
        serde_json::from_value(value).context("Sender voyage is not visible on this connection")?;
    ensure!(
        process.session_id == source.session_id,
        "Sender identity changed"
    );
    ensure!(process.deletion.is_none(), "Sender voyage was deleted");
    ensure!(
        process.archive.is_none(),
        "Sender voyage is archived; restore it explicitly before opening its history"
    );
    let snapshot: Snapshot = serde_json::from_value(
        client
            .voyage(target.session, process.incarnation, VoyageCommand::Snapshot)
            .await
            .context("Sender history is unavailable or access was denied")?,
    )?;
    // The existing loader pages at 128 messages, hydrates large tool arguments,
    // fences the revision and enforces 16K-message / 64MiB reading limits.
    let messages = super::history::load(
        &client,
        target,
        process.incarnation,
        snapshot.revision,
        0,
        snapshot.total_messages,
    )
    .await
    .context(
        "Sender history could not be loaded (changed, unavailable, or reading limit reached)",
    )?;
    let mut group = None;
    let mut found = Vec::new();
    for message in &messages {
        if matches!(message.role.as_str(), "user" | "assistant")
            && (!message.content.is_empty() || !message.parts.is_empty())
        {
            group = None;
        }
        for call in &message.tool_calls {
            let group = *group.get_or_insert(message.message_index);
            if message.role == "assistant"
                && call.name == "vessel"
                && call.id == source.tool_call_id
                && matches!(
                    call.arguments["action"].as_str(),
                    Some("create" | "submit" | "steer")
                )
                && call.arguments["command_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok())
                    == Some(source.command_id)
            {
                found.push((call.id.clone(), group));
            }
        }
        if snapshot
            .turns
            .iter()
            .any(|t| t.message_end == Some(message.message_index + 1))
        {
            group = None;
        }
    }
    ensure!(
        !found.is_empty(),
        "The sending tool call is not in the sender's canonical history"
    );
    ensure!(
        found.len() == 1,
        "More than one tool call uses this command identity; refusing an ambiguous jump"
    );
    let (call, group) = found.pop().unwrap();
    Ok(Located {
        target,
        process,
        snapshot,
        messages,
        call,
        group,
    })
}

impl App {
    pub(in crate::process_client::ui) fn open_coordination(&mut self, source: CoordinationSource) {
        let Some(origin) = self.selected else { return };
        let request = Uuid::new_v4();
        self.coordination_request = Some(request);
        self.status = "Opening the sending tool call…".into();
        let clients = self
            .clients
            .routes()
            .filter(|r| self.clients.available(*r))
            .map(|r| (r, self.clients[r].clone()))
            .collect();
        let sender = self.sender.clone();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                locate(clients, source, origin),
            )
            .await
            .map_err(|_| "Sender navigation timed out; no session was changed".to_owned())
            .and_then(|r| r.map(Box::new).map_err(|e| e.to_string()));
            let _ = sender
                .send(Update::Coordination {
                    origin,
                    request,
                    result,
                })
                .await;
        });
        self.route_tasks
            .entry(origin.route.id)
            .or_default()
            .push(task);
    }

    pub(in crate::process_client::ui) fn coordination_arrived(
        &mut self,
        origin: Target,
        request: Uuid,
        result: Result<Box<Located>, String>,
    ) {
        if self.coordination_request != Some(request) {
            return;
        }
        self.coordination_request = None;
        if self.selected != Some(origin)
            || self.active_draft.is_some()
            || !self.clients.available(origin.route)
        {
            return;
        }
        let located = match result {
            Ok(located) => located,
            Err(error) => {
                self.status = crate::process_client::safe(&error);
                return;
            }
        };
        let Located {
            target,
            process,
            snapshot,
            messages,
            call,
            group,
        } = *located;
        if !self.clients.available(target.route) {
            self.status = "Sender connection changed; click the link again".into();
            return;
        }
        if self.views.get(&target).is_some_and(|v| {
            v.process.incarnation != process.incarnation
                || v.snapshot
                    .as_ref()
                    .is_some_and(|s| s.revision > snapshot.revision)
        }) {
            self.status = "Sender conversation changed; click the link again".into();
            return;
        }
        if !self.views.contains_key(&target) {
            let mut view = View::new(process);
            if drafts::load(&self.clients[target.route], &mut view).is_err() {
                self.status =
                    "Could not load the sender's saved draft; navigation cancelled".into();
                return;
            }
            self.views.insert(target, view);
        }
        let view = self.views.get_mut(&target).unwrap();
        let mut state = view.transcript.borrow_mut();
        state.messages = messages;
        state.loaded_revision = Some(snapshot.revision);
        state.requested_from = None;
        state.loading = false;
        state.attempted = None;
        state.attempted_from = None;
        state.expanded.insert(group, true);
        state.tool_expanded.insert(call.clone());
        state.anchor = Some(super::Anchor {
            key: super::Key::Tool(call),
            offset: 0,
        });
        state.new_output = false;
        state.search = None;
        state.query.clear();
        state.search_next = false;
        state.error = None;
        state.dirty = true;
        drop(state);
        view.snapshot = Some(snapshot);
        view.panel = None;
        view.terminals.open = false;
        view.terminals.clear_displayed();
        self.selected = Some(target);
        self.vessel_filter = None;
        self.archives = false;
        self.sidebar.focus = super::super::sidebar::Focus::Voyages;
        self.interactions.borrow_mut().focused = false;
        self.status = "Opened the sending tool call".into();
    }
}
