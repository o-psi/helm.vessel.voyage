use super::*;
use std::time::Duration;
#[tokio::test]
async fn process_lifecycle_capture_and_human_takeover_refuse_model_input() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let tool = ProcessTool::with_limits(2, 65536);
    let response = tool
        .execute(
            json!({"action":"start","command":"/bin/cat","name":"fixture"}),
            &context,
        )
        .await
        .unwrap();
    assert!(response.contains("started"));
    let id = tool.metadata().unwrap()[0].id;
    assert!(tool.has_owned_work());
    assert_eq!(tool.metadata().unwrap().len(), 1);
    tool.execute(
        json!({"action":"write","id":id,"data":"fixture-visible\n"}),
        &context,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let read = tool
        .execute(json!({"action":"read","id":id}), &context)
        .await
        .unwrap();
    assert!(read.contains("fixture-visible"));
    tool.execute(
        json!({"action":"rename","id":id,"name":"renamed"}),
        &context,
    )
    .await
    .unwrap();
    assert_eq!(tool.metadata().unwrap()[0].name.as_deref(), Some("renamed"));
    tool.execute(
        json!({"action":"resize","id":id,"cols":90,"rows":30}),
        &context,
    )
    .await
    .unwrap();
    tool.execute(json!({"action":"select","id":id}), &context)
        .await
        .unwrap();
    tool.attach(TerminalId(id)).await.unwrap();
    tool.write(TerminalId(id), b"private-human-input\n".to_vec())
        .await
        .unwrap();
    assert!(
        tool.execute(json!({"action":"write","id":id,"data":"model"}), &context)
            .await
            .is_err()
    );
    let read = tool
        .execute(json!({"action":"read","id":id}), &context)
        .await
        .unwrap();
    assert!(!read.contains("private-human-input"));
    assert!(!InteractiveTerminals::list(&tool).await.unwrap().is_empty());
    let report = tool.shutdown(Duration::from_secs(3)).await;
    assert!(report.observation_complete, "{report:?}");
    assert!(tool.can_retire());
    assert!(
        tool.execute(json!({"action":"start","command":"true"}), &context)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn invalid_identity_limits_and_names_do_not_spawn_work() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let tool = ProcessTool::with_limits(1, 4096);
    for value in [
        json!({"action":"read","id":Uuid::new_v4()}),
        json!({"action":"resize","id":Uuid::new_v4(),"cols":0,"rows":2}),
        json!({"action":"start","command":"true","cols":0}),
        json!({"action":"start","command":"true","rows":0}),
    ] {
        assert!(tool.execute(value, &context).await.is_err());
    }
    assert!(!tool.has_owned_work());
    assert!(
        tool.shutdown(Duration::from_secs(1))
            .await
            .observation_complete
    );
}
