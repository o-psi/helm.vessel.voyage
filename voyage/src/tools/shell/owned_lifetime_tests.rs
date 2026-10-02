//! Real managed-shell job/drain contracts; fixed ordinary harmless commands.
use super::*;
use crate::tools::process::owned_lifetime_tests::owned;
use crate::tools::{ApprovalOutcome, ApprovalRequest, Approver, ToolRegistry};
use std::sync::atomic::AtomicUsize;

macro_rules! case {
    ($name:ident,$body:block)=>{#[test]fn $name(){owned::run(concat!(module_path!(),"::",stringify!($name)),||tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async $body));}};
}
struct Rig {
    shell: ManagedShell,
    registry: Arc<ToolRegistry>,
    context: ToolContext,
}
impl Rig {
    fn new() -> Self {
        let shell = ManagedShell::new();
        let mut registry = ToolRegistry::default();
        registry.register(shell.clone());
        Self {
            shell,
            registry: Arc::new(registry),
            context: owned::context(),
        }
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.context.policy.workspace().join(name)
    }
    fn launch(
        &self,
        command: String,
    ) -> tokio::task::JoinHandle<Result<crate::tools::ToolReport, ToolError>> {
        let registry = self.registry.clone();
        let context = self.context.clone();
        tokio::spawn(async move {
            registry
                .execute_report_with_workflow_secrets(
                    "shell",
                    json!({"command":command}),
                    &context,
                    None,
                )
                .await
        })
    }
    async fn finish(mut self) {
        let report = self.shell.shutdown(Duration::from_secs(4)).await;
        assert!(report.observation_complete && report.remaining.is_empty());
        assert!(!self.shell.has_owned_work());
        drop(std::mem::replace(
            &mut self.registry,
            Arc::new(ToolRegistry::default()),
        ));
        owned::until(|| self.shell.can_retire()).await;
    }
    async fn witnessed(&self, name: &str) -> (u32, String) {
        let path = self.path(name);
        owned::until(|| std::fs::metadata(&path).is_ok_and(|v| v.len() > 0)).await;
        let text = std::fs::read_to_string(path).unwrap();
        let pid: u32 = text.trim().parse().unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        assert_eq!(fields[3].parse::<u32>().unwrap(), pid);
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(format!("/proc/{pid}")).unwrap().uid(),
            unsafe { libc::geteuid() }
        );
        assert!(crate::tools::process::SessionIdentity::capture(pid).is_ok());
        owned::witness(pid, fields[19]);
        (pid, fields[19].into())
    }
    fn held(&self, name: &str) -> String {
        format!(
            "printf '%s' $$ > '{}.next'; mv '{}.next' '{}'; i=0; while [ ! -f '{}' ] && [ $i -lt 400 ]; do sleep .02; i=$((i+1)); done",
            self.path(name).display(),
            self.path(name).display(),
            self.path(name).display(),
            self.path("release").display()
        )
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        if !self.shell.has_owned_work() {
            return;
        }
        let shell = self.shell.clone();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(shell.shutdown(Duration::from_secs(4)))
                .observation_complete
        });
        if !worker.join().unwrap_or(false) {
            eprintln!("owned shell failure cleanup unconfirmed; private evidence retained");
        }
    }
}

async fn complete(
    task: tokio::task::JoinHandle<Result<crate::tools::ToolReport, ToolError>>,
) -> Result<crate::tools::ToolReport, ToolError> {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
}
fn gone(pid: u32) {
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

struct HeldApproval {
    entered: Arc<AtomicBool>,
    release: Arc<tokio::sync::Notify>,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl Approver for HeldApproval {
    async fn approve(&self, _: &ApprovalRequest) -> ApprovalOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.store(true, Ordering::Release);
        self.release.notified().await;
        ApprovalOutcome::Approved
    }
}
#[derive(Debug)]
struct Authority(Arc<AtomicBool>);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.0.load(Ordering::Acquire), "owned authority revoked");
        Ok(())
    }
}

case!(
    cancelled_or_denied_admission_has_no_process_marker_or_owned_job,
    {
        let r = Rig::new();
        r.context.cancellation.cancel();
        let command = format!("printf effect > '{}'", r.path("effect").display());
        assert!(complete(r.launch(command)).await.is_err());
        assert!(!r.path("effect").exists());
        assert!(!r.shell.has_owned_work());
        r.finish().await;
        for access in [
            crate::config::AccessMode::ReadOnly,
            crate::config::AccessMode::Approval,
        ] {
            let mut refused = Rig::new();
            let config = crate::Config {
                access: Some(access),
                ..Default::default()
            };
            refused.context.policy = Arc::new(
                crate::policy::Policy::new(&config, refused.context.policy.workspace().into())
                    .unwrap(),
            );
            assert!(
                complete(refused.launch(format!(
                    "printf effect > '{}'",
                    refused.path("effect").display()
                )))
                .await
                .is_err()
            );
            assert!(!refused.path("effect").exists());
            assert!(!refused.shell.has_owned_work());
            refused.finish().await;
        }
    }
);
case!(
    held_approval_rechecks_authority_before_spawn_without_effect_replay,
    {
        let mut r = Rig::new();
        let authority = Arc::new(AtomicBool::new(true));
        let config = crate::Config {
            access: Some(crate::config::AccessMode::Approval),
            ..Default::default()
        };
        r.context.policy = Arc::new(
            crate::policy::Policy::new(&config, r.context.policy.workspace().into())
                .unwrap()
                .with_execution_authority(Arc::new(Authority(authority.clone()))),
        );
        let entered = Arc::new(AtomicBool::new(false));
        let release = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        r.context.approver = Arc::new(HeldApproval {
            entered: entered.clone(),
            release: release.clone(),
            calls: calls.clone(),
        });
        let task = r.launch(format!("printf effect > '{}'", r.path("effect").display()));
        owned::until(|| entered.load(Ordering::Acquire)).await;
        authority.store(false, Ordering::Release);
        release.notify_one();
        assert!(complete(task).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!r.path("effect").exists());
        assert!(!r.shell.has_owned_work());
        r.finish().await;
    }
);
case!(
    actual_command_report_preserves_exit_and_bounded_capture_then_observed_retirement,
    {
        let mut r = Rig::new();
        r.context.max_output_bytes = 128;
        let report = complete(r.launch("printf '%0400d' 0; exit 9".into()))
            .await
            .unwrap();
        assert_eq!(
            report.outcome.command,
            Some(voyage_protocol::tool_result::CommandOutcome::Exited { code: 9 })
        );
        assert!(report.output.text_fallback().len() <= 128);
        assert!(report.outcome.incomplete.is_some());
        r.finish().await;
    }
);
case!(
    continuous_stdout_stderr_flood_stays_bounded_and_cancellation_retires_session,
    {
        let mut r = Rig::new();
        r.context.max_output_bytes = 256;
        let task=r.launch(format!("printf '%s' $$ > '{}.next'; mv '{}.next' '{}'; while :; do printf 'PUBLIC-OUTPUT'; printf 'PUBLIC-ERROR' >&2; done",r.path("pid").display(),r.path("pid").display(),r.path("pid").display()));
        let (pid, _) = r.witnessed("pid").await;
        r.context.cancellation.cancel();
        assert!(complete(task).await.is_err());
        r.finish().await;
        gone(pid);
    }
);
case!(
    leader_exit_with_same_session_descendant_pipe_retains_cleanup_until_shutdown,
    {
        let r = Rig::new();
        let command = format!(
            "sleep 20 & printf '%s' $! > '{}.next'; mv '{}.next' '{}'; printf '%s' $$ > '{}.next'; mv '{}.next' '{}'; printf result-before-descendant; exit 0",
            r.path("descendant").display(),
            r.path("descendant").display(),
            r.path("descendant").display(),
            r.path("pid").display(),
            r.path("pid").display(),
            r.path("pid").display()
        );
        let task = r.launch(command);
        let (pid, _) = r.witnessed("pid").await;
        let descendant: u32 = std::fs::read_to_string(r.path("descendant"))
            .unwrap()
            .parse()
            .unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{descendant}/stat")).unwrap();
        let fields = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        assert_eq!(fields[3].parse::<u32>().unwrap(), pid);
        let descendant_start = fields[19].to_string();
        // The descendant retains stdout; capture completion waits for EOF rather
        // than turning a leader exit alone into a cleaned session.
        assert!(r.shell.has_owned_work());
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
        let shutdown = r.shell.shutdown(Duration::from_secs(4)).await;
        assert!(shutdown.observation_complete);
        assert!(shutdown.remaining.is_empty());
        assert!(complete(task).await.is_err());
        r.finish().await;
        gone(pid);
        // A grandchild may be a zombie owned by the host reaper. This is positive
        // absence of the witnessed live session member, not a direct-child reap.
        assert!(
            std::fs::read_to_string(format!("/proc/{descendant}/stat")).map_or(true, |stat| {
                let fields = stat
                    .rsplit_once(") ")
                    .unwrap()
                    .1
                    .split_whitespace()
                    .collect::<Vec<_>>();
                fields[19] != descendant_start || matches!(fields[0], "Z" | "X")
            })
        );
    }
);
case!(
    timeout_kills_owned_session_before_reporting_complete_shutdown,
    {
        let mut r = Rig::new();
        r.context.timeout = Duration::from_millis(250);
        let task = r.launch(r.held("pid"));
        let (pid, _) = r.witnessed("pid").await;
        assert!(matches!(complete(task).await, Err(ToolError::Timeout(_))));
        r.finish().await;
        gone(pid);
    }
);
case!(
    dropping_waiting_dispatch_cancels_owned_job_without_second_command,
    {
        let r = Rig::new();
        let task = r.launch(r.held("pid"));
        let (pid, _) = r.witnessed("pid").await;
        task.abort();
        assert!(
            task.await
                .as_ref()
                .err()
                .is_some_and(|error| error.is_cancelled())
        );
        let report = r.shell.shutdown(Duration::from_secs(4)).await;
        assert!(report.observation_complete && report.remaining.is_empty());
        r.finish().await;
        gone(pid);
    }
);
case!(
    shutdown_closes_admission_while_observing_the_existing_job,
    {
        let r = Rig::new();
        let task = r.launch(r.held("pid"));
        let (pid, _) = r.witnessed("pid").await;
        let first = r.shell.shutdown(Duration::ZERO).await;
        assert!(!first.observation_complete && !first.remaining.is_empty());
        assert!(
            complete(r.launch(format!(
                "printf forbidden > '{}'",
                r.path("second").display()
            )))
            .await
            .is_err()
        );
        assert!(!r.path("second").exists());
        assert!(complete(task).await.is_err());
        r.finish().await;
        gone(pid);
    }
);
case!(
    clone_retirement_requires_empty_observed_work_and_last_external_handle,
    {
        let r = Rig::new();
        let held = r.shell.clone();
        let report = complete(r.launch("printf visible".into())).await.unwrap();
        assert!(report.output.text_fallback().contains("visible"));
        assert!(!r.shell.can_retire());
        let report = r.shell.shutdown(Duration::from_secs(4)).await;
        assert!(report.observation_complete);
        assert!(!r.shell.can_retire());
        drop(held);
        r.finish().await;
    }
);
case!(
    cancelled_private_workflow_binding_never_releases_synthetic_environment,
    {
        use crate::workflow::{Document, secrets::SecretInputs};
        let r = Rig::new();
        let document:Document=serde_json::from_value(json!({"schema_version":1,"id":"owned","version":"1","description":"owned","prompt":"owned","parameters":{"value":{"type":"string","secret":true}}})).unwrap();
        let bindings = SecretInputs::collect(
            &document,
            vec![("value".into(), "synthetic-private-owned-only".into())],
        )
        .unwrap()
        .bind(r.context.execution_id)
        .unwrap();
        let mut context = r.context.clone();
        context.cancellation = CancellationToken::new();
        context.cancellation.cancel();
        let result=r.registry.execute_report_with_workflow_secrets("shell",json!({"command":format!("printf '%s' \"$HELM_WORKFLOW_VALUE\" > '{}'",r.path("private-effect").display()),"workflow_secrets":["value"]}),&context,Some(&bindings)).await;
        assert!(result.is_err());
        assert!(!r.path("private-effect").exists());
        assert!(!r.shell.has_owned_work());
        r.finish().await;
    }
);
