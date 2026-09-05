use super::*;

#[test]
fn saved_workflows_have_a_namespaced_palette_command() {
    let mut app = App::new(
        Session::new(PathBuf::from("/tmp"), "test-model".into()),
        Vec::new(),
    );
    app.composer.insert_str("/workf");
    let matches = slash_palette_matches(&app.palette_context());
    assert_eq!(
        matches
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["workflow"]
    );
    assert!(crate::workflow::RESERVED.contains(&"workflow"));
    assert!(matches[0].description.contains("workflow"));
}

#[test]
fn slash_palette_lists_all_commands_above_the_composer_and_filters() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let mut app = App::new(session, Vec::new());
    app.composer.insert('/');
    assert_eq!(
        slash_palette_matches(&app.palette_context()).len(),
        SLASH_COMMANDS.len()
    );

    let backend = ratatui::backend::TestBackend::new(140, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let snapshot = rows.join("\n");
    assert!(snapshot.contains("Commands"));
    for command in SLASH_COMMANDS {
        assert!(
            snapshot.contains(command.usage),
            "palette omitted {}",
            command.usage
        );
    }
    let palette_row = rows
        .iter()
        .position(|row| row.contains("Commands"))
        .unwrap();
    assert!(palette_row < 24);

    app.composer.text = "/co".into();
    app.composer.cursor = app.composer.text.len();
    let matches = slash_palette_matches(&app.palette_context());
    assert_eq!(
        matches
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>(),
        ["config", "compact", "completions"]
    );
}

#[test]
fn slash_palette_offers_contextual_values_and_dynamic_records() {
    let mut first = Session::new(PathBuf::from("/tmp"), "test-model".into());
    first.name = Some("deployment notes".into());
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let mut app = App::new(session, vec![first.clone()]);

    app.composer.insert_str("/access ");
    assert_eq!(
        slash_palette_items(&app.palette_context())
            .iter()
            .map(|item| item.usage.as_str())
            .collect::<Vec<_>>(),
        ["read-only", "approval", "unrestricted"]
    );
    app.palette.selected_slash_command = 1;
    complete_selected_slash_command(&mut app);
    assert_eq!(app.composer.text, "/access approval");
    assert!(slash_input_is_complete(&app.palette_context()));

    app.model_panel.models = vec![ModelInfo::minimal("gpt-dynamic")];
    app.composer = Composer::default();
    app.composer.insert_str("/models dynamic");
    let models = slash_palette_items(&app.palette_context());
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].completion, "/model gpt-dynamic");

    app.composer = Composer::default();
    app.composer.insert_str("/resume deploy");
    let sessions = slash_palette_items(&app.palette_context());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].completion, format!("/resume {}", first.id));

    app.composer = Composer::default();
    app.composer.insert_str("/set ");
    assert_eq!(
        slash_palette_items(&app.palette_context()).len(),
        CONFIG_OVERRIDE_SPECS.len()
    );
    assert!(
        slash_palette_items(&app.palette_context())
            .iter()
            .any(|item| item.usage == "codex_command")
    );

    app.composer = Composer::default();
    app.composer.insert_str("/set access ");
    assert_eq!(
        slash_palette_items(&app.palette_context())
            .iter()
            .map(|item| item.usage.as_str())
            .collect::<Vec<_>>(),
        ["read-only", "approval", "unrestricted"]
    );

    for input in ["/name ", "/branch ", "/run "] {
        app.composer = Composer::default();
        app.composer.insert_str(input);
        assert!(
            !slash_palette_items(&app.palette_context()).is_empty(),
            "{input} should explain or suggest its next argument"
        );
    }

    app.composer = Composer::default();
    app.composer.insert_str("/set provider_retry_attempts ");
    assert!(
        slash_palette_items(&app.palette_context())
            .iter()
            .any(|item| item.usage == "0")
    );

    app.composer = Composer::default();
    app.composer.insert_str("/provider codex-");
    assert!(
        slash_palette_items(&app.palette_context())
            .iter()
            .any(|item| item.usage == "codex-subscription")
    );

    app.composer = Composer::default();
    app.composer.insert_str("/run --no-save ");
    assert_eq!(
        slash_palette_items(&app.palette_context())[0].usage,
        "<PROMPT>"
    );
}

#[test]
fn slash_palette_completes_filesystem_arguments() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("project-alpha")).unwrap();
    std::fs::write(directory.path().join("helm-special.toml"), "model = 'test'").unwrap();
    let session = Session::new(directory.path().into(), "test-model".into());
    let mut app = App::new(session, Vec::new());

    app.composer.insert_str("/workspace pro");
    let workspace = slash_palette_items(&app.palette_context());
    assert_eq!(workspace.len(), 1);
    assert_eq!(
        workspace[0].completion,
        format!("/workspace project-alpha{}", std::path::MAIN_SEPARATOR)
    );

    app.composer = Composer::default();
    app.composer.insert_str("/config helm-");
    let config = slash_palette_items(&app.palette_context());
    assert_eq!(config.len(), 1);
    assert_eq!(config[0].completion, "/config helm-special.toml");
}

#[test]
fn slash_palette_completion_and_dismissal_preserve_composer_input() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let mut app = App::new(session, Vec::new());
    app.composer.insert_str("/na");
    complete_selected_slash_command(&mut app);
    assert_eq!(app.composer.text, "/name ");
    assert_eq!(app.composer.cursor, app.composer.text.len());
    assert!(slash_palette_matches(&app.palette_context()).is_empty());

    app.composer = Composer::default();
    app.composer.insert_str("/help");
    assert!(slash_input_is_complete(&app.palette_context()));
    app.palette.slash_palette_dismissed = true;
    assert!(slash_palette_matches(&app.palette_context()).is_empty());
    assert_eq!(app.composer.text, "/help");
    reset_slash_palette(&mut app);
    assert_eq!(slash_palette_matches(&app.palette_context()).len(), 1);
}
