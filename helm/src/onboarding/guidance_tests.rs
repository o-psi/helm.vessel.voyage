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
