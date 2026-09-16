use super::*;

fn fixture(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    root
}

#[test]
fn manifest_shapes_never_claim_execution() {
    for (name, body, ecosystem, count) in [
        ("Cargo.toml", "[package]\nname='fixture'", "Rust", 3),
        ("Cargo.toml", "[workspace]\nmembers=[]", "Rust", 3),
        (
            "package.json",
            r#"{"packageManager":"pnpm@9","scripts":{"build":"ignored","test":"ignored","lint":"ignored","typecheck":"ignored"}}"#,
            "JavaScript/TypeScript",
            4,
        ),
        (
            "package.json",
            r#"{"scripts":{"test":42}}"#,
            "JavaScript/TypeScript",
            0,
        ),
        ("pyproject.toml", "[tool.pytest]\n[tool.ruff]", "Python", 2),
        ("pyproject.toml", "[build-system]", "Python", 0),
        ("go.mod", "module example.invalid/fixture", "Go", 2),
    ] {
        let project = parse_project(name, body, name, ".").unwrap();
        assert_eq!(project.ecosystem, ecosystem);
        assert_eq!(project.commands.len(), count);
        assert!(
            project
                .commands
                .iter()
                .all(|c| !c.verified && c.evidence == name)
        );
    }
    for (name, body) in [
        ("Cargo.toml", "bad ["),
        ("Cargo.toml", "name='x'"),
        ("package.json", "[]"),
        ("package.json", "{"),
        ("pyproject.toml", "bad ["),
        ("pyproject.toml", "name='x'"),
        ("go.mod", "go 1.24"),
    ] {
        assert!(
            parse_project(name, body, name, ".").is_err(),
            "{name}: {body}"
        );
    }
    assert!(safe_name("portable_name-1.toml"));
    for name in ["", "../x", "has space", "é", "a\n"] {
        assert!(!safe_name(name));
    }
    assert!(!safe_name(&"a".repeat(129)));
}

#[test]
fn discovery_is_sorted_bounded_and_does_not_copy_script_bodies() {
    let root = fixture(&[
        ("Cargo.toml", b"[workspace]\nmembers=[]"),
        (
            "web/package.json",
            br#"{"scripts":{"test":"SECRET_SCRIPT_BODY"}}"#,
        ),
        ("py/pyproject.toml", b"[tool.pytest]"),
        ("go/go.mod", b"module fixture.invalid/go"),
        ("bad/package.json", b"not-json"),
        ("invalid/Cargo.toml", &[0xff]),
        ("target/Cargo.toml", b"[package]"),
        (".git/Cargo.toml", b"[package]"),
        ("one/two/three/Cargo.toml", b"[package]"),
        ("unsafe name", b"ignored"),
    ]);
    let report = inspect(root.path()).unwrap();
    assert_eq!(report.projects.len(), 4);
    assert!(
        report
            .evidence
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    );
    assert!(report.evidence.iter().all(|e| e.sha256.len() == 64));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("unsafe filename"))
    );
    assert!(report.warnings.iter().any(|w| w.contains("Malformed")));
    let draft = render(&report);
    assert!(draft.contains("unverified"));
    assert!(!draft.contains("SECRET_SCRIPT_BODY"));
    assert!(!draft.contains("target/Cargo.toml"));
    assert!(!draft.contains("three/Cargo.toml"));
    assert_eq!(report, inspect(root.path()).unwrap());
}

#[test]
fn guidance_filters_to_supported_commands_and_retains_evidence() {
    let root = fixture(&[
        ("Cargo.toml", b"[package]\nname='fixture'"),
        ("AGENTS.md", b"Run `cargo test`.\n```sh\ncargo build\n```"),
        ("agents.md", b"never cargo test"),
        ("README.md", b"cargo test"),
    ]);
    let report = inspect(root.path()).unwrap();
    let commands = &report.projects[0].commands;
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].argv, ["cargo", "build"]);
    assert_eq!(commands[1].argv, ["cargo", "test"]);
    assert!(commands[1].evidence.contains("README.md"));
    assert!(commands.iter().all(|c| !c.verified));
    assert!(report.warnings.iter().any(|w| w.contains("shadowed")));
}

#[test]
fn uncertain_guidance_omits_commands_instead_of_guessing() {
    for guidance in [
        b"never cargo test".as_slice(),
        b"> cargo test",
        b"```python\ncargo test\n```",
        b"```sh\ncargo test",
        b"<!-- cargo test -->",
        b"cargo test && echo x",
        b"cargo check",
        b"Use cargo for checks",
        &[0xff],
        b"cargo test\x1b",
    ] {
        let root = fixture(&[("Cargo.toml", b"[package]"), ("AGENTS.md", guidance)]);
        let report = inspect(root.path()).unwrap();
        assert!(
            report.projects[0].commands.is_empty(),
            "guidance {guidance:?}"
        );
        assert!(render(&report).contains("No command inferred"));
    }
    for guidance in [
        "cargo test\n".repeat(513),
        "x".repeat(4097),
        "cargo test\n".repeat(65),
    ] {
        let root = fixture(&[
            ("Cargo.toml", b"[package]"),
            ("README.md", guidance.as_bytes()),
        ]);
        assert!(
            inspect(root.path()).unwrap().projects[0]
                .commands
                .is_empty()
        );
    }
}

#[test]
fn package_manager_conflicts_and_undeclared_scripts_are_not_recommended() {
    for guidance in [
        "npm run test",
        "pnpm run missing",
        "pnpm run test\nyarn run test",
        "pnpm run --test",
    ] {
        let root = fixture(&[
            (
                "package.json",
                br#"{"packageManager":"pnpm@9","scripts":{"test":"ignored"}}"#,
            ),
            ("README.md", guidance.as_bytes()),
        ]);
        assert!(
            inspect(root.path()).unwrap().projects[0]
                .commands
                .is_empty()
        );
    }
    let root = fixture(&[
        ("package.json", br#"{"scripts":{"check:all":"ignored"}}"#),
        ("README.md", b"pnpm run check:all"),
    ]);
    assert_eq!(
        inspect(root.path()).unwrap().projects[0].commands[0].argv,
        ["pnpm", "run", "check:all"]
    );
}

#[test]
fn empty_and_oversized_evidence_are_explicit() {
    let root = fixture(&[]);
    assert!(
        inspect(root.path())
            .unwrap()
            .warnings
            .iter()
            .any(|w| w.contains("No supported manifest"))
    );
    std::fs::write(root.path().join("AGENTS.md"), vec![b'a'; MAX_FILE + 1]).unwrap();
    std::fs::write(root.path().join("Cargo.toml"), b"[package]").unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(report.projects[0].commands.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Could not safely read"))
    );
}

#[cfg(unix)]
#[test]
fn symlinked_evidence_is_not_followed() {
    let root = fixture(&[("source", b"[package]")]);
    std::os::unix::fs::symlink(root.path().join("source"), root.path().join("Cargo.toml")).unwrap();
    let report = inspect(root.path()).unwrap();
    assert!(report.projects.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Could not safely read"))
    );
}
