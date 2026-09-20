//! Voyage-owned browser viewer. Only private socket-bound browser operations cross this adapter.
//! No local Chromium, general executor, payload journal, or HTTP fallback.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::host_browser::{
    HostBrowserBinding, HostBrowserControlMode as Mode, HostBrowserOperation as Op,
};

#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub summary: String,
    pub launcher: Option<PathBuf>,
    pub finished: bool,
}
#[derive(Clone, Copy)]
pub(crate) enum Control {
    Human,
    Private,
    Agent,
    Close,
}
pub(crate) struct Handle {
    initial_incarnation: Uuid,
    owner: Arc<Mutex<(Uuid, u64)>>,
    pub state: watch::Receiver<Status>,
    control: mpsc::Sender<Control>,
    stop: CancellationToken,
    job: Option<tokio::task::JoinHandle<Result<()>>>,
}
impl Handle {
    pub fn start(client: Client, session: Uuid, incarnation: Uuid, revision: u64) -> Self {
        let (tx, state) = watch::channel(Status {
            summary: "Opening executing-host browser viewer".into(),
            launcher: None,
            finished: false,
        });
        let (control, rx) = mpsc::channel(8);
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let owner = Arc::new(Mutex::new((incarnation, revision)));
        let adapter_owner = owner.clone();
        let job = tokio::spawn(async move {
            let result = run(client, session, adapter_owner, cancelled, rx, tx.clone()).await;
            tx.send_modify(|s| {
                s.finished = true;
                s.launcher = None;
                s.summary = if result.is_ok() {
                    "Viewer detached; host browser is not implicitly closed"
                } else {
                    "Viewer unavailable or socket lost; unknown effects are not replayed"
                }
                .into();
            });
            result
        });
        Self {
            initial_incarnation: incarnation,
            owner,
            state,
            control,
            stop,
            job: Some(job),
        }
    }
    /// The catalogue can lag a verified preparation. Accept only the original
    /// observation or our explicitly prepared owner, never an arbitrary restart.
    pub fn accepts_incarnation(&self, observed: Uuid) -> bool {
        observed == self.initial_incarnation || observed == self.owner.lock().unwrap().0
    }
    pub fn control(&self, control: Control) -> Result<()> {
        self.control
            .try_send(control)
            .map_err(|_| anyhow::anyhow!("Browser control queue unavailable"))
    }
    pub fn stop(&self) {
        self.stop.cancel();
    }
    pub fn finished(&self) -> bool {
        self.job.as_ref().is_none_or(|j| j.is_finished())
    }
    pub async fn finish(&mut self) -> Result<()> {
        self.stop();
        if let Some(job) = self.job.take() {
            job.await.context("Viewer cleanup task failed")??;
        }
        Ok(())
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Adapter {
    client: Client,
    session: Uuid,
    owner: Arc<Mutex<(Uuid, u64)>>,
    socket: Uuid,
    origin: String,
    host: String,
    launch: Mutex<Option<String>>,
    secret: String,
    csrf: String,
    gate: tokio::sync::Mutex<()>,
    binding: Mutex<Option<HostBrowserBinding>>,
    stop: CancellationToken,
    status: watch::Sender<Status>,
}
impl Adapter {
    fn headers_ok(&self, headers: &HeaderMap) -> bool {
        headers.get(header::HOST).and_then(|v| v.to_str().ok()) == Some(self.host.as_str())
            && headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
                == Some(self.origin.as_str())
    }
    fn authenticated(&self, headers: &HeaderMap) -> bool {
        self.headers_ok(headers)
            && headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                == Some(self.secret.as_str())
            && headers.get("x-helm-csrf").and_then(|v| v.to_str().ok()) == Some(self.csrf.as_str())
    }
    async fn dispatch(&self, mut op: Op) -> Result<Value> {
        let incarnation = self.owner.lock().unwrap().0;
        let (value, owner) = self
            .client
            .host_browser_observed(self.socket, self.session, incarnation, op.clone())
            .await?;
        if !super::transport::browser_prepared(&value) {
            return Ok(value);
        }
        let revision = self
            .client
            .host_browser_revision(self.socket, self.session, owner)
            .await?;
        // Exact intent was explicitly NOT dispatched or admitted. Keep its ID;
        // only replace the owner/revision preconditions after observing preparation.
        rebind_prepared(&mut op, owner, revision)?;
        *self.owner.lock().unwrap() = (owner, revision);
        let value = self
            .client
            .host_browser(self.socket, self.session, owner, op)
            .await?;
        ensure!(
            !super::transport::browser_prepared(&value),
            "Repeated browser preparation; not replayed"
        );
        Ok(value)
    }
    async fn exchange(&self, op: Op) -> Result<Value> {
        let incarnation = self.owner.lock().unwrap().0;
        ensure!(!self.stop.is_cancelled(), "Viewer detached");
        ensure!(op.valid(), "Invalid browser operation");
        ensure!(
            op.binding().is_none_or(|b| b.incarnation == incarnation),
            "Browser owner mismatch"
        );
        if let Op::Start { incarnation, .. } = &op {
            ensure!(
                *incarnation == self.owner.lock().unwrap().0,
                "Browser owner mismatch"
            );
        }
        let result = self.dispatch(op).await;
        // Unknown outcomes poison this viewer; opening a new viewer is an explicit human action.
        if result.is_err() {
            self.stop.cancel();
        }
        let value = result?;
        let status = &value["status"];
        let binding = serde_json::from_value::<HostBrowserBinding>(status["binding"].clone())
            .ok()
            .filter(|b| b.incarnation == self.owner.lock().unwrap().0 && b.valid());
        *self.binding.lock().unwrap() = binding;
        let mode = match status["mode"].as_str() {
            Some("human") => "human",
            Some("private") => "private",
            Some("agent") => "agent",
            _ => "unknown",
        };
        let running = status["running"].as_bool().unwrap_or(false);
        self.status.send_modify(|s| {
            s.summary = format!(
                "Executing-host browser: {}; control: {mode}",
                if running { "running" } else { "not running" }
            )
        });
        Ok(value)
    }
}

fn rebind_prepared(op: &mut Op, owner: Uuid, revision: u64) -> Result<()> {
    match op {
        Op::Status {} => {}
        Op::Start {
            incarnation,
            expected_revision,
            ..
        } => {
            *incarnation = owner;
            *expected_revision = revision;
        }
        _ => anyhow::bail!("Only unbound Status/Start may prepare an owner"),
    }
    Ok(())
}

async fn bootstrap(State(a): State<Arc<Adapter>>, headers: HeaderMap, body: Bytes) -> Response {
    if !a.headers_ok(&headers) || a.stop.is_cancelled() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut launch = a.launch.lock().unwrap();
    if !consume_launch(&mut launch, &body) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (incarnation, revision) = *a.owner.lock().unwrap();
    Json(json!({"authorization":a.secret,"csrf":a.csrf,"incarnation":incarnation,"revision":revision})).into_response()
}
fn consume_launch(launch: &mut Option<String>, supplied: &[u8]) -> bool {
    if launch
        .as_ref()
        .is_some_and(|token| token.as_bytes() == supplied)
    {
        *launch = None;
        true
    } else {
        false
    }
}
async fn operation(State(a): State<Arc<Adapter>>, headers: HeaderMap, body: Bytes) -> Response {
    if !a.authenticated(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(op) = serde_json::from_slice::<Op>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // Revocation and website dialog replies must not wait behind the operation
    // they interrupt. Other effects remain serialized in this local adapter.
    let interrupt = matches!(
        &op,
        Op::Status {}
            | Op::Control { .. }
            | Op::Detach { .. }
            | Op::Close { .. }
            | Op::Input {
                input: voyage_protocol::host_browser::HostBrowserInput::Dialog { .. }
                    | voyage_protocol::host_browser::HostBrowserInput::History {
                        direction: voyage_protocol::host_browser::HostBrowserHistory::Stop
                    },
                ..
            }
    );
    let _guard = if interrupt {
        None
    } else {
        match a.gate.try_lock() {
            Ok(guard) => Some(guard),
            Err(_) => return StatusCode::CONFLICT.into_response(),
        }
    };
    match a.exchange(op).await {
        Ok(value) => {
            let (incarnation, revision) = *a.owner.lock().unwrap();
            Json(json!({"result":value,"context":{"incarnation":incarnation,"revision":revision}}))
                .into_response()
        }
        Err(_) => (
            StatusCode::CONFLICT,
            "Browser operation refused or outcome unknown; not replayed",
        )
            .into_response(),
    }
}
async fn alive(State(a): State<Arc<Adapter>>, headers: HeaderMap) -> StatusCode {
    if a.authenticated(&headers) && !a.stop.is_cancelled() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::FORBIDDEN
    }
}
async fn page(State(a): State<Arc<Adapter>>, headers: HeaderMap) -> Response {
    if headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(a.host.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], PAGE).into_response()
}
const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer"><title>Voyage host browser</title><link rel="stylesheet" href="/viewer.css"><main id="viewer"></main><script type="module" src="/native.mjs"></script>"#;
const SCRIPT: &str = r#"import {mountBrowserViewer} from '/viewer.mjs';
const token=location.hash.slice(1); history.replaceState(null,'',location.pathname);
const root=document.getElementById('viewer');
let viewer;
try {
 const boot=await fetch('/bootstrap',{method:'POST',body:token,credentials:'omit',cache:'no-store'});
 if(!boot.ok) throw Error();
 const auth=await boot.json();
 const headers={'Content-Type':'application/json','Authorization':auth.authorization,'X-Helm-CSRF':auth.csrf};
 let lost=false;
 const disconnect=()=>{lost=true;viewer?.disconnect();};
 const heartbeat=setInterval(async()=>{try{const r=await fetch('/alive',{method:'POST',headers,credentials:'omit',signal:AbortSignal.timeout(1500)});if(!r.ok)throw Error();}catch{clearInterval(heartbeat);disconnect();}},500);
 viewer=mountBrowserViewer(root,{context:()=>({incarnation:auth.incarnation,revision:auth.revision}),transport:async operation=>{
   if(lost) throw Error('Socket lost; explicitly open a fresh viewer');
   try {
   const response=await fetch('/operation',{method:'POST',credentials:'omit',cache:'no-store',headers,body:JSON.stringify(operation)});
   if(!response.ok){viewer?.disconnect();throw Error('Viewer disconnected or operation refused; never replayed');}
   const reply=await response.json();
   Object.assign(auth,reply.context);
   return reply.result;
   } catch(error) {disconnect();throw error;}
 }});
 window.addEventListener('pagehide',()=>{clearInterval(heartbeat);viewer.dispose();},{once:true});
} catch { root.textContent='Viewer unavailable. Return to Helm and explicitly open a fresh viewer. Host browser has not been closed.'; }
"#;

async fn run(
    client: Client,
    session: Uuid,
    owner: Arc<Mutex<(Uuid, u64)>>,
    stop: CancellationToken,
    mut controls: mpsc::Receiver<Control>,
    status: watch::Sender<Status>,
) -> Result<()> {
    let mut connection = client.connection_state();
    let observed = *connection.borrow_and_update();
    let socket = observed
        .socket_id
        .context("Connect this Vessel before opening a viewer")?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let host = listener.local_addr()?.to_string();
    let origin = format!("http://{host}");
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let a = Arc::new(Adapter {
        client,
        session,
        owner,
        socket,
        origin: origin.clone(),
        host,
        launch: Mutex::new(Some(token.clone())),
        secret: format!(
            "Bearer {}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ),
        csrf: Uuid::new_v4().to_string(),
        gate: tokio::sync::Mutex::new(()),
        binding: Mutex::new(None),
        stop: stop.clone(),
        status: status.clone(),
    });
    let router = Router::new().route("/", get(page))
        .route("/alive", post(alive)).route("/bootstrap", post(bootstrap)).route("/operation", post(operation))
        .route("/native.mjs", get(|| async { ([(header::CONTENT_TYPE,"text/javascript")], SCRIPT) }))
        .route("/viewer.mjs", get(|| async { ([(header::CONTENT_TYPE,"text/javascript")], include_str!("../../browser-view/viewer.mjs")) }))
        .route("/capture.mjs", get(|| async { ([(header::CONTENT_TYPE,"text/javascript")], include_str!("../../browser-view/capture.mjs")) }))
        .route("/capture.css", get(|| async { ([(header::CONTENT_TYPE,"text/css")], include_str!("../../browser-view/capture.css")) }))
        .route("/viewer.css", get(|| async { ([(header::CONTENT_TYPE,"text/css")], include_str!("../../browser-view/viewer.css")) }))
        .layer(DefaultBodyLimit::max(384 * 1024))
        .layer(axum::middleware::map_response(|mut response: Response| async move {
            let h = response.headers_mut();
            h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            h.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
            h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
            h.insert(header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; media-src blob:; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'".parse().unwrap());
            response
        })).with_state(a.clone());
    let base = super::cli::default_directory();
    let root = base
        .parent()
        .context("Viewer storage unavailable")?
        .join(format!("host-view-{}", Uuid::new_v4()));
    let _private = crate::attachment::local_actor::storage::Directory::open(&root)?;
    let launcher = super::browser::launcher(&root, &format!("{origin}/#{token}"))?;
    status.send_modify(|s| {
        s.launcher = Some(launcher.clone());
        s.summary =
            "Viewer ready. Start/Connect explicitly in the viewer; browser runs on the Voyage host"
                .into();
    });
    let shutdown = stop.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    });
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                changed = connection.changed() => {
                    let current = *connection.borrow_and_update();
                    if changed.is_err() || current != observed { stop.cancel(); break; }
                }
                command = controls.recv() => {
                    let Some(command) = command else { break; };
                    // Revocation must interrupt a pending page operation/dialog.
                    let binding = a.binding.lock().unwrap().clone();
                    if let Some(binding) = binding {
                        let command_id = Uuid::new_v4();
                        let op = match command { Control::Close => Op::Close {command_id,binding}, _ => Op::Control {command_id,binding,mode:match command { Control::Human=>Mode::Human, Control::Private=>Mode::Private, _=>Mode::Agent }} };
                        a.exchange(op).await?;
                    } else { status.send_modify(|s|s.summary="Connect the viewer before requesting browser control or close".into()); }
                }
                _ = &mut server => { anyhow::bail!("Local viewer server ended"); }
            }
        }
        Ok(())
    }.await;
    stop.cancel();
    // Detach is best effort on the original socket only. Never close the host browser on exit.
    let binding = a.binding.lock().unwrap().clone();
    if let Some(binding) = binding {
        let _ = a
            .client
            .host_browser(
                socket,
                session,
                binding.incarnation,
                Op::Detach {
                    command_id: Uuid::new_v4(),
                    binding,
                },
            )
            .await;
    }
    if !server.is_finished()
        && tokio::time::timeout(Duration::from_secs(2), &mut server)
            .await
            .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    std::fs::remove_file(&launcher).context("Private launcher cleanup failed")?;
    drop(_private);
    std::fs::remove_dir(&root).context("Private viewer directory cleanup failed")?;
    result
}

pub(super) async fn run_connected(client: Client, session: Uuid) -> Result<()> {
    use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VoyageCommand};
    let info: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect {
                session_id: session,
            })
            .await?,
    )?;
    let snapshot = client
        .voyage(session, info.incarnation, VoyageCommand::Snapshot)
        .await?;
    let revision = snapshot["revision"]
        .as_u64()
        .context("Voyage revision unavailable")?;
    let mut handle = Handle::start(client, session, info.incarnation, revision);
    let mut opened = false;
    println!("Voyage host browser viewer. Ctrl+C detaches; it does not close the host browser.");
    loop {
        let state = handle.state.borrow_and_update().clone();
        println!("{}", state.summary);
        if !opened && let Some(path) = state.launcher {
            let _ = super::browser::open_launcher(path).await;
            opened = true;
        }
        if state.finished {
            break;
        }
        tokio::select! { _=tokio::signal::ctrl_c()=>break, changed=handle.state.changed()=>if changed.is_err(){break;} }
    }
    handle.finish().await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter() -> Adapter {
        let (status, _) = watch::channel(Status {
            summary: String::new(),
            launcher: None,
            finished: false,
        });
        Adapter {
            client: Client::local(
                std::env::temp_dir().join(format!("missing-host-view-test-{}", Uuid::new_v4())),
            ),
            session: Uuid::new_v4(),
            owner: Arc::new(Mutex::new((Uuid::new_v4(), 1))),
            socket: Uuid::new_v4(),
            origin: "http://127.0.0.1:12345".into(),
            host: "127.0.0.1:12345".into(),
            launch: Mutex::new(Some("one-use".into())),
            secret: "Bearer test-secret".into(),
            csrf: "test-csrf".into(),
            gate: tokio::sync::Mutex::new(()),
            binding: Mutex::new(None),
            stop: CancellationToken::new(),
            status,
        }
    }
    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        for (key, value) in [
            ("host", "127.0.0.1:12345"),
            ("origin", "http://127.0.0.1:12345"),
            ("authorization", "Bearer test-secret"),
            ("x-helm-csrf", "test-csrf"),
        ] {
            h.insert(
                axum::http::HeaderName::from_static(key),
                value.parse().unwrap(),
            );
        }
        h
    }
    #[test]
    fn preparation_requires_exact_non_admission_and_preserves_start_identity() {
        use super::super::transport::browser_prepared;
        assert!(browser_prepared(
            &json!({"status":"prepared","not_dispatched":true})
        ));
        for value in [
            json!({"status":"prepared"}),
            json!({"not_dispatched":true}),
            json!({"status":"prepared","not_dispatched":"true"}),
            json!({"status":"applied","not_dispatched":true}),
        ] {
            assert!(!browser_prepared(&value));
        }
        let id = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let mut op = Op::Start {
            command_id: id,
            incarnation: Uuid::new_v4(),
            expected_revision: 1,
        };
        rebind_prepared(&mut op, owner, 9).unwrap();
        assert_eq!(
            op,
            Op::Start {
                command_id: id,
                incarnation: owner,
                expected_revision: 9
            }
        );
        let mut status = Op::Status {};
        rebind_prepared(&mut status, owner, 9).unwrap();
        assert_eq!(status, Op::Status {});
        let mut receipt = Op::Receipt { command_id: id };
        assert!(rebind_prepared(&mut receipt, owner, 9).is_err());
    }
    #[test]
    fn exact_host_origin_authorization_and_csrf_are_required() {
        let a = adapter();
        assert!(a.authenticated(&headers()));
        for key in ["host", "origin", "authorization", "x-helm-csrf"] {
            let mut h = headers();
            h.remove(key);
            assert!(!a.authenticated(&h));
            h.insert(
                axum::http::HeaderName::from_static(key),
                "wrong".parse().unwrap(),
            );
            assert!(!a.authenticated(&h));
        }
        let mut h = headers();
        h.insert("origin", "null".parse().unwrap());
        assert!(!a.authenticated(&h));
    }
    #[tokio::test]
    async fn stale_socket_is_refused_without_connecting_or_replaying() {
        let a = adapter();
        assert!(a.exchange(Op::Status {}).await.is_err());
        assert!(a.stop.is_cancelled());
        assert_eq!(a.client.connection_state().borrow().socket_id, None);
        assert!(a.exchange(Op::Status {}).await.is_err());
    }
    #[tokio::test]
    async fn bootstrap_is_origin_checked_and_one_use() {
        let a = Arc::new(adapter());
        let mut h = headers();
        h.remove("origin");
        assert_eq!(
            bootstrap(State(a.clone()), h, Bytes::from_static(b"one-use"))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            bootstrap(State(a.clone()), headers(), Bytes::from_static(b"one-use"))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            bootstrap(State(a), headers(), Bytes::from_static(b"one-use"))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn malformed_or_unauthenticated_operations_have_no_transport_effect() {
        let a = Arc::new(adapter());
        assert_eq!(
            operation(
                State(a.clone()),
                HeaderMap::new(),
                Bytes::from_static(b"{}")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            operation(
                State(a.clone()),
                headers(),
                Bytes::from_static(br#"{"action":"status","socket_id":"spoof"}"#)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(!a.stop.is_cancelled());
        assert_eq!(a.client.connection_state().borrow().socket_id, None);
    }
    #[test]
    fn launcher_is_one_use_and_wrong_token_does_not_consume() {
        let mut token = Some("secret".into());
        assert!(!consume_launch(&mut token, b"wrong"));
        assert!(consume_launch(&mut token, b"secret"));
        assert!(!consume_launch(&mut token, b"secret"));
    }
    #[test]
    fn page_never_contains_bootstrap_credentials_and_has_no_inline_script() {
        assert!(!PAGE.contains("<script>"));
        assert!(SCRIPT.contains("history.replaceState"));
        assert!(SCRIPT.contains("credentials:'omit'"));
    }
}
