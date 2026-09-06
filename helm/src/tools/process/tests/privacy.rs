use super::*;

async fn screen_contains(tool: &ProcessTool, id: TerminalId, expected: &str) -> TerminalSnapshot {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = tool.snapshot(id).await.unwrap();
            let text = snapshot
                .cells
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.contains(expected) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("native private output was not displayed to human")
}

#[tokio::test]
async fn native_echo_and_application_repeats_of_direct_human_input_never_enter_model_capture() {
    for echo in ["echo", "-echo"] {
        let directory = tempfile::tempdir().unwrap();
        let ctx = context(directory.path());
        let tool = ProcessTool::default();
        let started = tool.execute(json!({"action":"start","name":"privacy-fixture","command":format!("stty {echo}; printf ready; cat")}), &ctx).await.unwrap();
        let id = TerminalId(Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap());
        screen_contains(&tool, id, "ready").await;
        tool.write(id, b"human-only-canary-\xe7\x95\x8c\n".to_vec())
            .await
            .unwrap();
        screen_contains(&tool, id, "human-only-canary-界").await;
        let model = tool
            .execute(json!({"action":"read","id":id.0}), &ctx)
            .await
            .unwrap();
        let cleanup = tool.shutdown(Duration::from_secs(5)).await;
        #[cfg(target_os = "linux")]
        assert!(cleanup.observation_complete);
        assert!(
            !model.contains("human-only-canary"),
            "human input reached model capture with stty {echo}: {model}"
        );
    }
}

#[tokio::test]
async fn explicit_attach_discards_unread_and_withholds_late_output_but_preserves_other_model_terminals()
 {
    let directory = tempfile::tempdir().unwrap();
    let ctx = context(directory.path());
    let tool = ProcessTool::default();
    let mut ids = Vec::new();
    for name in ["human-terminal", "model-terminal"] {
        let started = tool
            .execute(
                json!({"action":"start","name":name,"command":"stty -echo; printf ready; cat"}),
                &ctx,
            )
            .await
            .unwrap();
        ids.push(TerminalId(
            Uuid::parse_str(started.split_whitespace().last().unwrap()).unwrap(),
        ));
    }
    let human = ids[0];
    screen_contains(&tool, human, "ready").await;
    let disclosed = tool.read(human.0, 4096).unwrap();
    assert!(disclosed.contains("ready"));
    tool.execute(
        json!({"action":"write","id":human.0,"data":"unread-before-attach\n"}),
        &ctx,
    )
    .await
    .unwrap();
    let before = screen_contains(&tool, human, "unread-before-attach").await;
    let attached = tool.attach(human).await.unwrap();
    let privacy = attached.privacy.as_ref().unwrap();
    assert!(privacy.discarded_unread_bytes > 0);
    assert_eq!(attached.dropped_unread_bytes, before.dropped_unread_bytes);
    assert!(attached.revision >= before.revision);
    assert!(
        !tool
            .read(human.0, 4096)
            .unwrap()
            .contains("unread-before-attach")
    );
    // There is intentionally no detach operation that can resume capture.
    tool.write(human, b"private-first\n".to_vec())
        .await
        .unwrap();
    let first = screen_contains(&tool, human, "private-first").await;
    tool.write(
        human,
        b"late-repeat-private-first\x1b]0;private-title-canary\x07\n".to_vec(),
    )
    .await
    .unwrap();
    let late = screen_contains(&tool, human, "late-repeat-private-first").await;
    let reattached = tool.attach(human).await.unwrap();
    assert_eq!(reattached.privacy, late.privacy);
    assert!(late.revision > first.revision);
    assert!(
        late.privacy.unwrap().suppressed_output_bytes
            > first.privacy.unwrap().suppressed_output_bytes
    );
    let model_read = tool.read(human.0, 4096).unwrap();
    assert!(model_read.contains("model capture unavailable"));
    assert!(!model_read.contains("private-first"));
    let model_write = tool
        .execute(
            json!({"action":"write","id":human.0,"data":"must-not-enter-terminal\n"}),
            &ctx,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(model_write.contains("private"));
    assert!(!model_write.contains("must-not-enter-terminal"));
    let metadata = tool.execute(json!({"action":"list"}), &ctx).await.unwrap();
    assert!(!metadata.contains("private-first") && !metadata.contains("private-title-canary"));
    assert_eq!(reattached.title, "human-terminal");
    tool.execute(
        json!({"action":"write","id":ids[1].0,"data":"model-still-visible\n"}),
        &ctx,
    )
    .await
    .unwrap();
    screen_contains(&tool, ids[1], "model-still-visible").await;
    assert!(
        tool.read(ids[1].0, 4096)
            .unwrap()
            .contains("model-still-visible")
    );
    assert!(tool.snapshot(ids[1]).await.unwrap().privacy.is_none());
    // Existing explicit interruption/termination authority must remain usable.
    tool.execute(json!({"action":"interrupt","id":human.0}), &ctx)
        .await
        .unwrap();
    tool.execute(json!({"action":"terminate","id":human.0}), &ctx)
        .await
        .unwrap();
    let cleanup = tool.shutdown(Duration::from_secs(5)).await;
    #[cfg(target_os = "linux")]
    assert!(cleanup.observation_complete);
    assert!(
        disclosed.contains("ready"),
        "privacy must not rewrite already disclosed text"
    );
}

#[test]
fn capture_privacy_omissions_are_distinct_from_bounded_eviction() {
    let mut capture = Capture::new(2, 10);
    capture.capture_output(b"abcdefgh", 5);
    assert_eq!(capture.dropped, 3);
    assert_eq!(capture.bytes, b"defgh");
    capture.make_private(5); // de were already read; only fgh are unread.
    assert_eq!(capture.privacy.as_ref().unwrap().discarded_unread_bytes, 3);
    assert!(capture.bytes.is_empty());
    capture.capture_output(b"secret-late-output", 5);
    capture.make_private(0); // Reattach cannot reset either privacy counter.
    assert_eq!(capture.dropped, 3);
    assert_eq!(
        capture.privacy.as_ref().unwrap().suppressed_output_bytes,
        18
    );
    assert!(capture.bytes.is_empty());
    assert_eq!(capture.base, 26);
}
