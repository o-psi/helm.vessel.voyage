//! Actual public ToolRegistry/InteractiveTerminals lifetimes in private children.
use super::*;
#[path = "../owned_child_lifetime_tests.rs"]
pub(crate) mod owned;
use std::{collections::BTreeSet, time::Duration};

macro_rules! case {
    ($name:ident,$body:block)=>{#[test]fn $name(){owned::run(concat!(module_path!(),"::",stringify!($name)),||tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async $body));}};
}
struct Rig {
    context: ToolContext,
    tool: ProcessTool,
    registry: super::super::ToolRegistry,
    pids: Vec<(u32, String)>,
}
impl Rig {
    fn new(limit: usize, capture: usize) -> Self {
        let tool = ProcessTool::with_limits(limit, capture);
        let mut registry = super::super::ToolRegistry::default();
        registry.register(tool.clone());
        Self {
            context: owned::context(),
            tool,
            registry,
            pids: Vec::new(),
        }
    }
    async fn call(&self, value: Value) -> Result<String, ToolError> {
        self.registry.execute("process", value, &self.context).await
    }
    async fn start(&mut self, name: &str, command: &str) -> Uuid {
        let before = self
            .tool
            .metadata()
            .unwrap()
            .iter()
            .map(|v| v.id)
            .collect::<BTreeSet<_>>();
        self.call(json!({"action":"start","name":name,"command":command}))
            .await
            .unwrap();
        let id = self
            .tool
            .metadata()
            .unwrap()
            .into_iter()
            .find(|v| !before.contains(&v.id))
            .unwrap()
            .id;
        let pid = self.tool.processes.lock().unwrap()[&id]
            .child
            .process_id()
            .unwrap();
        let tail = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields = tail
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
        owned::witness(pid, fields[19]);
        self.pids.push((pid, fields[19].into()));
        id
    }
    async fn finish(mut self) {
        let report = self.tool.shutdown(Duration::from_secs(4)).await;
        assert!(
            report.observation_complete
                && report.remaining.is_empty()
                && report.failures.is_empty()
        );
        for (pid, _) in &self.pids {
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        }
        assert!(!self.tool.has_owned_work());
        drop(std::mem::take(&mut self.registry));
        owned::until(|| self.tool.can_retire()).await;
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if !self.tool.has_owned_work() {
            return;
        }
        let tool = self.tool.clone();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(tool.shutdown(Duration::from_secs(4)))
                .observation_complete
        });
        if !worker.join().unwrap_or(false) {
            eprintln!("owned PTY failure cleanup unconfirmed; private evidence retained");
        }
    }
}

case!(two_named_terminals_keep_input_and_selection_on_exact_ids, {
    let mut r = Rig::new(2, 4096);
    let a = r.start("a", "/bin/cat").await;
    let b = r.start("b", "/bin/cat").await;
    r.call(json!({"action":"select","name":"a"})).await.unwrap();
    r.call(json!({"action":"write","data":"owned-a-marker\n"}))
        .await
        .unwrap();
    owned::until(|| {
        r.tool.processes.lock().unwrap()[&a]
            .output
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("owned-a-marker")
    })
    .await;
    assert!(
        !r.tool.processes.lock().unwrap()[&b]
            .output
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("owned-a-marker")
    );
    r.call(json!({"action":"rename","id":a,"name":"renamed-a"}))
        .await
        .unwrap();
    assert!(r.call(json!({"action":"read","name":"a"})).await.is_err());
    assert!(
        r.call(json!({"action":"read","name":"renamed-a"}))
            .await
            .unwrap()
            .contains("owned-a-marker")
    );
    r.finish().await;
});
case!(
    malformed_identity_names_cwd_and_sizes_preserve_live_sibling,
    {
        let mut r = Rig::new(2, 4096);
        let a = r.start("a", "/bin/cat").await;
        let before = r.tool.metadata().unwrap();
        for command in [
            json!({"action":"start","name":"a","command":"true"}),
            json!({"action":"start","command":"true","cwd":r.context.policy.workspace().join("missing-cwd-353")}),
            json!({"action":"start","command":"true","cols":0}),
            json!({"action":"rename","id":Uuid::new_v4(),"name":"x"}),
            json!({"action":"select","name":"absent"}),
            json!({"action":"read","id":a,"unknown":"field"}),
        ] {
            assert!(r.call(command).await.is_err());
        }
        let after = r.tool.metadata().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, before[0].id);
        assert_eq!(after[0].name, before[0].name);
        r.finish().await;
    }
);
case!(capacity_recovers_only_after_actual_terminal_retirement, {
    let mut r = Rig::new(1, 4096);
    let a = r.start("a", "/bin/cat").await;
    assert!(
        r.call(json!({"action":"start","command":"/bin/cat","name":"b"}))
            .await
            .is_err()
    );
    r.call(json!({"action":"terminate","id":a})).await.unwrap();
    owned::until(|| r.tool.pending.lock().unwrap().is_empty()).await;
    assert!(!std::path::Path::new(&format!("/proc/{}", r.pids[0].0)).exists());
    let b = r.start("b", "/bin/cat").await;
    assert_ne!(a, b);
    r.finish().await;
});
case!(
    postspawn_failure_retains_real_child_until_explicit_shutdown,
    {
        let r = Rig::new(1, 4096);
        let failed = r.tool.start_after_spawn(
            "/bin/cat".into(),
            Some("failed".into()),
            None,
            Default::default(),
            24,
            80,
            &r.context,
            || {
                Err(ToolError::Failed(
                    "owned after-spawn fixture failure".into(),
                ))
            },
        );
        assert!(failed.is_err());
        assert!(r.tool.metadata().unwrap().is_empty());
        assert!(r.tool.has_owned_work());
        assert_eq!(r.tool.pending.lock().unwrap().len(), 1);
        let pid = r
            .tool
            .pending
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .process_id()
            .unwrap();
        assert!(
            r.call(json!({"action":"start","command":"true"}))
                .await
                .is_err()
        );
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        owned::witness(pid, fields[19]);
        r.finish().await;
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
);
case!(
    bounded_unicode_capture_reports_gap_and_advances_read_cursor,
    {
        let mut r = Rig::new(1, 1024);
        r.context.max_output_bytes = 256;
        // Literal fixture output exercises the admitted production command
        // path; arithmetic expansion is deliberately refused by shell policy.
        let command = format!("printf '%s' '{}'; /bin/cat", "界".repeat(900));
        let id = r.start("bounded", &command).await;
        owned::until(|| {
            r.tool.processes.lock().unwrap()[&id]
                .output
                .lock()
                .unwrap()
                .dropped
                > 0
        })
        .await;
        assert!(
            r.tool
                .snapshot(TerminalId(id))
                .await
                .unwrap()
                .dropped_unread_bytes
                > 0
        );
        let first = r.call(json!({"action":"read","id":id})).await.unwrap();
        assert!(first.len() < 1024);
        assert!(first.contains("unread bytes remain"));
        let cursor = r.tool.processes.lock().unwrap()[&id].cursor;
        let second = r.call(json!({"action":"read","id":id})).await.unwrap();
        assert!(second.len() < 1024);
        assert!(r.tool.processes.lock().unwrap()[&id].cursor >= cursor);
        r.finish().await;
    }
);
case!(
    synthetic_human_attach_permanently_fences_model_input_and_capture,
    {
        let mut r = Rig::new(2, 4096);
        let a = r.start("a", "/bin/cat").await;
        let b = r.start("b", "/bin/cat").await;
        r.tool.attach(TerminalId(a)).await.unwrap();
        r.tool
            .write(TerminalId(a), b"synthetic-private-canary\n".to_vec())
            .await
            .unwrap();
        owned::until(|| {
            r.tool.processes.lock().unwrap()[&a]
                .output
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("synthetic-private-canary")
        })
        .await;
        assert!(
            r.call(json!({"action":"write","id":a,"data":"forbidden-model-suffix"}))
                .await
                .is_err()
        );
        let read = r.call(json!({"action":"read","id":a})).await.unwrap();
        assert!(!read.contains("synthetic-private-canary") && read.contains("private"));
        assert!(
            r.tool.processes.lock().unwrap()[&a]
                .output
                .lock()
                .unwrap()
                .bytes
                .is_empty()
        );
        r.call(json!({"action":"write","id":b,"data":"public-b-marker\n"}))
            .await
            .unwrap();
        owned::until(|| {
            r.tool.processes.lock().unwrap()[&b]
                .output
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("public-b-marker")
        })
        .await;
        assert!(
            r.call(json!({"action":"read","id":b}))
                .await
                .unwrap()
                .contains("public-b-marker")
        );
        r.finish().await;
    }
);
case!(
    cancelled_and_oversized_input_cannot_queue_a_suffix_or_replay,
    {
        let mut r = Rig::new(1, 4096);
        let id = r.start("a", "/bin/cat").await;
        let mut cancelled = r.context.clone();
        cancelled.cancellation = tokio_util::sync::CancellationToken::new();
        cancelled.cancellation.cancel();
        assert!(
            r.registry
                .execute(
                    "process",
                    json!({"action":"write","id":id,"data":"forbidden-cancelled"}),
                    &cancelled
                )
                .await
                .is_err()
        );
        assert!(
            r.call(json!({"action":"write","id":id,"data":"x".repeat(65537)}))
                .await
                .is_err()
        );
        r.call(json!({"action":"write","id":id,"data":"accepted-once\n"}))
            .await
            .unwrap();
        owned::until(|| {
            r.tool.processes.lock().unwrap()[&id]
                .output
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("accepted-once")
        })
        .await;
        assert!(
            !r.tool.processes.lock().unwrap()[&id]
                .output
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("forbidden-cancelled")
        );
        r.finish().await;
    }
);
case!(
    typed_screen_resize_keeps_private_output_and_exact_terminal,
    {
        let mut r = Rig::new(1, 4096);
        let id = r
            .start("a", r"printf '\033[31mREADY\033[0m'; /bin/cat")
            .await;
        owned::until(|| {
            r.tool.processes.lock().unwrap()[&id]
                .output
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains("READY")
        })
        .await;
        let before = r.tool.attach(TerminalId(id)).await.unwrap();
        crate::terminal::InteractiveTerminals::resize(&r.tool, TerminalId(id), 90, 30)
            .await
            .unwrap();
        let after = r.tool.snapshot(TerminalId(id)).await.unwrap();
        assert_eq!(after.id, TerminalId(id));
        assert!(after.revision > before.revision);
        assert_eq!(after.cells.len(), 30);
        assert_eq!(after.cells[0].len(), 90);
        assert!(
            crate::terminal::InteractiveTerminals::resize(&r.tool, TerminalId(id), 0, 30)
                .await
                .is_err()
        );
        assert!(
            r.call(json!({"action":"write","id":id,"data":"blocked"}))
                .await
                .is_err()
        );
        r.finish().await;
    }
);
case!(interrupt_and_terminate_leave_owned_sibling_usable, {
    let mut r = Rig::new(2, 4096);
    let a = r
        .start(
            "a",
            "trap 'printf \"INT_SEEN\\n\"' INT; printf 'INT_READY\\n'; while :; do sleep .1; done",
        )
        .await;
    let b = r.start("b", "/bin/cat").await;
    owned::until(|| {
        r.tool.processes.lock().unwrap()[&a]
            .output
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("INT_READY")
    })
    .await;
    r.call(json!({"action":"interrupt","id":a})).await.unwrap();
    owned::until(|| {
        r.tool.processes.lock().unwrap()[&a]
            .output
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("INT_SEEN")
    })
    .await;
    r.call(json!({"action":"write","id":b,"data":"sibling-still-live\n"}))
        .await
        .unwrap();
    owned::until(|| {
        r.tool.processes.lock().unwrap()[&b]
            .output
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("sibling-still-live")
    })
    .await;
    r.call(json!({"action":"terminate","id":a})).await.unwrap();
    assert_eq!(r.tool.metadata().unwrap()[0].id, b);
    r.finish().await;
});
case!(
    busy_zero_budget_shutdown_preserves_obligation_until_observed,
    {
        let mut r = Rig::new(1, 4096);
        let id = r.start("a", "/bin/cat").await;
        let starting = r.tool.starting.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _lock = starting.lock().unwrap();
            entered_tx.send(()).unwrap();
            // Hold the real admission lock on another thread, with a finite
            // refusal bound; no synchronous guard is held across this await.
            let _ = release_rx.recv_timeout(Duration::from_secs(4));
        });
        entered_rx.recv_timeout(Duration::from_secs(4)).unwrap();
        let report = r.tool.shutdown(Duration::ZERO).await;
        assert!(!report.observation_complete);
        assert!(r.tool.has_owned_work());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            r.call(json!({"action":"write","id":id,"data":"blocked-after-shutdown"}))
                .await
                .is_err()
        );
        r.finish().await;
    }
);
