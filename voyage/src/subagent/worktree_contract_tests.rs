use super::*;
#[test]
fn planned_git_arguments_are_single_components_and_never_shell_fragments() {
    let manager = WorktreeManager {
        repository: PathBuf::from("/unused"),
        root: PathBuf::from("/offline root"),
        managed: None,
        legacy_root: None,
        environment: None,
        policy: None,
    };
    for name in [
        "",
        "-option",
        "../escape",
        "nested/name",
        "space name",
        "x;touch",
        "x\ny",
        "☃",
    ] {
        assert!(manager.planned_path(name).is_err(), "{name}");
    }
    for name in ["safe", "safe-2", "safe_3"] {
        assert_eq!(
            manager.planned_path(name).unwrap(),
            PathBuf::from("/offline root").join(name)
        );
        let command = manager.create_command(name, "main").unwrap();
        let args = shell_words::split(&command).unwrap();
        assert_eq!(
            args,
            vec![
                "git".to_owned(),
                "worktree".into(),
                "add".into(),
                "-b".into(),
                format!("agents/{name}"),
                format!("/offline root/{name}"),
                "main".into()
            ]
        );
    }
    for reference in ["", "-b", "HEAD~1", "main;echo", "../bad"] {
        assert!(manager.create_command("safe", reference).is_err());
    }
    assert!(manager.create_command("safe", "release/v1").is_ok());
}
