//! Disposable Git checks, never touching the caller's repository or policy.
use super::*;
use std::{path::Path, process::Command, sync::Arc};
fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "Git fixture command failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn discovery_delegation_and_dirty_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let children = tmp.path().join("children");
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir(&children).unwrap();
    assert!(
        WorktreeManager::discover(&repo, children.clone())
            .unwrap()
            .is_none()
    );
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join("file"), "base\n").unwrap();
    git(&repo, &["add", "file"]);
    git(&repo, &["commit", "-m", "fixture"]);
    let manager = WorktreeManager::discover(&repo, children.clone())
        .unwrap()
        .unwrap();
    for (read, write) in [(false, false), (true, false), (false, true)] {
        let config = crate::Config {
            access: Some(crate::config::AccessMode::Unrestricted),
            allow_read: if read { vec![children.clone()] } else { vec![] },
            allow_write: if write {
                vec![children.clone()]
            } else {
                vec![]
            },
            ..Default::default()
        };
        let policy = Arc::new(crate::policy::Policy::new(&config, repo.clone()).unwrap());
        assert!(
            manager
                .clone()
                .with_policy(policy)
                .create("refused", "HEAD")
                .is_err()
        );
        assert!(!children.join("refused").exists());
    }
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        allow_read: vec![children.clone()],
        allow_write: vec![children.clone()],
        ..Default::default()
    };
    let policy = Arc::new(crate::policy::Policy::new(&config, repo.clone()).unwrap());
    let manager = manager.with_policy(policy);
    let lease = manager.create("fixture", "HEAD").unwrap();
    assert!(
        WorktreeManager::discover(&lease.path, tmp.path().join("nested"))
            .unwrap()
            .is_some()
    );
    std::fs::write(lease.path.join("file"), "changed\n").unwrap();
    assert!(manager.remove(&lease).is_err());
    assert_eq!(
        std::fs::read_to_string(lease.path.join("file")).unwrap(),
        "changed\n"
    );
    manager.commit(&lease, "fixture change").unwrap();
    manager.integrate(&lease, "main").unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.join("file")).unwrap(),
        "changed\n"
    );
    manager.remove(&lease).unwrap();
    assert!(!lease.path.exists());
    let alternate = tmp.path().join("alternate");
    std::fs::create_dir(&alternate).unwrap();
    std::fs::create_dir(alternate.join(".local-git")).unwrap();
    git(
        &alternate,
        &["init", "--separate-git-dir=.local-git/worktree.git"],
    );
    // This synthetic pointer alone is removed, not any user metadata.
    std::fs::remove_file(alternate.join(".git")).unwrap();
    assert!(
        WorktreeManager::discover(&alternate, children)
            .unwrap()
            .is_some()
    );
    std::fs::remove_file(alternate.join(".local-git/worktree.git/HEAD")).unwrap();
    assert!(WorktreeManager::discover(&alternate, tmp.path().join("other")).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_escape_and_command_denial_still_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let outside = tmp.path().join("outside");
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, repo.join("escape")).unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        deny_commands: vec!["git".into()],
        ..Default::default()
    };
    let policy = crate::policy::Policy::new(&config, repo.clone()).unwrap();
    assert!(
        policy
            .check_delegated_workspace(&repo.join("escape/new"))
            .is_err()
    );
    assert!(
        policy
            .check_command_denials(&["git", "worktree", "add"])
            .is_err()
    );
}
struct Immediate;
#[async_trait::async_trait]
impl SubagentExecutor for Immediate {
    async fn execute(&self, _: ExecutionContext) -> Result<SubagentResult, String> {
        Ok(SubagentResult {
            summary: "fixture done".into(),
        })
    }
}
#[tokio::test]
async fn archived_observations_and_unknown_ids_are_distinct() {
    let runtime =
        SubagentRuntime::new(Arc::new(Immediate), RuntimeLimits::default(), None).unwrap();
    let policy = AgentPolicy {
        access: crate::config::AccessMode::ReadOnly,
        readable_roots: vec![],
        writable_roots: vec![],
        allowed_tools: Default::default(),
        approval: ApprovalPolicy::Deny,
        budget: AgentBudget {
            max_tokens: 1,
            max_terminals: 1,
        },
    };
    let id = runtime
        .spawn(SpawnRequest {
            name: "fixture".into(),
            task: "fixture".into(),
            parent_id: None,
            policy: policy.clone(),
            budget: policy.budget.clone(),
            worktree: None,
            branch: None,
        })
        .await
        .unwrap();
    runtime.wait(id).await.unwrap().unwrap();
    assert_eq!(
        runtime.get(id).await.unwrap().status,
        AgentStatus::Completed
    );
    assert!(matches!(
        runtime.follow_up(id, "new work").await,
        Err(RuntimeError::Archived(_))
    ));
    assert!(matches!(
        runtime.follow_up(AgentId::new(), "new work").await,
        Err(RuntimeError::Unknown(_))
    ));
    assert_eq!(
        runtime.wait(id).await.unwrap().unwrap().summary,
        "fixture done"
    );
}
