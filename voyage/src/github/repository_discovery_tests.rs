//! Local subprocess discovery never invokes credentials, hooks or remote URLs.
use super::*;
use std::{os::unix::fs::PermissionsExt, sync::Arc};

fn controlled_git(bytes: &[u8], status: u8) -> (tempfile::TempDir, crate::tools::ToolContext) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("result"), bytes).unwrap();
    let script = format!(
        "#!/bin/sh\nif [ -n \"$GIT_CONFIG_COUNT\" ]; then exit 91; fi\nprintf '%s\\n' \"$@\" > \"$PWD/argv\"\n/bin/cat \"$PWD/result\"\nexit {status}\n"
    );
    let executable = root.path().join("git");
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context
        .environment
        .insert("PATH".into(), root.path().to_string_lossy().into_owned());
    context
        .environment
        .insert("GIT_CONFIG_COUNT".into(), "1".into());
    context
        .environment
        .insert("GIT_CONFIG_KEY_0".into(), "remote.injected.url".into());
    context.environment.insert(
        "GIT_CONFIG_VALUE_0".into(),
        "https://evil.invalid/private".into(),
    );
    (root, context)
}

#[tokio::test]
async fn discovery_uses_exact_local_argv_and_marks_untrusted_remotes_unavailable() {
    let (root,mut context) = controlled_git(b"remote.origin.url\nhttps://github.com/Example/Project.git\0remote.private-name.url\nhttps://credential@evil.invalid/private\0",0);
    context.redactor = Arc::new(crate::tools::Redactor::new(["private-name".into()]));
    let candidates = discover(&context).await.unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0].repository.as_ref().unwrap().slug(),
        "example/project"
    );
    assert!(!candidates[0].unavailable);
    assert!(candidates[1].unavailable);
    assert!(candidates[1].repository.is_none());
    assert_eq!(candidates[1].remote, "[REDACTED]");
    assert_eq!(
        std::fs::read_to_string(root.path().join("argv")).unwrap(),
        "config\n--local\n--null\n--no-includes\n--get-regexp\n^remote\\..*\\.url$\n"
    );
    assert!(
        !serde_json::to_string(&candidates)
            .unwrap()
            .contains("credential")
    );
}

#[tokio::test]
async fn empty_discovery_is_valid_but_non_git_status_is_authored_refusal() {
    let (_root, context) = controlled_git(b"", 1);
    assert!(discover(&context).await.unwrap().is_empty());
    let (_root, context) = controlled_git(b"PRIVATE_DIAGNOSTIC", 2);
    let error = discover(&context).await.unwrap_err().to_string();
    assert!(error.contains("unavailable for this workspace"));
    assert!(!error.contains("PRIVATE_DIAGNOSTIC"));
}

#[tokio::test]
async fn invalid_output_encoding_key_and_remote_name_are_rejected_before_network_selection() {
    for bytes in [
        vec![0xff, 0],
        b"remote.origin.url-without-newline\0".to_vec(),
        b"url.override\nhttps://github.com/example/project\0".to_vec(),
        b"remote..url\nhttps://github.com/example/project\0".to_vec(),
        format!(
            "remote.{}.url\nhttps://github.com/example/project\0",
            "x".repeat(129)
        )
        .into_bytes(),
    ] {
        let (_root, context) = controlled_git(&bytes, 0);
        assert!(discover(&context).await.is_err());
    }
}

#[tokio::test]
async fn discovery_output_and_candidate_limits_refuse_instead_of_silently_truncating() {
    let (_root, context) = controlled_git(&vec![b'x'; 65537], 0);
    assert!(
        discover(&context)
            .await
            .unwrap_err()
            .to_string()
            .contains("configuration exceeds limit")
    );
    let records = (0..65)
        .map(|i| format!("remote.remote{i}.url\nhttps://github.com/example/project\0"))
        .collect::<String>();
    let (_root, context) = controlled_git(records.as_bytes(), 0);
    assert!(
        discover(&context)
            .await
            .unwrap_err()
            .to_string()
            .contains("remote count exceeds limit")
    );
}

#[tokio::test]
async fn missing_local_git_has_no_remote_candidate_or_subprocess_effect() {
    let (root, mut context) = controlled_git(b"", 0);
    std::fs::remove_file(root.path().join("git")).unwrap();
    context
        .environment
        .insert("PATH".into(), root.path().to_string_lossy().into_owned());
    assert!(
        discover(&context)
            .await
            .unwrap_err()
            .to_string()
            .contains("could not start Git")
    );
    assert!(!root.path().join("argv").exists());
}

#[tokio::test]
async fn cancellation_refuses_discovery_without_returning_a_remote_candidate() {
    let (root, context) = controlled_git(b"", 0);
    context.cancellation.cancel();
    let error = discover(&context).await.unwrap_err().to_string();
    assert!(error.contains("cancelled"));
    // Cancellation returns no candidate. Verify that the helper's only local
    // observation file is stable after the bounded kill/wait path returns.
    let evidence = std::fs::read(root.path().join("argv")).unwrap_or_default();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        std::fs::read(root.path().join("argv")).unwrap_or_default(),
        evidence
    );
}

#[tokio::test]
async fn command_denial_prevents_local_git_process_creation() {
    let (root, mut context) = controlled_git(b"", 0);
    let config = crate::Config {
        access: Some(crate::config::AccessMode::ReadOnly),
        ..Default::default()
    };
    context.policy = Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    let error = discover(&context).await.unwrap_err().to_string();
    assert!(error.contains("denied by command policy"));
    assert!(!root.path().join("argv").exists());
}
