#![cfg(target_os = "linux")]
use super::*;
use crate::{
    config::{AccessMode, Config},
    policy::Policy,
    tools::{ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor},
};

struct Answer(ApprovalOutcome);
#[async_trait]
impl Approver for Answer {
    async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
        self.0.clone()
    }
}
fn context(path: &std::path::Path) -> ToolContext {
    ToolContext {
        completion: None,
        policy: Arc::new(
            Policy::new(
                &Config {
                    access: Some(AccessMode::Unrestricted),
                    ..Config::default()
                },
                path.into(),
            )
            .unwrap(),
        ),
        approver: Arc::new(Answer(ApprovalOutcome::Approved)),
        timeout: Duration::from_secs(3),
        max_output_bytes: 4096,
        environment: [("PATH".into(), std::env::var("PATH").unwrap())]
            .into_iter()
            .collect(),
        cancellation: CancellationToken::new(),
        execution_id: Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: Arc::new(Redactor::default()),
    }
}
async fn ready(path: &std::path::Path) -> Vec<u32> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                let ids = text
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<Vec<u32>, _>>();
                if let Ok(ids) = ids
                    && !ids.is_empty()
                {
                    return ids;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !matches!(
            stat.rsplit_once(") ").unwrap().1.split_whitespace().next(),
            Some("Z" | "X")
        )
    })
}
async fn cleaned(shell: &ManagedShell) {
    let report = shell.shutdown(Duration::from_secs(6)).await;
    assert!(report.observation_complete, "{report:?}");
    assert!(report.remaining.is_empty());
}
#[tokio::test]
async fn normal_output_exit_and_workspace_match_ordinary_shell() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let shell = ManagedShell::new();
    for command in [
        "printf 'hello'; printf 'problem' >&2; exit 7",
        "pwd",
        "kill -TERM $$",
    ] {
        let args = json!({"command":command});
        let expected = Shell.execute(args.clone(), &ctx).await.unwrap();
        let actual = shell.execute(args, &ctx).await.unwrap();
        assert_eq!(actual, expected);
    }
    cleaned(&shell).await;
}
#[tokio::test]
async fn output_is_bounded_even_when_both_pipes_flood() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = context(dir.path());
    ctx.max_output_bytes = 128;
    let shell = ManagedShell::new();
    let output = shell.execute(json!({"command":"python3 -c 'import os; os.write(1,b\"x\"*200000); os.write(2,b\"y\"*200000)'"}), &ctx).await.unwrap();
    assert!(output.len() < 256, "{}", output.len());
    assert!(output.contains("truncated"));
    cleaned(&shell).await;
}
#[tokio::test]
async fn policy_and_declined_approval_never_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    for mode in [AccessMode::ReadOnly, AccessMode::Approval] {
        let mut ctx = context(dir.path());
        ctx.policy = Arc::new(
            Policy::new(
                &Config {
                    access: Some(mode),
                    ..Config::default()
                },
                dir.path().into(),
            )
            .unwrap(),
        );
        ctx.approver = Arc::new(Answer(ApprovalOutcome::Denied));
        assert!(matches!(
            shell
                .execute(json!({"command":"touch forbidden"}), &ctx)
                .await,
            Err(ToolError::Denied(_))
        ));
    }
    assert!(!dir.path().join("forbidden").exists());
    cleaned(&shell).await;
}
#[tokio::test]
async fn timeout_and_cancellation_reap_direct_child() {
    for cancel in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let shell = ManagedShell::new();
        let mut ctx = context(dir.path());
        ctx.timeout = if cancel {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(2)
        };
        let token = ctx.cancellation.clone();
        let copy = shell.clone();
        let task = tokio::spawn(async move {
            copy.execute(json!({"command":"echo $$ > ready; exec sleep 30"}), &ctx)
                .await
        });
        let pid = ready(&dir.path().join("ready")).await[0];
        if cancel {
            token.cancel();
        }
        let result = task.await.unwrap();
        if cancel {
            assert!(matches!(result, Err(ToolError::Cancelled)));
        } else {
            assert!(matches!(result, Err(ToolError::Timeout(_))));
        }
        cleaned(&shell).await;
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
}
#[tokio::test]
async fn abandoned_execute_remains_owned_until_observed() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    let copy = shell.clone();
    let task = tokio::spawn(async move {
        copy.execute(json!({"command":"echo $$ > ready; exec sleep 30"}), &ctx)
            .await
    });
    let pid = ready(&dir.path().join("ready")).await[0];
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    cleaned(&shell).await;
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
#[tokio::test]
async fn successful_background_output_keeps_session_owned_without_cancelling_it() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    std::fs::write(
        dir.path().join("background.py"),
        r#"import os,time
child=os.fork()
if child == 0:
 os.setpgid(0,0)
 os.close(1); os.close(2)
 while True: time.sleep(1)
open('ready','w').write(str(os.getpid())+' '+str(child))
print('background-started',flush=True)
"#,
    )
    .unwrap();
    let output = shell
        .execute(json!({"command":"exec python3 background.py"}), &ctx)
        .await
        .unwrap();
    let ids = ready(&dir.path().join("ready")).await;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prematurely_cancelled = shell
        .state
        .lock()
        .unwrap()
        .jobs
        .values()
        .any(|job| job.cancel.is_cancelled());
    let remained_alive = alive(ids[1]);
    let root_waitable = std::path::Path::new(&format!("/proc/{}", ids[0])).exists();
    cleaned(&shell).await;
    assert!(output.contains("background-started"));
    assert!(
        !prematurely_cancelled,
        "returning successful output must disarm the abandoned-call guard"
    );
    assert!(
        remained_alive,
        "successful output must not cancel background job"
    );
    assert!(
        root_waitable,
        "leader must remain waitable while background session exists"
    );
    assert!(!alive(ids[1]));
    assert!(!std::path::Path::new(&format!("/proc/{}", ids[0])).exists());
}
#[tokio::test]
async fn shutdown_is_repeatable_and_does_not_signal_unrelated_session() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let other = ManagedShell::new();
    let ctx = context(dir.path());
    let copy = other.clone();
    let otherctx = ctx.clone();
    let task = tokio::spawn(async move {
        copy.execute(
            json!({"command":"echo $$ > other; exec sleep 30"}),
            &otherctx,
        )
        .await
    });
    let pid = ready(&dir.path().join("other")).await[0];
    let copy = shell.clone();
    let ownctx = ctx.clone();
    let owned = tokio::spawn(async move {
        copy.execute(json!({"command":"echo $$ > owned; exec sleep 30"}), &ownctx)
            .await
    });
    let owned_pid = ready(&dir.path().join("owned")).await[0];
    cleaned(&shell).await;
    assert!(matches!(owned.await.unwrap(), Err(ToolError::Cancelled)));
    assert!(!std::path::Path::new(&format!("/proc/{owned_pid}")).exists());
    assert!(shell.shutdown(Duration::ZERO).await.observation_complete);
    assert!(alive(pid));
    assert!(matches!(
        shell
            .execute(json!({"command":"touch forbidden"}), &ctx)
            .await,
        Err(ToolError::Cancelled)
    ));
    assert!(!dir.path().join("forbidden").exists());
    cleaned(&other).await;
    assert!(matches!(task.await.unwrap(), Err(ToolError::Cancelled)));
}
#[tokio::test]
async fn queued_approval_cannot_spawn_after_shutdown() {
    struct Paused {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait]
    impl Approver for Paused {
        async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
            self.entered.notify_one();
            self.release.notified().await;
            ApprovalOutcome::Approved
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let mut ctx = context(dir.path());
    ctx.policy = Arc::new(
        Policy::new(
            &Config {
                access: Some(AccessMode::Approval),
                ..Config::default()
            },
            dir.path().into(),
        )
        .unwrap(),
    );
    let approval = Arc::new(Paused {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    ctx.approver = approval.clone();
    let copy = shell.clone();
    let task = tokio::spawn(async move {
        copy.execute(json!({"command":"touch forbidden"}), &ctx)
            .await
    });
    approval.entered.notified().await;
    cleaned(&shell).await;
    approval.release.notify_one();
    assert!(matches!(task.await.unwrap(), Err(ToolError::Cancelled)));
    assert!(!dir.path().join("forbidden").exists());
}

#[tokio::test]
async fn zero_budget_shutdown_is_honest_and_a_later_call_can_observe_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    let copy = shell.clone();
    let task = tokio::spawn(async move {
        copy.execute(json!({"command":"echo $$ > ready; exec sleep 30"}), &ctx)
            .await
    });
    let pid = ready(&dir.path().join("ready")).await[0];
    let before = Instant::now();
    let report = shell.shutdown(Duration::ZERO).await;
    assert!(before.elapsed() < Duration::from_millis(100));
    if !report.observation_complete {
        assert_eq!(report.remaining.len(), 1);
    }
    cleaned(&shell).await;
    assert!(matches!(task.await.unwrap(), Err(ToolError::Cancelled)));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
#[tokio::test]
async fn failed_spawn_and_invalid_arguments_leave_no_cleanup_obligation() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    assert!(matches!(
        shell.execute(json!({"command":7}), &ctx).await,
        Err(ToolError::InvalidArguments(_))
    ));
    std::fs::remove_dir(dir.path()).unwrap();
    assert!(matches!(
        shell.execute(json!({"command":"echo never"}), &ctx).await,
        Err(ToolError::Failed(_))
    ));
    cleaned(&shell).await;
}

#[tokio::test]
async fn completed_jobs_reclaim_capacity_and_cancel_before_start_cannot_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    for _ in 0..MAX_JOBS + 2 {
        assert!(
            shell
                .execute(json!({"command":"printf ok"}), &ctx)
                .await
                .unwrap()
                .contains("ok")
        );
    }
    ctx.cancellation.cancel();
    assert!(matches!(
        shell
            .execute(json!({"command":"touch forbidden"}), &ctx)
            .await,
        Err(ToolError::Cancelled)
    ));
    assert!(!dir.path().join("forbidden").exists());
    cleaned(&shell).await;
}
#[tokio::test]
async fn finished_but_unobserved_jobs_keep_capacity_and_cleanup_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    // Inject observation failure after worker completion; there is no running
    // child in this fixture. Finished alone is deliberately insufficient proof.
    {
        let mut state = shell.state.lock().unwrap();
        for _ in 0..MAX_JOBS {
            state.jobs.insert(
                Uuid::new_v4(),
                Arc::new(Job {
                    cancel: CancellationToken::new(),
                    finished: AtomicBool::new(true),
                    observed: AtomicBool::new(false),
                    retained: Mutex::new(None),
                }),
            );
        }
    }
    let error = shell
        .execute(json!({"command":"touch forbidden"}), &context(dir.path()))
        .await
        .unwrap_err();
    assert!(matches!(error,ToolError::Failed(ref message) if message.contains("capacity")));
    assert!(!dir.path().join("forbidden").exists());
    for _ in 0..2 {
        let report = shell.shutdown(Duration::from_millis(10)).await;
        assert!(!report.observation_complete);
        assert_eq!(report.remaining.len(), MAX_JOBS);
    }
}
#[tokio::test]
async fn capture_hard_ceiling_applies_even_when_context_allows_more() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let mut ctx = context(dir.path());
    ctx.max_output_bytes = MAX_CAPTURE + 1024;
    ctx.timeout = Duration::from_secs(10);
    let output = shell
        .execute(
            json!({"command":"python3 -c 'import os; os.write(1,b\"x\"*(9*1024*1024))'"}),
            &ctx,
        )
        .await
        .unwrap();
    assert!(output.len() < MAX_CAPTURE + 100);
    assert!(output.contains(&format!("truncated at {MAX_CAPTURE} bytes")));
    cleaned(&shell).await;
}

#[tokio::test]
async fn overflowing_execution_timeout_rejects_before_any_effect() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let mut ctx = context(dir.path());
    ctx.timeout = Duration::MAX;
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        shell.execute(json!({"command":"echo $$ > overflow-effect"}), &ctx),
    )
    .await
    .expect("oversized duration must be rejected promptly");
    // The old post-spawn panic could report a channel error before the child
    // writes its marker. Wait for that bounded, immediately exiting fixture.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let effect = std::fs::read_to_string(dir.path().join("overflow-effect")).ok();
    if let Some(pid) = effect.as_deref().and_then(|s| s.trim().parse::<i32>().ok()) {
        // Regression failure cleanup only: a panicked old worker discarded its
        // std::Child. The marker identifies this fixture's own waitable child.
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
    }
    assert!(result.is_err());
    assert!(
        effect.is_none(),
        "invalid timeout spawned an effectful child"
    );
    cleaned(&shell).await;
}
#[tokio::test]
async fn overflowing_shutdown_timeout_is_bounded_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let shell = ManagedShell::new();
    let ctx = context(dir.path());
    let copy = shell.clone();
    let task = tokio::spawn(async move {
        copy.execute(json!({"command":"echo $$ > ready; exec sleep 30"}), &ctx)
            .await
    });
    let pid = ready(&dir.path().join("ready")).await[0];
    let copy = shell.clone();
    // A task boundary captures the old Instant overflow panic, allowing cleanup
    // to run before asserting that the public operation must never panic.
    let mut waiter = tokio::spawn(async move { copy.shutdown(Duration::MAX).await });
    let result = tokio::time::timeout(Duration::from_secs(1), &mut waiter).await;
    if result.is_err() {
        waiter.abort();
    }
    cleaned(&shell).await;
    assert!(matches!(task.await.unwrap(), Err(ToolError::Cancelled)));
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    let report = result
        .expect("oversized shutdown must return promptly")
        .expect("oversized shutdown duration must not panic");
    assert!(report.observation_complete || !report.remaining.is_empty());
}
