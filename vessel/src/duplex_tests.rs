use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct FixtureBackend {
    allowed: bool,
    disconnects: AtomicUsize,
}
impl Backend for FixtureBackend {
    fn command(&self, _: VesselRequest) -> BackendFuture<VesselResponse> {
        Box::pin(async { unknown() })
    }
    fn authorize(&self, _: Option<Uuid>) -> BackendFuture<bool> {
        let allowed = self.allowed;
        Box::pin(async move { allowed })
    }
    fn disconnected(&self, _: Uuid) {
        self.disconnects.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn subprotocol_matching_handles_multiple_headers_without_substrings() {
    let mut headers = axum::http::HeaderMap::new();
    assert!(!supports(&headers));
    headers.append(
        "sec-websocket-protocol",
        format!("{SUBPROTOCOL}-other").parse().unwrap(),
    );
    assert!(!supports(&headers));
    headers.append(
        "sec-websocket-protocol",
        axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
    );
    assert!(!supports(&headers));
    headers.append(
        "sec-websocket-protocol",
        format!("unrelated,  {SUBPROTOCOL}  ,other")
            .parse()
            .unwrap(),
    );
    assert!(supports(&headers));
}
#[tokio::test]
async fn publication_preserves_scope_and_releases_memory_on_receive_or_close() {
    let (tx, mut rx) = mpsc::channel(2);
    let session = Uuid::new_v4();
    let id = Uuid::new_v4();
    assert!(
        enqueue(
            &tx,
            ServerFrame::Reply {
                request_id: id,
                response: VesselResponse {
                    protocol: VESSEL_API_VERSION,
                    result: serde_json::Value::Null,
                    error: Some("revoked".into()),
                    outcome_unknown: false
                }
            },
            Some(session)
        )
        .await
    );
    let output = rx.recv().await.unwrap();
    assert_eq!(output.session, Some(session));
    let decoded: ServerFrame = serde_json::from_str(&output.text).unwrap();
    assert!(
        matches!(decoded, ServerFrame::Reply { request_id: actual, response } if actual == id && response.error.as_deref() == Some("revoked"))
    );
    assert_eq!(output._bytes.num_permits(), output.text.len());
    drop(output);
    drop(rx);
    assert!(!enqueue(&tx, ServerFrame::Subscribed { request_id: id }, None).await);
}
#[tokio::test]
async fn oversized_frames_are_refused_before_entering_the_queue() {
    let (tx, mut rx) = mpsc::channel(1);
    assert!(
        !enqueue(
            &tx,
            ServerFrame::Reply {
                request_id: Uuid::new_v4(),
                response: VesselResponse {
                    protocol: VESSEL_API_VERSION,
                    result: serde_json::Value::Null,
                    error: Some("x".repeat(MAX_FRAME_BYTES)),
                    outcome_unknown: false
                }
            },
            None
        )
        .await
    );
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}
#[tokio::test]
async fn reverse_browser_notification_is_typed_and_fails_closed_on_disconnect() {
    let (tx, mut rx) = mpsc::channel::<Reverse>(1);
    let c = Connection {
        socket_id: Uuid::new_v4(),
        tx,
    };
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let receiver = async {
        let reverse = rx.recv().await.unwrap();
        assert!(
            matches!(reverse.request, ReverseRequest::BrowserWork { session_id, incarnation:actual } if session_id == session && actual == incarnation)
        );
        reverse.result.send(Some(ReverseReply::Accepted)).unwrap();
    };
    let (reply, ()) = tokio::join!(c.browser_work(session, incarnation), receiver);
    assert!(matches!(reply, Some(ReverseReply::Accepted)));
    let receiver = async {
        drop(rx.recv().await.unwrap());
    };
    let (reply, ()) = tokio::join!(c.browser_work(session, incarnation), receiver);
    assert!(reply.is_none());
    drop(rx);
    assert!(c.browser_work(session, incarnation).await.is_none());
}
#[tokio::test]
async fn registration_drop_removes_only_its_socket_and_notifies_backend_once() {
    let backend = Arc::new(FixtureBackend {
        allowed: true,
        disconnects: AtomicUsize::new(0),
    });
    let (tx, _rx) = mpsc::channel(1);
    let id = Uuid::new_v4();
    let c = Connection { socket_id: id, tx };
    connections().lock().unwrap().insert(id, c);
    let registration = Registration(id, backend.clone());
    assert_eq!(connection(id).unwrap().socket_id, id);
    let dynamic: Arc<dyn Backend> = backend.clone();
    assert!(authorized(&dynamic, Some(Uuid::new_v4())).await);
    let denied: Arc<dyn Backend> = Arc::new(FixtureBackend {
        allowed: false,
        disconnects: AtomicUsize::new(0),
    });
    assert!(!authorized(&denied, None).await);
    drop(registration);
    assert!(connection(id).is_none());
    assert_eq!(backend.disconnects.load(Ordering::SeqCst), 1);
    let response = unknown();
    assert!(response.outcome_unknown);
    assert!(response.result.is_null());
    assert!(response.error.unwrap().contains("do not replay"));
}
