use super::*;
struct Offline;
#[async_trait]
impl super::super::SubagentExecutor for Offline {
    async fn execute(
        &self,
        context: super::super::ExecutionContext,
    ) -> Result<super::super::SubagentResult, String> {
        if context.task == "fail" {
            Err("offline failure".into())
        } else {
            Ok(super::super::SubagentResult {
                summary: context.task,
            })
        }
    }
}
#[tokio::test]
async fn model_surface_dispatches_offline_supervision_and_retained_outcomes() {
    let root = tempfile::tempdir().unwrap();
    let ctx = crate::tools::reliability_tests::context(root.path());
    let runtime = Arc::new(
        SubagentRuntime::new(
            Arc::new(Offline),
            super::super::RuntimeLimits::default(),
            None,
        )
        .unwrap(),
    );
    let budget = AgentBudget {
        max_tokens: 100,
        max_terminals: 1,
    };
    let policy = AgentPolicy {
        access: crate::config::AccessMode::ReadOnly,
        readable_roots: vec![root.path().into()],
        writable_roots: vec![],
        allowed_tools: Default::default(),
        approval: super::super::ApprovalPolicy::Deny,
        budget: budget.clone(),
    };
    let tool = SubagentTool::new(runtime.clone(), policy, budget).with_worktrees(None);
    crate::tools::schema::CompiledSchema::compile(&tool.definition().input_schema).unwrap();
    let mut ids = Vec::new();
    for task in ["done", "fail"] {
        let output = tool
            .execute(json!({"action":"spawn","name":"offline","task":task}), &ctx)
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        let id: Uuid = serde_json::from_value(output["id"].clone()).unwrap();
        ids.push(id);
        let output = tool
            .execute(json!({"action":"wait","id":id}), &ctx)
            .await
            .unwrap();
        let output: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            output["status"],
            if task == "fail" {
                "failed"
            } else {
                "completed"
            }
        );
        let status: Value = serde_json::from_str(
            &tool
                .execute(json!({"action":"status","id":id}), &ctx)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(status["id"], id.to_string());
    }
    let outcomes: Value = serde_json::from_str(
        &tool
            .execute(json!({"action":"wait_many","ids":ids}), &ctx)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(outcomes["results"].as_array().unwrap().len(), 2);
    assert_eq!(outcomes["results"][0]["status"], "completed");
    assert_eq!(outcomes["results"][1]["status"], "failed");
    for arguments in [
        json!({"action":"list"}),
        json!({"action":"list","parent_id":ids[0]}),
        json!({"action":"archive","limit":1}),
    ] {
        let result = tool.execute(arguments, &ctx).await.unwrap();
        assert!(serde_json::from_str::<Value>(&result).is_ok());
    }
    for arguments in [
        json!({"action":"wait_many","ids":[]}),
        json!({"action":"spawn","name":"bad","task":"task","worktree":true}),
        json!({"action":"worktree_status","id":ids[0]}),
        json!({"action":"cleanup","id":ids[0]}),
        json!({"action":"commit","id":ids[0],"message":"message"}),
        json!({"action":"integrate","id":ids[0],"target":"main"}),
        json!({"action":"worktree_conflicts","id":ids[0],"other_id":ids[1]}),
        json!({"action":"message","id":Uuid::nil(),"message":"hello"}),
        json!({"action":"follow_up","id":ids[0],"message":"hello"}),
        json!({"action":"status","id":Uuid::nil()}),
        json!({"action":"cancel","id":Uuid::nil()}),
    ] {
        assert!(
            tool.execute(arguments.clone(), &ctx).await.is_err(),
            "{arguments}"
        );
    }
    let tool = tool.with_worktree_error(Some("offline discovery refused".into()));
    let error = tool
        .execute(
            json!({"action":"spawn","name":"n","task":"t","worktree":true}),
            &ctx,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("offline discovery refused"));
    runtime.shutdown().await;
}
#[test]
fn tagged_actions_reject_cross_action_arguments_and_roundtrip_valid_requests() {
    let id = Uuid::nil();
    for value in [
        json!({"action":"list"}),
        json!({"action":"status","id":id}),
        json!({"action":"archive"}),
        json!({"action":"spawn","name":"n","task":"t"}),
        json!({"action":"message","id":id,"message":"m"}),
    ] {
        let action: Args = serde_json::from_value(value).unwrap();
        let encoded = serde_json::to_value(action).unwrap();
        assert!(serde_json::from_value::<Args>(encoded.clone()).is_ok());
        let mut extra = encoded;
        extra["unexpected"] = json!(true);
        assert!(serde_json::from_value::<Args>(extra).is_err());
    }
}
