use super::*;
fn record() -> AgentRecord {
    let budget = crate::subagent::AgentBudget {
        max_tokens: 10,
        max_terminals: 1,
    };
    AgentRecord {
        completion: None,
        id: AgentId::new(),
        parent_id: None,
        name: "fixture".into(),
        task: "task".into(),
        status: AgentStatus::Queued,
        policy: crate::subagent::AgentPolicy {
            access: crate::config::AccessMode::ReadOnly,
            readable_roots: vec![],
            writable_roots: vec![],
            allowed_tools: Default::default(),
            approval: crate::subagent::ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: None,
        branch: None,
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        updated_at: Utc::now(),
        recent_progress: vec![],
        result: None,
        error: None,
    }
}
#[test]
fn tree_recovery_prunes_only_terminal_unprotected_leaves_without_worktrees() {
    let mut tree = AgentTree::default();
    let parent = record();
    tree.insert(parent.clone()).unwrap();
    assert!(tree.insert(parent.clone()).is_err());
    let mut child = record();
    child.parent_id = Some(parent.id);
    tree.insert(child.clone()).unwrap();
    let mut missing = record();
    missing.parent_id = Some(AgentId::new());
    assert!(tree.insert(missing).is_err());
    assert_eq!(tree.children(parent.id).len(), 1);
    assert_eq!(tree.recover_after_restart(), 2);
    assert_eq!(tree.recover_after_restart(), 0);
    assert!(
        tree.agents[&child.id]
            .error
            .as_ref()
            .unwrap()
            .contains("did not survive")
    );
    assert!(tree.prune_terminal_leaves(0, Some(child.id)).is_empty());
    tree.agents.get_mut(&child.id).unwrap().worktree = Some("retained".into());
    assert!(tree.prune_terminal_leaves(0, None).is_empty());
    tree.agents.get_mut(&child.id).unwrap().worktree = None;
    assert_eq!(
        tree.prune_terminal_leaves(0, None),
        vec![child.id, parent.id]
    );
}
#[tokio::test]
async fn persisted_tree_requires_writer_and_archives_immutable_final_records() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agents.json");
    let mut store = AgentTreeStore::new(path.clone());
    let mut item = record();
    store.acquire_runtime_owner().unwrap();
    assert!(AgentTreeStore::new(path).acquire_runtime_owner().is_ok());
    store.create(item.clone()).await.unwrap();
    item.status = AgentStatus::Running;
    store.update(item.clone()).await.unwrap();
    assert_eq!(store.recover_after_restart().await.unwrap(), 1);
    let recovered = store.get(item.id).await.unwrap().unwrap();
    assert_eq!(recovered.status, AgentStatus::Interrupted);
    assert_eq!(
        store.prune_terminal_leaves(0, None).await.unwrap(),
        vec![item.id]
    );
    assert!(store.list().await.unwrap().is_empty());
    let archived = store.get_archived(item.id).await.unwrap().unwrap();
    assert_eq!(archived.record, recovered);
    assert_eq!(store.list_archived(None, 1).await.unwrap().agents.len(), 1);
    assert!(store.list_archived(None, 0).await.is_err());
    let archive = crate::subagent::archive::AgentArchive::new(&store.path);
    archive.put(&recovered).await.unwrap();
    let mut changed = recovered;
    changed.name = "changed".into();
    assert!(archive.put(&changed).await.is_err());
}
