use super::*;

#[tokio::test]
async fn startup_only_controls_create_explicit_session_preserving_handoffs() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = SessionStore::new(directory.path().join("sessions"));

    let mut access = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    let access_id = access.session.id.to_string();
    handle_command("/access unrestricted", &mut access, &mut store, None, None)
        .await
        .unwrap();
    assert_eq!(
        access.exit,
        Some(TuiExit::Launch(CliRequest {
            arguments: vec![
                "--access".into(),
                "unrestricted".into(),
                "chat".into(),
                "--resume".into(),
                access_id,
            ],
            use_active_config: true,
            resume_after: false,
            verbose: None,
            log_format: None,
        }))
    );

    let mut setting = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    handle_command("/set max_tokens 32", &mut setting, &mut store, None, None)
        .await
        .unwrap();
    let Some(TuiExit::Launch(request)) = setting.exit else {
        panic!("expected CLI handoff");
    };
    assert_eq!(request.arguments[0..2], ["--set", "max_tokens=32"]);
    assert!(request.use_active_config);

    let mut doctor = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    handle_command("/doctor", &mut doctor, &mut store, None, None)
        .await
        .unwrap();
    let Some(TuiExit::Launch(request)) = doctor.exit else {
        panic!("expected CLI handoff");
    };
    assert_eq!(request.arguments, ["doctor"]);
    assert!(request.resume_after);
}

#[tokio::test]
async fn clear_requires_explicit_confirmation() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let mut session = Session::new(directory.path().into(), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "keep me"));
    let mut app = App::new(session, Vec::new());
    handle_command("/clear", &mut app, &mut store, None, None)
        .await
        .unwrap();
    assert_eq!(app.session.messages.len(), 1);
    handle_command("/clear confirm", &mut app, &mut store, None, None)
        .await
        .unwrap();
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn new_command_starts_named_empty_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let mut session = Session::new(directory.path().into(), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "old conversation"));
    let old_id = session.id;
    let mut app = App::new(session, Vec::new());

    handle_command("/new field work", &mut app, &mut store, None, None)
        .await
        .unwrap();

    assert_ne!(app.session.id, old_id);
    assert_eq!(app.session.name.as_deref(), Some("field work"));
    assert!(app.session.messages.is_empty());
    assert_eq!(app.session.model, "test-model");
    assert_eq!(app.session.workspace, directory.path());
}

#[tokio::test]
async fn local_commands_name_and_export_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let session = Session::new(directory.path().into(), "test-model".into());
    let mut app = App::new(session, Vec::new());
    assert!(
        handle_command("/name field work", &mut app, &mut store, None, None)
            .await
            .unwrap()
    );
    assert_eq!(app.session.name.as_deref(), Some("field work"));
    let export = directory.path().join("export.md");
    handle_command(
        &format!("/export {}", export.display()),
        &mut app,
        &mut store,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(export.exists());
}

#[tokio::test]
async fn retired_voyage_command_cannot_launch_or_mutate_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SessionStore::new(dir.path().join("sessions"));
    for command in [
        "/voyage",
        "/voyage http://127.0.0.1:9480 workstation",
        "/attach",
    ] {
        let mut session = Session::new(dir.path().into(), "test-model".into());
        session
            .messages
            .push(crate::Message::new(Role::User, "keep me"));
        let id = session.id;
        let mut app = App::new(session, Vec::new());
        handle_command(command, &mut app, &mut store, None, None)
            .await
            .unwrap();
        assert!(app.exit.is_none());
        assert!(app.status.starts_with("Unknown or incomplete command:"));
        assert_eq!(app.session.id, id);
        assert_eq!(app.session.messages.len(), 1);
        assert!(!dir.path().join("sessions").exists());
    }
    assert!(
        !SLASH_COMMANDS
            .iter()
            .any(|command| command.name == "voyage" || command.name == "attach")
    );
}

#[tokio::test]
async fn new_and_branch_commands_transfer_session_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let unbound = SessionStore::new(directory.path().join("sessions"));
    let mut original = Session::new(directory.path().into(), "test".into());
    let mut store = unbound.with_execution(original.id).await.unwrap();
    store.save(&mut original).await.unwrap();
    let original_id = original.id;
    let mut app = App::new(original, vec![]);
    handle_command("/new owned-new", &mut app, &mut store, None, None)
        .await
        .unwrap();
    let new_id = app.session.id;
    assert_ne!(new_id, original_id);
    assert_eq!(store.owned_session_id(), Some(new_id));
    unbound.with_execution(original_id).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), unbound.with_execution(new_id))
            .await
            .is_err()
    );
    handle_command("/name owned-new", &mut app, &mut store, None, None)
        .await
        .unwrap();
    handle_command("/branch owned-branch", &mut app, &mut store, None, None)
        .await
        .unwrap();
    let branch_id = app.session.id;
    assert_ne!(branch_id, new_id);
    assert_eq!(app.session.parent_id, Some(new_id));
    assert_eq!(store.owned_session_id(), Some(branch_id));
    unbound.with_execution(new_id).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), unbound.with_execution(branch_id))
            .await
            .is_err()
    );
    handle_command("/clear confirm", &mut app, &mut store, None, None)
        .await
        .unwrap();
    assert_eq!(store.owned_session_id(), Some(branch_id));
}
