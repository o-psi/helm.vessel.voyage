use super::*;
use std::fs;

fn node(root: &Path, manager: Option<&str>) {
    let mut value = serde_json::json!({"scripts":{"test":"echo unsupported; exit 1","test:ci":"echo actual-tests","build":"echo build"}});
    if let Some(manager) = manager {
        value["packageManager"] = manager.into();
    }
    fs::write(root.join("package.json"), value.to_string()).unwrap();
}

#[test]
fn contradictory_existing_guidance_never_produces_the_observed_npm_guess() {
    let root = tempfile::tempdir().unwrap();
    node(root.path(), None);
    let agents =
        "Use pnpm only. Do not use npm in this repository. Run pnpm run test:ci for tests.\n";
    fs::write(root.path().join("AGENTS.md"), agents).unwrap();
    fs::write(
        root.path().join("README.md"),
        "Testing: pnpm run test:ci. npm run test is intentionally unsupported.\n",
    )
    .unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(report.projects[0].commands.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("conflict") && warning.contains("AGENTS.md"))
    );
    assert_eq!(
        fs::read_to_string(root.path().join("AGENTS.md")).unwrap(),
        agents
    );
}

#[test]
fn documented_script_uses_manifest_and_guidance_evidence_without_execution() {
    let root = tempfile::tempdir().unwrap();
    node(root.path(), None);
    fs::write(
        root.path().join("AGENTS.md"),
        "# Tests\n```sh\npnpm run test:ci\n```\n",
    )
    .unwrap();
    let report = inspect(root.path()).unwrap();
    let commands = &report.projects[0].commands;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].argv, ["pnpm", "run", "test:ci"]);
    assert!(
        commands[0].evidence.contains("AGENTS.md") && commands[0].evidence.contains("package.json")
    );
    assert!(commands[0].confidence.contains("documented"));
    assert!(!commands[0].verified);
    assert!(!root.path().join("AGENTS.generated.md").exists());
}

#[test]
fn manifest_disagreement_and_undeclared_or_composed_commands_are_omitted() {
    for (manager, text) in [
        (Some("npm@10"), "pnpm run test:ci\n"),
        (None, "pnpm run missing\n"),
        (None, "pnpm run test:ci && touch SHOULD_NOT_EXIST\n"),
        (None, "Do not run pnpm commands until reviewed.\n"),
    ] {
        let root = tempfile::tempdir().unwrap();
        node(root.path(), manager);
        fs::write(root.path().join("CONTRIBUTING.md"), text).unwrap();
        let report = inspect(root.path()).unwrap();
        assert!(report.projects[0].commands.is_empty(), "{text}");
        assert!(!report.warnings.is_empty());
        assert!(!root.path().join("SHOULD_NOT_EXIST").exists());
        assert!(!render(&report).contains("SHOULD_NOT_EXIST"));
    }
}

#[test]
fn guidance_scope_and_active_spelling_do_not_borrow_sibling_conventions() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        fs::create_dir(root.path().join(name)).unwrap();
        node(&root.path().join(name), None);
    }
    fs::write(root.path().join("a/AGENTS.md"), "pnpm run test:ci\n").unwrap();
    fs::write(root.path().join("a/agents.md"), "npm run test\n").unwrap();
    fs::write(root.path().join("b/README.md"), "yarn run build\n").unwrap();
    let report = inspect(root.path()).unwrap();
    assert_eq!(
        report.projects[0].commands[0].argv,
        ["pnpm", "run", "test:ci"]
    );
    assert_eq!(
        report.projects[1].commands[0].argv,
        ["yarn", "run", "build"]
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("agents.md") && warning.contains("shadow"))
    );
}

#[test]
fn prose_is_manual_review_and_confidence_does_not_hide_default_runner_inference() {
    let root = tempfile::tempdir().unwrap();
    node(root.path(), None);
    fs::write(
        root.path().join("README.md"),
        "Review the security procedures before testing.\n",
    )
    .unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("manual review") && warning.contains("README.md"))
    );
    assert!(
        report.projects[0]
            .commands
            .iter()
            .all(|command| command.confidence.contains("inferred npm"))
    );
}

#[test]
fn unsupported_rust_convention_does_not_advertise_conflicting_default_test() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(root.path().join("AGENTS.md"), "cargo nextest run\n").unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(report.projects[0].commands.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("manual review"))
    );
}

#[test]
fn negative_and_quoted_examples_never_become_positive_recommendations() {
    for guidance in [
        "Do not run `pnpm run test:ci`.\n",
        "Don't use this:\n```sh\npnpm run test:ci\n```\n",
        "Don’t use this:\npnpm run test:ci\n",
        "Incorrect example:\npnpm run test:ci\n",
        "Avoid pnpm commands.\npnpm run test:ci\n",
        "> pnpm run test:ci\n",
        "\"pnpm run test:ci\"\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        node(root.path(), None);
        fs::write(root.path().join("AGENTS.md"), guidance).unwrap();
        let report = inspect(root.path()).unwrap();
        assert!(report.projects[0].commands.is_empty(), "{guidance}");
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("omitted"))
        );
    }
}

#[test]
fn bounded_or_unreadable_guidance_does_not_establish_positive_evidence() {
    for guidance in [
        vec![0xff],
        b"pnpm run test:ci\n\x1b[2J".to_vec(),
        ("\n".repeat(512) + "pnpm run test:ci\n").into_bytes(),
        ("x".repeat(4097) + "\npnpm run test:ci\n").into_bytes(),
        "pnpm run test:ci\n".repeat(65).into_bytes(),
    ] {
        let root = tempfile::tempdir().unwrap();
        node(root.path(), None);
        fs::write(root.path().join("README.md"), guidance).unwrap();
        let report = inspect(root.path()).unwrap();
        assert!(report.projects[0].commands.is_empty());
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("omitted"))
        );
        assert!(!render(&report).contains('\x1b'));
    }
}

#[test]
fn ancestor_and_nested_disagreement_is_explicit_without_rewriting_either() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("web")).unwrap();
    node(&root.path().join("web"), None);
    fs::write(root.path().join("AGENTS.md"), "pnpm run test:ci\n").unwrap();
    fs::write(
        root.path().join("web/CONTRIBUTING.md"),
        "yarn run test:ci\n",
    )
    .unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(report.projects[0].commands.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("conflict")
                && warning.contains("web/CONTRIBUTING.md")
                && warning.contains("AGENTS.md"))
    );
    assert_eq!(
        fs::read_to_string(root.path().join("AGENTS.md")).unwrap(),
        "pnpm run test:ci\n"
    );
}

#[test]
fn exact_supported_non_node_shapes_keep_argument_bytes_and_evidence() {
    for (manifest, contents, command) in [
        ("Cargo.toml", "[workspace]\nmembers=[]\n", "cargo test"),
        ("pyproject.toml", "[tool.ruff]\n", "python3 -m ruff check ."),
        ("go.mod", "module fixture.test/project\n", "go test ./..."),
    ] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(manifest), contents).unwrap();
        fs::write(root.path().join("README.md"), format!("Run `{command}`.\n")).unwrap();
        let report = inspect(root.path()).unwrap();
        assert_eq!(report.projects[0].commands.len(), 1);
        assert_eq!(
            report.projects[0].commands[0].argv,
            command.split_whitespace().collect::<Vec<_>>()
        );
        assert!(!report.projects[0].commands[0].verified);
        assert!(
            report.projects[0].commands[0]
                .evidence
                .contains("README.md")
        );
    }
}

#[test]
fn guidance_cannot_introduce_option_shaped_or_expanding_script_arguments() {
    for script in ["--help", "$(touch_CANARY)", "test;echo_CANARY"] {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("package.json"),
            serde_json::json!({"scripts":{script:"touch CANARY"}}).to_string(),
        )
        .unwrap();
        fs::write(
            root.path().join("AGENTS.md"),
            format!("pnpm run {script}\n"),
        )
        .unwrap();
        let report = inspect(root.path()).unwrap();
        assert!(report.projects[0].commands.is_empty());
        assert!(!render(&report).contains("CANARY"));
        assert!(!root.path().join("CANARY").exists());
    }
}
