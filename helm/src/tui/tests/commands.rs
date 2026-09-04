use super::*;

#[tokio::test]
async fn startup_only_controls_create_explicit_session_preserving_handoffs() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));

    let mut access = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    let access_id = access.session.id.to_string();
    handle_command("/access unrestricted", &mut access, &store, None, None)
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
    handle_command("/set max_tokens 32", &mut setting, &store, None, None)
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
    handle_command("/doctor", &mut doctor, &store, None, None)
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
    let store = SessionStore::new(directory.path().join("sessions"));
    let mut session = Session::new(directory.path().into(), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "keep me"));
    let mut app = App::new(session, Vec::new());
    handle_command("/clear", &mut app, &store, None, None)
        .await
        .unwrap();
    assert_eq!(app.session.messages.len(), 1);
    handle_command("/clear confirm", &mut app, &store, None, None)
        .await
        .unwrap();
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn new_command_starts_named_empty_session() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));
    let mut session = Session::new(directory.path().into(), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "old conversation"));
    let old_id = session.id;
    let mut app = App::new(session, Vec::new());

    handle_command("/new field work", &mut app, &store, None, None)
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
    let store = SessionStore::new(directory.path().join("sessions"));
    let session = Session::new(directory.path().into(), "test-model".into());
    let mut app = App::new(session, Vec::new());
    assert!(
        handle_command("/name field work", &mut app, &store, None, None)
            .await
            .unwrap()
    );
    assert_eq!(app.session.name.as_deref(), Some("field work"));
    let export = directory.path().join("export.md");
    handle_command(
        &format!("/export {}", export.display()),
        &mut app,
        &store,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(export.exists());
}
