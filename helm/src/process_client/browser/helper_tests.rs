use super::*;
use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream};

// No child, browser, terminal, environment changes or external service.
pub(crate) fn synthetic() -> (Arc<Helper>, DuplexStream) {
    let (input, peer) = tokio::io::duplex(FRAME_LIMIT + 1);
    (
        Arc::new(Helper {
            input: tokio::sync::Mutex::new(Box::new(input)),
            child: tokio::sync::Mutex::new(None),
            pending: Default::default(),
            events: broadcast::channel(32).0,
            stopped: CancellationToken::new(),
            reader: tokio::sync::Mutex::new(None),
            pid: 0,
        }),
        peer,
    )
}

pub(crate) async fn respond(
    helper: &Helper,
    peer: &mut BufReader<DuplexStream>,
    value: Value,
) -> Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), peer.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    let id = request["id"].as_str().unwrap();
    let sender = helper.pending.lock().unwrap().remove(id).unwrap();
    sender.send(Ok(value)).unwrap();
    request
}

#[tokio::test]
async fn ipc_exact_identity_envelope_and_refusal() {
    let (helper, peer) = synthetic();
    let mut peer = BufReader::new(peer);
    for envelope in [
        json!({"ok":true,"result":{"epoch":7}}),
        json!({"ok":false}),
        json!({"ok":true}),
    ] {
        let h = helper.clone();
        let task =
            tokio::spawn(
                async move { h.call(json!({"op":"status"}), Duration::from_secs(2)).await },
            );
        let request = respond(&helper, &mut peer, envelope.clone()).await;
        assert_eq!(request["op"], "status");
        let result = task.await.unwrap();
        if envelope["ok"] == true {
            assert_eq!(result.unwrap(), envelope["result"]);
        } else {
            assert!(result.unwrap_err().to_string().contains("refused"));
        }
        assert!(helper.pending.lock().unwrap().is_empty());
    }
    let h = helper.clone();
    let task = tokio::spawn(async move {
        h.call_exact(
            json!({"op":"action","id":"wrong"}),
            "durable-id".into(),
            Duration::from_secs(2),
        )
        .await
    });
    assert_eq!(
        respond(&helper, &mut peer, json!({"ok":false})).await["id"],
        "durable-id"
    );
    assert_eq!(task.await.unwrap().unwrap(), json!({"ok":false}));
}

#[tokio::test]
async fn ipc_admission_limits_timeouts_and_closed_writer() {
    let (helper, peer) = synthetic();
    for request in [json!(null), json!({"payload":"x".repeat(FRAME_LIMIT)})] {
        assert!(
            helper
                .call_exact(request, "id".into(), Duration::from_millis(10))
                .await
                .is_err()
        );
        assert!(helper.pending.lock().unwrap().is_empty());
    }
    let (sender, _receiver) = oneshot::channel();
    helper
        .pending
        .lock()
        .unwrap()
        .insert("duplicate".into(), sender);
    assert!(
        helper
            .call_exact(json!({}), "duplicate".into(), Duration::from_millis(10))
            .await
            .unwrap_err()
            .to_string()
            .contains("already in flight")
    );
    for n in 0..15 {
        helper
            .pending
            .lock()
            .unwrap()
            .insert(n.to_string(), oneshot::channel().0);
    }
    assert!(
        helper
            .call(json!({}), Duration::from_millis(10))
            .await
            .unwrap_err()
            .to_string()
            .contains("queue full")
    );
    helper.pending.lock().unwrap().clear();
    assert!(
        helper
            .call(json!({}), Duration::from_millis(10))
            .await
            .unwrap_err()
            .to_string()
            .contains("must not be replayed")
    );
    assert!(helper.pending.lock().unwrap().is_empty());
    drop(peer);
    assert!(
        helper
            .call(json!({}), Duration::from_secs(1))
            .await
            .is_err()
    );
    assert!(helper.pending.lock().unwrap().is_empty());
    helper.stopped().cancel();
    assert!(
        helper
            .call(json!({}), Duration::from_secs(1))
            .await
            .unwrap_err()
            .to_string()
            .contains("unavailable")
    );
    let mut events = helper.events();
    helper.events.send(json!({"event":"private"})).unwrap();
    assert_eq!(events.recv().await.unwrap()["event"], "private");
}
