//! Synthetic driver for the real approval frontend; never calls GitHub.
#[cfg(unix)]
#[test]
fn terminal_driver() {
    let Ok(mode) = std::env::var("HELM_GITHUB_APPROVAL_DRIVER") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        use crate::tools::{ApprovalRequest, InteractionMode};
        use tokio::signal::unix::{SignalKind, signal};
        let mut stop = signal(SignalKind::user_defined1()).unwrap();
        let request = ApprovalRequest {
            id: uuid::Uuid::nil(),
            execution_id: uuid::Uuid::nil(),
            action: "github.publish".into(),
            target: "https://github.com/fixture/repository/issues/1".into(),
            reason: (0..50)
                .map(|index| format!("BODY_{index:03}: Unicode 🧭 e\u{301} 日本語; escaped \\u001b\\u202e\n"))
                .collect::<String>()
                + "FINAL_EXACT_BODY",
            mode: if mode == "unattended" {
                InteractionMode::Unattended
            } else {
                InteractionMode::Attended
            },
        };
        let outcome = tokio::select! {
            biased;
            _ = stop.recv() => "Cancelled".to_owned(),
            outcome = super::approval::approve_terminal(&request) => format!("{outcome:?}"),
        };
        println!("DRIVER_OUTCOME={outcome}");
    });
}

#[cfg(unix)]
#[test]
fn terminal_approval_pty_contract() {
    let status = std::process::Command::new("python3")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/system/github_approval.py"))
        .arg("--test-binary")
        .arg(std::env::current_exe().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "actual GitHub approval PTY fixture failed");
}
