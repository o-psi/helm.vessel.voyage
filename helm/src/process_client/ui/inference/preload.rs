//! Selected-destination metadata only: bounded, ephemeral, never credentials.
use super::super::Route;
use super::*;
use std::time::{Duration, Instant};

const CAPACITY: usize = 8;
const FRESH: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq)]
pub(super) struct Scope {
    destination: Destination,
    pub(super) route: Route,
    workspace: std::path::PathBuf,
    client: (super::super::super::connections::ConnectionId, u64),
    socket: super::super::super::duplex::ConnectionState,
    incarnation: Option<Uuid>,
    settings: Settings,
    account: Option<voyage_protocol::accounts::AccountBinding>,
    provider: String,
}
struct Entry {
    scope: Scope,
    result: std::result::Result<serde_json::Value, String>,
    expires: Option<Instant>,
}
pub(super) struct Job {
    scope: Scope,
    pub(super) receiver:
        tokio::sync::oneshot::Receiver<std::result::Result<serde_json::Value, String>>,
    task: Option<tokio::task::JoinHandle<()>>,
    abort: tokio::task::AbortHandle,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
#[derive(Default)]
pub(super) struct Cache {
    entries: std::collections::VecDeque<Entry>,
    job: Option<Job>,
    pub foreground: Option<(Uuid, Scope)>,
}
impl Cache {
    fn peek(&self, scope: &Scope) -> Option<&Entry> {
        self.entries.iter().find(|e| &e.scope == scope)
    }
    fn get(&self, scope: &Scope) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| &e.scope == scope && e.expires.is_none_or(|t| t > Instant::now()))
    }
    fn store(&mut self, scope: Scope, result: &std::result::Result<serde_json::Value, String>) {
        // Bound both entry count and retained payload. Oversized catalogues are
        // failures for warming, not repeated background requests.
        let value = result
            .as_ref()
            .ok()
            .filter(|v| {
                let payload = v.get("value").unwrap_or(v);
                let payload = payload.get("inventory").unwrap_or(payload);
                v.to_string().len() <= 1024 * 1024
                    && serde_json::from_value::<Vec<crate::provider::ModelInfo>>(payload.clone())
                        .is_ok_and(|models| crate::provider::validate_models(&models, &[]).is_ok())
            })
            .cloned();
        let expires = value.as_ref().map(|_| Instant::now() + FRESH);
        self.entries.retain(|e| e.scope != scope);
        while self.entries.len() >= CAPACITY {
            self.entries.pop_front();
        }
        let result = value.ok_or_else(|| {
            result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| "Model catalog unavailable".into())
        });
        self.entries.push_back(Entry {
            scope,
            result,
            expires,
        });
    }
}
impl App {
    pub(in crate::process_client::ui) fn cancel_model_preload(&mut self) {
        if let Some(mut job) = self.inference.cache.job.take() {
            job.abort.abort();
            if let Some(task) = job.task.take() {
                self.retired_observers.push(task);
            }
        }
    }

    pub(super) fn model_scope(
        &self,
        destination: Destination,
        account: Option<voyage_protocol::accounts::AccountBinding>,
        provider: String,
    ) -> Result<Scope> {
        let (route, workspace) = self.account_destination(destination)?;
        ensure!(self.clients.available(route), "Destination unavailable");
        let client = &self.clients[route];
        Ok(Scope {
            destination,
            route,
            workspace,
            client: (client.id(), client.generation()),
            socket: *client.connection_state().borrow(),
            incarnation: match destination {
                Destination::Live(t) => Some(self.views[&t].process.incarnation),
                Destination::Draft(_) => None,
            },
            settings: self.inference_settings(destination)?,
            account,
            provider,
        })
    }
    pub(super) fn model_scope_current(&self, scope: &Scope) -> bool {
        self.model_scope(
            scope.destination,
            scope.account.clone(),
            scope.provider.clone(),
        )
        .is_ok_and(|s| s == *scope)
    }
    pub(super) fn model_command(
        &self,
        scope: &Scope,
    ) -> Result<voyage_protocol::vessel::VesselCommand> {
        use voyage_protocol::vessel::{VesselCommand, VoyageRequest};
        if let Some(account) = scope.account.clone() {
            return Ok(VesselCommand::AccountModels {
                workspace: scope.workspace.clone(),
                account,
            });
        }
        let Destination::Live(t) = scope.destination else {
            anyhow::bail!("Choose an account before loading models")
        };
        Ok(VesselCommand::Voyage(VoyageRequest {
            session_id: t.session,
            incarnation: scope.incarnation,
            command: VoyageCommand::Controls {
                run_id: self.views[&t]
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.run.as_ref())
                    .filter(|r| r.active())
                    .map(|r| r.run_id),
                section: "models".into(),
            },
        }))
    }
    // Warm data uses the same installation path as a fresh foreground response.
    // A negative cache entry suppresses automatic work, never an explicit Retry.
    pub(super) fn take_model_preload(&mut self, scope: &Scope) -> Option<Job> {
        if !self
            .inference
            .cache
            .job
            .as_ref()
            .is_some_and(|job| job.scope == *scope)
        {
            return None;
        }
        let mut job = self.inference.cache.job.take().unwrap();
        if let Some(task) = job.task.take() {
            self.retired_observers.push(task);
        }
        Some(job)
    }
    pub(super) fn use_warm_models(&mut self) -> bool {
        let Some(p) = self.inference.picker.as_ref() else {
            return false;
        };
        let account = if p.field == Field::Model {
            p.chooser.account.clone()
        } else {
            p.original.account.clone()
        };
        let provider = if p.field == Field::Model {
            p.chooser.provider.clone()
        } else {
            p.original.provider.clone()
        };
        let Ok(scope) = self.model_scope(p.destination, account, provider) else {
            return false;
        };
        let Some(result) = self.inference.cache.peek(&scope).map(|e| e.result.clone()) else {
            return false;
        };
        let id = p.id;
        let generation = if let Destination::Draft(d) = p.destination {
            self.inference.draft_generations.insert(d, id);
            Some(id)
        } else {
            None
        };
        self.inference_models(id, None, generation, result);
        true
    }
    pub(super) fn warm_selected_models(&mut self) {
        let selected = self
            .active_draft
            .map(Destination::Draft)
            .or(self.selected.map(Destination::Live));
        if let Some(mut job) = self.inference.cache.job.take() {
            if selected != Some(job.scope.destination) || !self.model_scope_current(&job.scope) {
                job.abort.abort();
                if let Some(task) = job.task.take() {
                    self.retired_observers.push(task);
                }
            } else {
                match job.receiver.try_recv() {
                    Ok(result) => {
                        self.inference.cache.store(job.scope.clone(), &result);
                        if let Some(task) = job.task.take() {
                            self.retired_observers.push(task);
                        }

                        // Only update a chooser still displaying this exact context.
                        if result.is_ok()
                            && self.inference.picker.as_ref().is_some_and(|p| {
                                p.destination == job.scope.destination
                                    && !p.loading
                                    && !p.chooser.accounts_loading
                            })
                        {
                            let choice = self.inference.picker.as_ref().map(|p| p.chooser.clone());
                            self.use_warm_models();
                            if let (Some(p), Some(choice)) =
                                (self.inference.picker.as_mut(), choice)
                            {
                                p.chooser = choice;
                            }
                        }
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        self.inference.cache.job = Some(job)
                    }
                    Err(_) => {
                        if let Some(task) = job.task.take() {
                            self.retired_observers.push(task);
                        }
                        self.inference
                            .cache
                            .store(job.scope.clone(), &Err("Model preload stopped".into()));
                    }
                }
            }
        }
        // Automatic account hydration must settle before warming an unresolved
        // draft. Never create another picker or bypass host default resolution.
        if self.inference.cache.job.is_some()
            || self.inference.catalog_job.is_some()
            || self.inference.account_load.is_some()
        {
            return;
        }
        let Some(destination) = selected else { return };
        let Ok(settings) = self.inference_settings(destination) else {
            return;
        };
        if matches!(destination, Destination::Draft(_)) && settings.account.is_none() {
            return;
        }
        let Ok(scope) = self.model_scope(
            destination,
            settings.account.clone(),
            settings.provider.clone(),
        ) else {
            return;
        };
        if self.inference.cache.get(&scope).is_some() {
            return;
        }
        let Ok(command) = self.model_command(&scope) else {
            return;
        };
        let client = self.clients[scope.route].clone();
        let account = scope.account.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = fetch(client, command, account).await;
            let _ = sender.send(result);
        });
        self.inference.cache.job = Some(Job {
            scope,
            receiver,
            abort: task.abort_handle(),
            task: Some(task),
        });
    }
    pub(super) fn cache_model_response(
        &mut self,
        id: Uuid,
        result: &std::result::Result<serde_json::Value, String>,
    ) -> bool {
        let Some((request, scope)) = self.inference.cache.foreground.take() else {
            return true;
        };
        if request != id {
            self.inference.cache.foreground = Some((request, scope));
            return false;
        }
        if !self.model_scope_current(&scope) {
            return false;
        }
        self.inference.cache.store(scope, result);
        true
    }
}
pub(super) async fn fetch(
    client: super::super::super::transport::Client,
    command: voyage_protocol::vessel::VesselCommand,
    account: Option<voyage_protocol::accounts::AccountBinding>,
) -> std::result::Result<serde_json::Value, String> {
    tokio::time::timeout(Duration::from_secs(12), client.request(command))
        .await
        .map_err(|_| "Model loading timed out".to_owned())
        .and_then(|v| {
            v.map_err(|e| {
                voyage_protocol::model_discovery::Failure::from_diagnostic(&e.to_string())
                    .map(|f| format!("[model_catalog:{}]", f.code()))
                    .unwrap_or_else(|| "Model catalog unavailable".into())
            })
        })
        .and_then(|v| catalog_payload(v, account.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            destination: Destination::Draft(Uuid::new_v4()),
            route: super::super::super::state::Route {
                id: Uuid::new_v4(),
                generation: 1,
            },
            workspace: "/fixture".into(),
            client: (Uuid::new_v4(), 1),
            socket: super::super::super::super::duplex::ConnectionState {
                socket_id: Some(Uuid::new_v4()),
                loss_generation: 0,
            },
            incarnation: Some(Uuid::new_v4()),
            settings: Settings::default(),
            account: Some(
                serde_json::from_value(serde_json::json!({
                    "account_id": Uuid::new_v4(), "connection_id": Uuid::new_v4(),
                    "identity_generation": 1, "connection_revision": 1, "transport": "openai_chat"
                }))
                .unwrap(),
            ),
            provider: "openai-chat".into(),
        }
    }
    #[test]
    fn every_context_component_isolated_and_failures_do_not_expire() {
        let original = scope();
        let mut cache = Cache::default();
        cache.store(original.clone(), &Err("[model_catalog:unavailable]".into()));
        assert!(cache.get(&original).unwrap().expires.is_none());
        let mut variants = Vec::new();
        let mut s = original.clone();
        s.destination = Destination::Draft(Uuid::new_v4());
        variants.push(s);
        let mut s = original.clone();
        s.route.generation += 1;
        variants.push(s);
        let mut s = original.clone();
        s.workspace = "/other".into();
        variants.push(s);
        let mut s = original.clone();
        s.client.1 += 1;
        variants.push(s);
        let mut s = original.clone();
        s.socket.socket_id = Some(Uuid::new_v4());
        variants.push(s);
        let mut s = original.clone();
        s.socket.loss_generation += 1;
        variants.push(s);
        let mut s = original.clone();
        s.incarnation = Some(Uuid::new_v4());
        variants.push(s);
        let mut s = original.clone();
        s.settings.model = "other".into();
        variants.push(s);
        let mut s = original.clone();
        s.provider = "anthropic".into();
        variants.push(s);
        let mut s = original.clone();
        s.account.as_mut().unwrap().identity_generation += 1;
        variants.push(s);
        let mut s = original.clone();
        s.account.as_mut().unwrap().connection_revision += 1;
        variants.push(s);
        let mut s = original.clone();
        s.account.as_mut().unwrap().connection_id = Uuid::new_v4();
        variants.push(s);
        let mut s = original.clone();
        s.account.as_mut().unwrap().account_id = Uuid::new_v4();
        variants.push(s);
        let mut s = original.clone();
        s.account.as_mut().unwrap().transport = voyage_protocol::accounts::Transport::Anthropic;
        variants.push(s);
        for variant in variants {
            assert!(cache.get(&variant).is_none());
        }
        for _ in 0..CAPACITY {
            cache.store(scope(), &Err("unavailable".into()));
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert!(cache.get(&original).is_none());
    }
    #[test]
    fn expired_success_and_oversized_payload_are_not_reused_as_metadata() {
        let mut cache = Cache::default();
        let scope = scope();
        cache.store(scope.clone(), &Ok(serde_json::json!([])));
        assert!(cache.get(&scope).unwrap().result.is_ok());
        cache.entries.front_mut().unwrap().expires = Some(Instant::now());
        assert!(cache.get(&scope).is_none());
        assert!(cache.peek(&scope).unwrap().result.is_ok());
        cache.store(
            scope.clone(),
            &Ok(serde_json::json!("x".repeat(1024 * 1024 + 1))),
        );
        assert!(cache.get(&scope).unwrap().result.is_err());
    }
}
