//! Exercise the provider-facing adapter through real isolated durable stores.
use super::*;
use crate::{
    completion::{
        Readiness, RunId,
        runtime::{Coordinator, RunHandle},
        store::{RunLedgerStore, RunScope},
    },
    config::AccessMode,
    subagent::{AgentBudget, AgentPolicy, AgentRecord, AgentStatus, ApprovalPolicy},
    todo::{NewTodo, Priority, TodoScope, TodoStatus},
    tools::{Redactor, reliability_tests::context},
};
use std::{collections::BTreeSet, time::Duration};

struct Fixture {
    root: tempfile::TempDir,
    tool: CompletionTool,
    context: ToolContext,
    session: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(root.path().join("completion"), root.path()).unwrap();
        let session = Uuid::new_v4();
        let run = RunHandle::create(coordinator.clone(), session, Uuid::new_v4())
            .await
            .unwrap();
        let todos = Arc::new(
            TodoStore::new(
                root.path().join("todos.json"),
                TodoScope::session(root.path().into(), session),
            )
            .with_coordinator(coordinator.clone()),
        );
        let mut agents =
            AgentTreeStore::new(root.path().join("agents.json")).with_coordinator(coordinator);
        agents.acquire_runtime_owner().unwrap();
        let mut context = context(root.path());
        context.completion = Some(run);
        Self {
            root,
            tool: CompletionTool::new(todos, agents),
            context,
            session,
        }
    }
    async fn call(&self, args: Value) -> Value {
        serde_json::from_str(&self.tool.execute(args, &self.context).await.unwrap()).unwrap()
    }
    async fn snapshot(&self) -> Readiness {
        serde_json::from_value(self.call(json!({"action":"snapshot"})).await).unwrap()
    }
    async fn todo(&self) -> TodoId {
        self.tool
            .todos
            .create(NewTodo {
                title: "Unfinished owned fixture work".into(),
                description: "A real store record, no provider".into(),
                priority: Priority::Normal,
                order: None,
                assignees: BTreeSet::new(),
            })
            .await
            .unwrap()
            .id
    }
    fn account(
        &self,
        kind: &str,
        id: Uuid,
        snapshot: &Readiness,
        disposition: &str,
        reason: &str,
    ) -> Value {
        json!({"action":"account", "kind":kind, "id":id, "revision":snapshot.revision,
            "fingerprint":snapshot.fingerprint, "disposition":disposition, "reason":reason})
    }
}

#[tokio::test]
async fn todo_adoption_is_explicit_and_stale_reviews_never_complete_work() {
    let mut f = Fixture::new().await;
    let id = f.todo().await;
    let empty = f.snapshot().await;
    assert_eq!(empty.total, 0);
    assert!(matches!(
        f.tool
            .execute(json!({"action":"read","kind":"todo","id":id.0}), &f.context)
            .await,
        Err(ToolError::Failed(_))
    ));
    assert_eq!(
        f.call(json!({"action":"adopt","kind":"todo","id":id.0,"revision":empty.revision}))
            .await,
        json!({"adopted":id.0})
    );
    let first = f.snapshot().await;
    assert_eq!(first.total, 1);
    assert_eq!(first.accounted, 0);
    let record = f
        .call(json!({"action":"read","kind":"todo","id":id.0}))
        .await;
    assert_eq!(record["status"], "pending");
    assert!(
        f.tool
            .execute(
                f.account(
                    "todo",
                    id.0,
                    &first,
                    "completed_with_evidence",
                    "No completed record or evidence exists"
                ),
                &f.context
            )
            .await
            .is_err()
    );
    f.tool
        .todos
        .append_note(
            id,
            crate::todo::EntryKind::Progress,
            "Record changed after review".into(),
            None,
        )
        .await
        .unwrap();
    assert!(
        f.tool
            .execute(
                f.account(
                    "todo",
                    id.0,
                    &first,
                    "deferred_with_impact",
                    "Impact remains"
                ),
                &f.context
            )
            .await
            .is_err()
    );
    let current = f.snapshot().await;
    f.context.redactor = Arc::new(Redactor::new(["synthetic-review-secret".to_owned()]));
    assert_eq!(
        f.call(f.account(
            "todo",
            id.0,
            &current,
            "deferred_with_impact",
            "Awaiting synthetic-review-secret fixture evidence; work remains pending"
        ))
        .await,
        json!({"accounted":id.0})
    );
    let accounted = f.snapshot().await;
    assert_eq!((accounted.accounted, accounted.incomplete), (1, 1));
    assert_eq!(
        f.tool.todos.snapshot().await.unwrap().items[&id].status,
        TodoStatus::Pending
    );
    assert!(!f.tool.todos.snapshot().await.unwrap().items[&id].archived());
    let run = f.context.completion.as_ref().unwrap();
    let store = RunLedgerStore::open(
        f.root.path().join("completion/ledgers"),
        RunScope::new(f.root.path(), f.session).unwrap(),
    )
    .unwrap();
    let text =
        String::from_utf8(store.load(RunId(run.run_id())).unwrap().to_json().unwrap()).unwrap();
    assert!(!text.contains("synthetic-review-secret"));
    assert!(text.contains("[REDACTED]"));
    assert_eq!(f.snapshot().await.total, 1);
}

#[tokio::test]
async fn agent_adoption_requires_terminal_work_and_read_preserves_the_actual_result() {
    let f = Fixture::new().await;
    let id = AgentId::new();
    let now = chrono::Utc::now();
    let budget = AgentBudget {
        max_tokens: 1,
        max_terminals: 1,
    };
    let mut agent = AgentRecord {
        completion: None,
        id,
        parent_id: None,
        name: "historical agent".into(),
        task: "owned fixture".into(),
        status: AgentStatus::Running,
        policy: AgentPolicy {
            access: AccessMode::ReadOnly,
            readable_roots: vec![],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: None,
        branch: None,
        created_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
        recent_progress: vec![],
        result: None,
        error: None,
    };
    f.tool.agents.create(agent.clone()).await.unwrap();
    let empty = f.snapshot().await;
    assert!(
        f.tool
            .execute(
                json!({"action":"adopt","kind":"agent","id":id.0,"revision":empty.revision}),
                &f.context
            )
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await.total, 0);
    agent.status = AgentStatus::Completed;
    agent.result = Some("Observed fixture result".into());
    agent.finished_at = Some(now);
    f.tool.agents.update(agent).await.unwrap();
    f.call(json!({"action":"adopt","kind":"agent","id":id.0,"revision":empty.revision}))
        .await;
    let record = f
        .call(json!({"action":"read","kind":"agent","id":id.0}))
        .await;
    assert_eq!(record["status"], "completed");
    assert_eq!(record["result"], "Observed fixture result");
    let ready = f.snapshot().await;
    f.call(f.account(
        "agent",
        id.0,
        &ready,
        "incorporated",
        "Read the exact terminal agent result",
    ))
    .await;
    assert_eq!(
        f.tool.agents.get(id).await.unwrap().unwrap().status,
        AgentStatus::Completed
    );
    assert_eq!(f.snapshot().await.total, 1);
}

#[tokio::test]
async fn adapter_refuses_missing_run_invalid_fields_cancellation_and_bounded_contention() {
    let mut f = Fixture::new().await;
    let original = f.context.completion.take();
    assert!(matches!(
        f.tool
            .execute(json!({"action":"snapshot"}), &f.context)
            .await,
        Err(ToolError::Failed(_))
    ));
    f.context.completion = original;
    for args in [
        json!({"action":"snapshot","revision":0}),
        json!({"action":"read","kind":"todo"}),
        json!({"action":"account","kind":"todo","id":Uuid::new_v4()}),
        json!({"action":"delete"}),
    ] {
        assert!(matches!(
            f.tool.execute(args, &f.context).await,
            Err(ToolError::InvalidArguments(_))
        ));
    }
    f.context.cancellation.cancel();
    assert!(matches!(
        f.tool
            .execute(json!({"action":"invalid"}), &f.context)
            .await,
        Err(ToolError::Cancelled)
    ));
    f.context.cancellation = Default::default();
    f.context.timeout = Duration::from_millis(20);
    let guard = f
        .context
        .completion
        .as_ref()
        .unwrap()
        .coordinator()
        .lock()
        .await
        .unwrap();
    assert!(matches!(
        f.tool
            .execute(json!({"action":"snapshot"}), &f.context)
            .await,
        Err(ToolError::Timeout(_))
    ));
    drop(guard);
    f.context.timeout = Duration::from_secs(3);
    assert_eq!(f.snapshot().await.total, 0);
    f.context.max_output_bytes = 32;
    let bounded = f
        .tool
        .execute(json!({"action":"snapshot"}), &f.context)
        .await
        .unwrap();
    assert!(bounded.len() < 512);
    assert!(bounded.contains("truncated"));
    assert_eq!(
        f.context
            .completion
            .as_ref()
            .unwrap()
            .snapshot(&f.tool.todos, &f.tool.agents, 64)
            .await
            .unwrap()
            .total,
        0
    );
}

#[test]
fn provider_schema_allows_each_action_only_with_its_own_fields() {
    let definition = CompletionTool::new(
        Arc::new(TodoStore::new(
            std::path::PathBuf::from("unused"),
            TodoScope::workspace(std::path::PathBuf::from("unused")),
        )),
        AgentTreeStore::new("unused".into()),
    )
    .definition();
    assert_eq!(definition.name, "completion");
    let schema = crate::tools::schema::CompiledSchema::compile(&definition.input_schema).unwrap();
    for example in definition.input_schema["examples"].as_array().unwrap() {
        schema.validate(example).unwrap();
        let mut extra = example.clone();
        extra["unknown"] = json!(true);
        assert!(schema.validate(&extra).is_err());
    }
    assert!(
        schema
            .validate(&json!({"action":"read","kind":"todo","id":Uuid::new_v4(),"revision":0}))
            .is_err()
    );
    assert!(
        schema
            .validate(&json!({"action":"snapshot","fingerprint":"invented"}))
            .is_err()
    );
}
