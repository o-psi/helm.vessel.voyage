use super::*;

#[test]
fn launcher_is_private_create_only_and_rejects_nonloopback_or_credential_urls() {
    let temp = tempfile::tempdir().unwrap();
    let path = launcher(temp.path(), "http://127.0.0.1:23456/#fixture&token").unwrap();
    assert_eq!(path, temp.path().join("open.html"));
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("no-referrer"));
    assert!(text.contains("fixture&amp;token"));
    assert!(launcher(temp.path(), "http://127.0.0.1:23456/#replacement").is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    for url in [
        "invalid",
        "https://127.0.0.1:23456/#token",
        "http://localhost:23456/#token",
        "http://example.invalid:23456/#token",
        "http://127.0.0.1/#token",
        "http://127.0.0.1:23456/",
        "http://127.0.0.1:23456/path#token",
        "http://127.0.0.1:23456/?q=x#token",
        "http://user:secret@127.0.0.1:23456/#token",
    ] {
        assert!(launcher(temp.path(), url).is_err(), "{url}");
    }
}

#[tokio::test]
async fn synthetic_handle_controls_cancel_and_join_without_browser_processes() {
    let (state_tx, state) = watch::channel(Status::default());
    let (control, mut input) = mpsc::channel(1);
    let stop = CancellationToken::new();
    let observed = stop.clone();
    let job = tokio::spawn(async move {
        observed.cancelled().await;
        Ok(())
    });
    let mut handle = Handle {
        state,
        control,
        stop: stop.clone(),
        job: Some(job),
    };
    assert!(!handle.finished());
    handle.control(Control::Human).unwrap();
    assert!(matches!(input.recv().await, Some(Control::Human)));
    handle.control(Control::Private).unwrap();
    assert!(matches!(input.recv().await, Some(Control::Private)));
    assert!(!state_tx.borrow().finished);
    handle.finish().await.unwrap();
    assert!(stop.is_cancelled());
    assert!(handle.finished());
    handle.finish().await.unwrap();
    drop(handle);
}

#[tokio::test]
async fn handle_reports_failed_cleanup_and_drop_only_cancels() {
    let (_state_tx, state) = watch::channel(Status::default());
    let (control, _input) = mpsc::channel(1);
    let stop = CancellationToken::new();
    let mut handle = Handle {
        state,
        control,
        stop: stop.clone(),
        job: Some(tokio::spawn(async {
            anyhow::bail!("fixture cleanup unresolved")
        })),
    };
    assert!(
        handle
            .finish()
            .await
            .unwrap_err()
            .to_string()
            .contains("unresolved")
    );
    assert!(stop.is_cancelled());
    assert!(handle.job.is_none());
    let (_state_tx, state) = watch::channel(Status::default());
    let (control, _input) = mpsc::channel(1);
    let stop = CancellationToken::new();
    let handle = Handle {
        state,
        control,
        stop: stop.clone(),
        job: None,
    };
    drop(handle);
    assert!(stop.is_cancelled());
    assert!(open_launcher(PathBuf::from("relative.html")).await.is_err());
}
