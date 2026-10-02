//! Real local Axum adapter and private launcher with a scripted loopback Vessel
//! peer. No Chromium, provider, production credential, or Voyage is launched.
use super::*;
use crate::process_client::loopback_tests::{Peer, response, send};
use std::os::unix::fs::MetadataExt;
use voyage_protocol::{
    duplex::ServerFrame,
    host_browser::{HostBrowserButton, HostBrowserHistory, HostBrowserInput},
    vessel::{VesselCommand, VesselResponse, VoyageCommand, VoyageRequest},
};

const WAIT: Duration = Duration::from_secs(4);

struct Live {
    peer: Peer,
    handle: Handle,
    session: Uuid,
    incarnation: Uuid,
    launcher: PathBuf,
    url: reqwest::Url,
    http: reqwest::Client,
    auth: Option<Value>,
}
impl Live {
    async fn new() -> Self {
        let peer = Peer::open().await;
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let mut handle = Handle::start(peer.client.clone(), session, incarnation, 17);
        let launcher = tokio::time::timeout(WAIT, async {
            loop {
                let state = handle.state.borrow_and_update().clone();
                assert!(!state.finished, "synthetic adapter ended before readiness");
                if let Some(path) = state.launcher {
                    break path;
                }
                handle.state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let text = std::fs::read_to_string(&launcher).unwrap();
        let href = text
            .split_once("href=\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap();
        let url = reqwest::Url::parse(href).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(url.path(), "/");
        assert_eq!(url.fragment().unwrap().len(), 64);
        assert!(url.query().is_none());
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(WAIT)
            .build()
            .unwrap();
        Self {
            peer,
            handle,
            session,
            incarnation,
            launcher,
            url,
            http,
            auth: None,
        }
    }
    fn origin(&self) -> String {
        self.url.origin().ascii_serialization()
    }
    fn request(&self, path: &str) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(self.url.join(path).unwrap())
            .header(header::ORIGIN, self.origin());
        if let Some(auth) = &self.auth {
            request = request
                .header(
                    header::AUTHORIZATION,
                    auth["authorization"].as_str().unwrap(),
                )
                .header("x-helm-csrf", auth["csrf"].as_str().unwrap());
        }
        request
    }
    async fn bootstrap(&mut self) {
        let reply = self
            .request("/bootstrap")
            .body(self.url.fragment().unwrap().to_owned())
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        let auth: Value = reply.json().await.unwrap();
        assert_eq!(auth["incarnation"], json!(self.incarnation));
        assert_eq!(auth["revision"], 17);
        assert_eq!(auth.as_object().unwrap().len(), 4);
        self.auth = Some(auth);
    }
    fn submit(&self, op: &Op) -> tokio::task::JoinHandle<reqwest::Response> {
        let request = self.request("/operation").json(op);
        tokio::spawn(async move { request.send().await.unwrap() })
    }
    async fn command(&mut self) -> (Uuid, Op) {
        let (id, command) = self.peer.command().await;
        let VesselCommand::Voyage(VoyageRequest {
            session_id,
            incarnation,
            command: VoyageCommand::HostBrowser { operation },
        }) = command
        else {
            panic!("only exact HostBrowser traffic is admitted by this fixture");
        };
        assert_eq!(session_id, self.session);
        assert_eq!(incarnation, Some(self.handle.owner.lock().unwrap().0));
        (id, operation)
    }
    fn binding(&self) -> HostBrowserBinding {
        HostBrowserBinding {
            incarnation: self.handle.owner.lock().unwrap().0,
            browser_id: Uuid::new_v4(),
            attachment_id: Uuid::new_v4(),
            tab_id: Uuid::new_v4(),
            document_epoch: 3,
            viewport_epoch: 2,
            controller_epoch: 4,
            capture_epoch: 5,
        }
    }
    async fn reply(&mut self, id: Uuid, value: Value) {
        let owner = self.handle.owner.lock().unwrap().0;
        self.peer.voyage_reply(id, self.session, owner, value).await;
    }
    async fn attach_status(&mut self) -> HostBrowserBinding {
        let binding = self.binding();
        let task = self.submit(&Op::Status {});
        let (id, op) = self.command().await;
        assert_eq!(op, Op::Status {});
        self.reply(
            id,
            json!({"status":{"running":true,"mode":"human","binding":binding}}),
        )
        .await;
        let reply = tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        let value: Value = reply.json().await.unwrap();
        assert_eq!(value["result"]["status"]["binding"], json!(binding));
        binding
    }
    async fn no_command(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(60), self.peer.command())
                .await
                .is_err(),
            "an unrequested command or replay crossed the synthetic peer"
        );
    }
    async fn detached(&mut self, expected: &HostBrowserBinding) {
        assert_eq!(expected.incarnation, self.handle.owner.lock().unwrap().0);
        self.handle.stop();
        let (id, op) = self.command().await;
        let Op::Detach {
            command_id,
            binding,
        } = op
        else {
            panic!("cleanup may only detach")
        };
        assert!(!command_id.is_nil());
        assert_eq!(&binding, expected);
        self.reply(id, json!({"detached":true})).await;
        self.finish().await;
    }
    async fn finish(&mut self) {
        tokio::time::timeout(Duration::from_secs(7), self.handle.finish())
            .await
            .unwrap()
            .unwrap();
        self.retired().await;
    }
    async fn retired(&self) {
        assert!(self.handle.finished());
        assert!(self.handle.state.borrow().finished);
        assert!(self.handle.state.borrow().launcher.is_none());
        assert!(!self.launcher.exists());
        assert!(!self.launcher.parent().unwrap().exists());
        let address = self.url.socket_addrs(|| None).unwrap()[0];
        assert!(
            tokio::time::timeout(WAIT, tokio::net::TcpStream::connect(address))
                .await
                .unwrap()
                .is_err()
        );
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.handle.stop();
        self.peer.client.disconnect();
    }
}

#[tokio::test]
async fn live_router_assets_host_policy_body_bound_and_private_launcher_cleanup() {
    let mut live = Live::new().await;
    let directory = std::fs::symlink_metadata(live.launcher.parent().unwrap()).unwrap();
    let file = std::fs::symlink_metadata(&live.launcher).unwrap();
    assert!(directory.is_dir() && file.is_file());
    // SAFETY: geteuid has no arguments or pointer preconditions and observes
    // only this test process's current identity; it changes no authority.
    assert_eq!(directory.uid(), unsafe { libc::geteuid() });
    assert_eq!(file.uid(), directory.uid());
    assert_eq!(directory.mode() & 0o777, 0o700);
    assert_eq!(file.mode() & 0o777, 0o600);
    assert_eq!(file.nlink(), 1);
    for (path, content_type) in [
        ("/", "text/html; charset=utf-8"),
        ("/native.mjs", "text/javascript"),
        ("/viewer.mjs", "text/javascript"),
        ("/rrweb-vendor.mjs", "text/javascript"),
        ("/viewer.css", "text/css"),
    ] {
        let reply = live
            .http
            .get(live.url.join(path).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), StatusCode::OK);
        assert_eq!(reply.headers()[header::CONTENT_TYPE], content_type);
        assert_eq!(reply.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(reply.headers()[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(reply.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let csp = reply.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'") && csp.contains("connect-src 'self'"));
        let body = reply.bytes().await.unwrap();
        assert!(!body.is_empty() && body.len() < 4 * 1024 * 1024);
        assert!(
            !body
                .windows(64)
                .any(|b| b == live.url.fragment().unwrap().as_bytes())
        );
    }
    let refused = live
        .http
        .get(live.url.join("/").unwrap())
        .header(header::HOST, "foreign.invalid")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let oversized = live
        .request("/bootstrap")
        .body(vec![b'x'; 3 * 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    live.no_command().await;
    live.finish().await;
    live.no_command().await;
}

#[tokio::test]
async fn live_bootstrap_rejects_foreign_requests_and_concurrent_consumption_is_one_use() {
    let mut live = Live::new().await;
    let token = live.url.fragment().unwrap().to_owned();
    for request in [
        live.http
            .post(live.url.join("/bootstrap").unwrap())
            .header(header::ORIGIN, "null")
            .body(token.clone()),
        live.request("/bootstrap")
            .header(header::HOST, "foreign.invalid")
            .body(token.clone()),
        live.request("/bootstrap").body("wrong-fixture-token"),
    ] {
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let (first, second) = tokio::join!(
        live.request("/bootstrap").body(token.clone()).send(),
        live.request("/bootstrap").body(token.clone()).send(),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let mut statuses = [first.status().as_u16(), second.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 403]);
    let success = if first.status() == StatusCode::OK {
        first
    } else {
        second
    };
    live.auth = Some(success.json().await.unwrap());
    assert_eq!(
        live.request("/alive").send().await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        live.request("/bootstrap")
            .body(token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    live.no_command().await;
    live.finish().await;
}

#[tokio::test]
async fn actual_start_retains_exact_intent_and_prepared_owner_context_only() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let command_id = Uuid::new_v4();
    let task = live.submit(&Op::Start {
        command_id,
        incarnation: live.incarnation,
        expected_revision: 17,
    });
    let (id, op) = live.command().await;
    assert_eq!(
        op,
        Op::Start {
            command_id,
            incarnation: live.incarnation,
            expected_revision: 17
        }
    );
    let prepared = Uuid::new_v4();
    live.peer
        .voyage_reply(
            id,
            live.session,
            prepared,
            json!({"status":"prepared","not_dispatched":true}),
        )
        .await;
    let (snapshot_id, snapshot) = live.peer.command().await;
    assert!(
        matches!(snapshot, VesselCommand::Voyage(VoyageRequest { session_id, incarnation:Some(owner), command:VoyageCommand::Snapshot }) if session_id==live.session && owner==prepared)
    );
    live.peer
        .voyage_reply(snapshot_id, live.session, prepared, json!({"revision":23}))
        .await;
    let (id, command) = live.peer.command().await;
    assert!(
        matches!(command, VesselCommand::Voyage(VoyageRequest { session_id, incarnation:Some(owner), command:VoyageCommand::HostBrowser { operation:Op::Start { command_id:exact, incarnation, expected_revision:23 } } }) if session_id==live.session && owner==prepared && incarnation==prepared && exact==command_id)
    );
    let mut binding = live.binding();
    binding.incarnation = prepared;
    live.peer
        .voyage_reply(
            id,
            live.session,
            prepared,
            json!({"status":{"running":true,"mode":"human","binding":binding}}),
        )
        .await;
    let reply = tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let value: Value = reply.json().await.unwrap();
    assert_eq!(
        value["context"],
        json!({"incarnation":prepared,"revision":23})
    );
    assert!(live.handle.accepts_incarnation(live.incarnation));
    assert!(live.handle.accepts_incarnation(prepared));
    assert!(!live.handle.accepts_incarnation(Uuid::new_v4()));
    assert_eq!(*live.handle.owner.lock().unwrap(), (prepared, 23));
    live.detached(&binding).await;
}

#[tokio::test]
async fn queued_native_controls_have_exact_binding_distinct_ids_and_only_explicit_close() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    let mut ids = std::collections::BTreeSet::new();
    for (control, mode) in [
        (Control::Human, Some(Mode::Human)),
        (Control::Private, Some(Mode::Private)),
        (Control::Agent, Some(Mode::Agent)),
        (Control::Close, None),
    ] {
        live.handle.control(control).unwrap();
        let (id, op) = live.command().await;
        let command_id = match op {
            Op::Control {
                command_id,
                binding: exact,
                mode: actual,
            } => {
                assert_eq!(exact, binding);
                assert_eq!(Some(actual), mode);
                command_id
            }
            Op::Close {
                command_id,
                binding: exact,
            } => {
                assert_eq!(exact, binding);
                assert!(mode.is_none());
                command_id
            }
            _ => panic!("unrequested browser operation"),
        };
        assert!(!command_id.is_nil());
        assert!(ids.insert(command_id));
        live.reply(
            id,
            json!({"status":{"running":mode.is_some(),"mode":"human","binding":binding}}),
        )
        .await;
    }
    live.detached(&binding).await;
    live.no_command().await;
}

#[tokio::test]
async fn unattached_controls_do_not_start_or_close_a_browser_and_queue_is_bounded() {
    let mut live = Live::new().await;
    for _ in 0..8 {
        live.handle.control(Control::Close).unwrap();
    }
    assert!(live.handle.control(Control::Private).is_err());
    tokio::time::timeout(WAIT, async {
        while !live
            .handle
            .state
            .borrow()
            .summary
            .contains("Connect the viewer before")
        {
            live.handle.state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    live.no_command().await;
    live.finish().await;
    assert!(live.handle.control(Control::Human).is_err());
    live.handle.finish().await.unwrap();
    live.no_command().await;
}

#[tokio::test]
async fn socket_retirement_while_start_is_pending_does_not_reconnect_or_replay() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let command_id = Uuid::new_v4();
    let task = live.submit(&Op::Start {
        command_id,
        incarnation: live.incarnation,
        expected_revision: 17,
    });
    let (_, operation) = live.command().await;
    assert_eq!(operation.mutation_id(), Some(command_id));
    live.peer.client.disconnect();
    let reply = tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(reply.status(), StatusCode::CONFLICT);
    live.finish().await;
    assert!(
        live.peer
            .client
            .connection_state()
            .borrow()
            .socket_id
            .is_none()
    );
    assert!(live.handle.control(Control::Close).is_err());
}

#[tokio::test]
async fn mirror_refusal_retains_viewer_and_current_bound_private_control() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    let task = live.submit(&Op::Mirror {
        binding: binding.clone(),
        since: 7,
    });
    let (id, op) = live.command().await;
    assert_eq!(
        op,
        Op::Mirror {
            binding: binding.clone(),
            since: 7
        }
    );
    live.peer
        .voyage_reply(
            id,
            live.session,
            Uuid::new_v4(),
            json!({"status":{"running":true,"mode":"agent"}}),
        )
        .await;
    assert_eq!(task.await.unwrap().status(), StatusCode::CONFLICT);
    assert_eq!(
        live.request("/alive").send().await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert!(!live.handle.stop.is_cancelled());
    live.handle.control(Control::Private).unwrap();
    let (id, op) = live.command().await;
    assert!(matches!(op, Op::Control { binding:exact, mode:Mode::Private, .. } if exact==binding));
    live.reply(
        id,
        json!({"status":{"running":true,"mode":"private","binding":binding}}),
    )
    .await;
    live.detached(&binding).await;
}

#[tokio::test]
async fn unknown_effect_reply_poisoning_fences_live_http_without_replaying_the_effect() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    let command_id = Uuid::new_v4();
    let task = live.submit(&Op::Input {
        command_id,
        binding: binding.clone(),
        sequence: 1,
        claim: false,
        input: HostBrowserInput::Text {
            text: "synthetic-private-not-journalled".into(),
        },
    });
    let (id, operation) = live.command().await;
    assert_eq!(operation.mutation_id(), Some(command_id));
    send(
        &mut live.peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("synthetic uncertain browser input".into()),
                outcome_unknown: true,
                ..response(Value::Null)
            },
        },
    )
    .await;
    let reply = task.await.unwrap();
    assert_eq!(reply.status(), StatusCode::CONFLICT);
    assert_eq!(
        reply.text().await.unwrap(),
        "Browser operation refused or outcome unknown; not replayed"
    );
    let late = live
        .request("/operation")
        .json(&Op::Receipt { command_id })
        .send()
        .await;
    if let Ok(reply) = late {
        assert_eq!(reply.status(), StatusCode::CONFLICT);
    }
    live.detached(&binding).await;
    live.no_command().await;
}

#[tokio::test]
async fn pending_ordinary_input_gate_refuses_overlap_but_explicit_private_control_interrupts() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    let command_id = Uuid::new_v4();
    let held = live.submit(&Op::Input {
        command_id,
        binding: binding.clone(),
        sequence: 1,
        claim: false,
        input: HostBrowserInput::Navigate {
            url: "https://synthetic.invalid/owned".into(),
        },
    });
    let (held_id, op) = live.command().await;
    assert_eq!(op.mutation_id(), Some(command_id));
    let refused = live
        .request("/operation")
        .json(&Op::Input {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
            sequence: 2,
            claim: false,
            input: HostBrowserInput::Fill {
                node_id: 1,
                text: "synthetic".into(),
            },
        })
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    live.no_command().await;
    let control_id = Uuid::new_v4();
    let private = live.submit(&Op::Control {
        command_id: control_id,
        binding: binding.clone(),
        mode: Mode::Private,
    });
    let (id, op) = live.command().await;
    assert_eq!(
        op,
        Op::Control {
            command_id: control_id,
            binding: binding.clone(),
            mode: Mode::Private
        }
    );
    let mut updated = binding;
    updated.controller_epoch += 1;
    let status = json!({"status":{"running":true,"mode":"private","binding":updated}});
    live.reply(id, status.clone()).await;
    assert_eq!(private.await.unwrap().status(), StatusCode::OK);
    live.reply(held_id, status).await;
    assert_eq!(held.await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        live.handle.state.borrow().summary,
        "Executing-host browser: running; control: private"
    );
    live.detached(&updated).await;
}

#[tokio::test]
async fn claim_dialog_and_navigation_stop_interrupt_the_gate_without_unrequested_replay() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    for (claim, input) in [
        (
            true,
            HostBrowserInput::Click {
                node_id: 1,
                button: HostBrowserButton::Left,
            },
        ),
        (
            false,
            HostBrowserInput::Dialog {
                accept: false,
                text: None,
            },
        ),
        (
            false,
            HostBrowserInput::History {
                direction: HostBrowserHistory::Stop,
            },
        ),
    ] {
        let held = live.submit(&Op::Input {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
            sequence: 1,
            claim: false,
            input: HostBrowserInput::Navigate {
                url: "https://synthetic.invalid/held".into(),
            },
        });
        let (held_id, _) = live.command().await;
        let interrupt_op = Op::Input {
            command_id: Uuid::new_v4(),
            binding: binding.clone(),
            sequence: 2,
            claim,
            input,
        };
        let interrupt = live.submit(&interrupt_op);
        let (id, op) = live.command().await;
        assert_eq!(op, interrupt_op);
        let value = json!({"status":{"running":true,"mode":"human","binding":binding}});
        live.reply(id, value.clone()).await;
        assert_eq!(interrupt.await.unwrap().status(), StatusCode::OK);
        live.reply(held_id, value).await;
        assert_eq!(held.await.unwrap().status(), StatusCode::OK);
    }
    live.detached(&binding).await;
    live.no_command().await;
}

#[tokio::test]
async fn invalid_live_intents_and_foreign_owner_never_reach_the_bound_socket() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let auth = live.auth.as_ref().unwrap();
    let mut exact = HeaderMap::new();
    exact.insert(header::ORIGIN, live.origin().parse().unwrap());
    exact.insert(
        header::AUTHORIZATION,
        auth["authorization"].as_str().unwrap().parse().unwrap(),
    );
    exact.insert(
        "x-helm-csrf",
        auth["csrf"].as_str().unwrap().parse().unwrap(),
    );
    for key in ["origin", "authorization", "x-helm-csrf"] {
        for missing in [true, false] {
            let mut headers = exact.clone();
            if missing {
                headers.remove(key);
            } else {
                headers.insert(
                    axum::http::HeaderName::from_static(key),
                    "foreign".parse().unwrap(),
                );
            }
            let reply = live
                .http
                .post(live.url.join("/operation").unwrap())
                .headers(headers.clone())
                .json(&Op::Status {})
                .send()
                .await
                .unwrap();
            assert_eq!(reply.status(), StatusCode::FORBIDDEN);
            assert_eq!(
                live.http
                    .post(live.url.join("/alive").unwrap())
                    .headers(headers)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
            assert!(!live.handle.stop.is_cancelled());
        }
    }
    let binding = live.binding();
    let mut foreign = binding.clone();
    foreign.incarnation = Uuid::new_v4();
    for op in [
        Op::Mirror {
            binding: foreign,
            since: 0,
        },
        Op::Start {
            command_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            expected_revision: 17,
        },
        Op::Close {
            command_id: Uuid::nil(),
            binding: binding.clone(),
        },
        Op::Input {
            command_id: Uuid::new_v4(),
            binding,
            sequence: 1,
            claim: true,
            input: HostBrowserInput::Text {
                text: "synthetic".into(),
            },
        },
    ] {
        let reply = live.request("/operation").json(&op).send().await.unwrap();
        assert_eq!(reply.status(), StatusCode::CONFLICT);
        assert!(!live.handle.stop.is_cancelled());
    }
    let extra = live
        .request("/operation")
        .body(r#"{"action":"status","socket_id":"forged"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(extra.status(), StatusCode::BAD_REQUEST);
    live.no_command().await;
    live.finish().await;
}

#[tokio::test]
async fn dropped_http_observer_does_not_replay_a_late_effect_and_detach_removes_server() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    let op = Op::Input {
        command_id: Uuid::new_v4(),
        binding: binding.clone(),
        sequence: 1,
        claim: false,
        input: HostBrowserInput::Click {
            node_id: 1,
            button: HostBrowserButton::Left,
        },
    };
    let request = live
        .request("/operation")
        .json(&op)
        .timeout(Duration::from_secs(1));
    let observer = tokio::spawn(async move { request.send().await });
    let (effect_id, exact) = live.command().await;
    assert_eq!(exact, op);
    assert!(observer.await.unwrap().is_err());
    let socket = live.peer.client.connection_state().borrow().socket_id;
    live.detached(&binding).await;
    // The adapter server is already gone. This original late response cannot
    // redirect to a new viewer or create a second effect on the retained socket.
    live.reply(
        effect_id,
        json!({"status":{"running":true,"mode":"human","binding":binding}}),
    )
    .await;
    assert_eq!(
        live.peer.client.connection_state().borrow().socket_id,
        socket
    );
    live.no_command().await;
}

#[tokio::test]
async fn failed_native_control_reports_unknown_but_still_observes_local_cleanup_without_replay() {
    let mut live = Live::new().await;
    live.bootstrap().await;
    let binding = live.attach_status().await;
    live.handle.control(Control::Private).unwrap();
    let (id, op) = live.command().await;
    assert!(matches!(op, Op::Control { binding:exact,mode:Mode::Private,.. } if exact==binding));
    send(
        &mut live.peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("synthetic native control uncertain".into()),
                outcome_unknown: true,
                ..response(Value::Null)
            },
        },
    )
    .await;
    let (id, detach) = live.command().await;
    assert!(matches!(detach, Op::Detach { binding:exact,.. } if exact==binding));
    live.reply(id, json!({"detached":true})).await;
    assert!(
        tokio::time::timeout(WAIT, live.handle.finish())
            .await
            .unwrap()
            .is_err()
    );
    live.retired().await;
    assert!(
        live.handle
            .state
            .borrow()
            .summary
            .contains("unknown effects are not replayed")
    );
    assert!(live.handle.control(Control::Private).is_err());
    live.no_command().await;
}

#[tokio::test]
async fn dropping_an_unattached_handle_cancels_and_removes_its_only_owned_launcher() {
    let live = Live::new().await;
    let launcher = live.launcher.clone();
    let root = launcher.parent().unwrap().to_owned();
    let stopped = live.handle.stop.clone();
    drop(live);
    assert!(stopped.is_cancelled());
    tokio::time::timeout(WAIT, async {
        while launcher.exists() || root.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn missing_socket_handle_ends_without_launcher_or_implicit_host_start() {
    let root = tempfile::tempdir().unwrap();
    let client = Client::local(root.path().join("absent-vessel"));
    let mut handle = Handle::start(client.clone(), Uuid::new_v4(), Uuid::new_v4(), 17);
    tokio::time::timeout(WAIT, async {
        while !handle.state.borrow().finished {
            handle.state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(handle.state.borrow().launcher.is_none());
    assert!(
        handle
            .state
            .borrow()
            .summary
            .contains("unknown effects are not replayed")
    );
    assert!(handle.finished());
    assert!(handle.control(Control::Close).is_err());
    assert!(handle.finish().await.is_err());
    assert!(handle.finish().await.is_ok());
    assert!(client.connection_state().borrow().socket_id.is_none());
    assert!(!root.path().join("absent-vessel").exists());
}
