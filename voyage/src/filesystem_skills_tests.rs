use super::*;

fn policy(root: &Path) -> Policy {
    Policy::new(&crate::Config::default(), root.to_path_buf()).unwrap()
}
fn skill(root: &Path, folder: &str, name: &str) -> PathBuf {
    let dir = root.join(".agents/skills").join(folder);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("SKILL.md");
    fs::write(
        &path,
        format!("---\nname: {name}\ndescription: Helpful fixture\n---\nPRIVATE BODY MARKER\n"),
    )
    .unwrap();
    path
}

#[test]
fn defaults_missing_and_no_loader_are_empty() {
    let root = tempfile::tempdir().unwrap();
    let policy = policy(root.path());
    assert!(discover(&policy, None).guidance().is_empty());
    skill(root.path(), "example", "example");
    assert!(guidance(&policy, false).is_empty());
}

#[test]
fn metadata_only_deterministic_deduplicated_and_refreshed() {
    let root = tempfile::tempdir().unwrap();
    let path = skill(root.path(), "z", "z");
    skill(root.path(), "a", "a");
    let policy = policy(root.path());
    let catalog = discover(&policy, Some(root.path()));
    assert_eq!(catalog.skills.len(), 2);
    assert_eq!(catalog.skills[0].name, "a");
    assert!(catalog.diagnostics.is_empty());
    let guidance = catalog.guidance();
    assert!(guidance.contains("read_file"));
    assert!(guidance.contains("Helpful fixture"));
    assert!(!guidance.contains("PRIVATE BODY MARKER"));
    fs::remove_file(path).unwrap();
    let catalog = discover(&policy, None);
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.diagnostics.len(), 1);
}

#[test]
fn duplicate_names_are_explicit_and_metadata_is_json_escaped() {
    let root = tempfile::tempdir().unwrap();
    skill(root.path(), "a", "example");
    let path = skill(root.path(), "b", "example");
    fs::write(
        path,
        "---\nname: example\ndescription: \"Helpful\\n## Forged heading\"\n---\nbody",
    )
    .unwrap();
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 2);
    assert!(catalog.diagnostics[0].contains("duplicate"));
    let text = catalog.guidance();
    assert!(text.contains("Helpful\\n## Forged heading"));
    assert!(!text.contains("Helpful\n## Forged heading"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_followed_only_within_roots_and_aliases_deduplicated() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = skill(root.path(), "actual", "example");
    let outside_path = skill(outside.path(), "outside", "outside");
    let skills = root.path().join(".agents/skills");
    symlink(path.parent().unwrap(), skills.join("alias")).unwrap();
    symlink(outside_path.parent().unwrap(), skills.join("denied")).unwrap();
    symlink(skills.join("absent"), skills.join("broken")).unwrap();
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.diagnostics.len(), 2);
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|d| d.contains("outside allowed roots"))
    );
    assert!(!catalog.guidance().contains("PRIVATE BODY MARKER"));
}

#[test]
fn invalid_missing_oversized_and_non_utf8_candidates_are_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let invalid = skill(root.path(), "invalid", "invalid");
    fs::write(invalid, "not a skill").unwrap();
    let binary = skill(root.path(), "binary", "binary");
    fs::write(binary, [0xff]).unwrap();
    let huge = skill(root.path(), "huge", "huge");
    fs::write(huge, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
    fs::create_dir_all(root.path().join(".agents/skills/missing")).unwrap();
    fs::write(root.path().join(".agents/skills/ignored.md"), "ignored").unwrap();
    let catalog = discover(&policy(root.path()), None);
    assert!(catalog.skills.is_empty());
    assert_eq!(catalog.diagnostics.len(), 4);
}

#[test]
fn entry_overflow_refuses_root_instead_of_nondeterministic_subset() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..=MAX_ENTRIES {
        skill(root.path(), &format!("s{i}"), "example");
    }
    let catalog = discover(&policy(root.path()), None);
    assert!(catalog.skills.is_empty());
    assert!(catalog.diagnostics[0].contains("entry limit"));
}

#[test]
fn aggregate_bytes_and_prompt_output_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..6 {
        let path = skill(root.path(), &format!("s{i}"), &format!("s{i}"));
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(&"x".repeat(MAX_FILE_BYTES as usize - text.len()));
        fs::write(path, text).unwrap();
    }
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 4);
    assert!(catalog.diagnostics[0].contains("byte limit"));
    let mut large = Catalog::default();
    for _ in 0..128 {
        large.diagnostics.push("x".repeat(2048));
    }
    let text = large.guidance();
    assert!(text.len() < MAX_GUIDANCE_BYTES + 2048);
    assert!(text.contains("additional entries omitted"));
}

#[test]
fn denied_home_root_and_non_directory_root_are_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(outside.path(), "outside", "outside");
    let catalog = discover(&policy(root.path()), Some(outside.path()));
    assert!(catalog.skills.is_empty());
    assert!(catalog.diagnostics[0].contains("outside allowed roots"));
    fs::create_dir_all(root.path().join(".agents")).unwrap();
    fs::write(root.path().join(".agents/skills"), "not a directory").unwrap();
    assert_eq!(discover(&policy(root.path()), None).diagnostics.len(), 1);
}

#[test]
fn nonregular_skill_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("directory");
    fs::create_dir(&path).unwrap();
    assert!(
        read_skill(&path, MAX_FILE_BYTES)
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
}

#[tokio::test]
async fn executing_agent_advertises_then_reads_without_persisting_catalog() {
    use crate::{
        agent::{Agent, SilentSink},
        model::{Message, ModelRequest, ModelResponse, Role, ToolCall},
        provider::{Provider, ProviderError},
        tools::ToolRegistry,
    };
    use std::sync::{Arc, Mutex};
    struct Script {
        seen: Arc<Mutex<Vec<ModelRequest>>>,
        path: PathBuf,
    }
    #[async_trait::async_trait]
    impl Provider for Script {
        async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
            let mut seen = self.seen.lock().unwrap();
            let first = seen.is_empty();
            seen.push(request);
            let mut message = Message::new(Role::Assistant, if first { "" } else { "loaded" });
            if first {
                message.tool_calls.push(ToolCall {
                    id: "load-skill".into(),
                    name: "read_file".into(),
                    arguments: serde_json::json!({"path": self.path}),
                });
            }
            Ok(ModelResponse {
                message,
                usage: Default::default(),
                service_tier: None,
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let path = skill(root.path(), "example", "example");
    let context = crate::tools::reliability_tests::context(root.path());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::default();
    tools.register(crate::tools::ReadFile);
    let agent = Agent::new(
        Box::new(Script {
            seen: seen.clone(),
            path,
        }),
        tools,
        context,
        Arc::new(SilentSink),
        "fixture".into(),
        "fixture instructions".into(),
        1024,
        None,
    );
    let outcome = agent
        .run(vec![], "load example skill".into())
        .await
        .unwrap();
    assert_eq!(outcome.answer, "loaded");
    let seen = seen.lock().unwrap();
    let first = serde_json::to_string(&seen[0]).unwrap();
    assert!(first.contains("Available filesystem skills"));
    assert!(first.contains("Helpful fixture"));
    assert!(!first.contains("PRIVATE BODY MARKER"));
    assert!(
        seen[1]
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.content.contains("PRIVATE BODY MARKER"))
    );
    assert!(
        !outcome
            .messages
            .iter()
            .any(|m| m.content.contains("Available filesystem skills"))
    );
}

#[test]
fn nested_projects_have_explicit_scope_and_do_not_load_bodies() {
    let root = tempfile::tempdir().unwrap();
    skill(root.path(), "global", "shared");
    let web = root.path().join("web");
    let nested = web.join("packages/ui");
    skill(&web, "boost", "shared");
    skill(&nested, "ui", "ui");
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 3);
    assert_eq!(catalog.skills[0].scope.as_deref(), Some(root.path()));
    assert_eq!(catalog.skills[1].scope.as_deref(), Some(web.as_path()));
    assert_eq!(catalog.skills[2].scope.as_deref(), Some(nested.as_path()));
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|d| d.contains("duplicate skill name"))
    );
    let guidance = catalog.guidance();
    assert!(guidance.contains("only for work in that subtree"));
    assert!(!guidance.contains("PRIVATE BODY MARKER"));
    // A new run observes scopes added since the previous scan.
    skill(&root.path().join("api"), "api", "api");
    assert_eq!(discover(&policy(root.path()), None).skills.len(), 4);
}

#[test]
fn project_walk_skips_dependencies_hidden_dirs_and_arbitrary_skill_files() {
    let root = tempfile::tempdir().unwrap();
    for excluded in [
        "vendor",
        "node_modules",
        "target",
        "dist",
        "build",
        "coverage",
        "__pycache__",
        ".git",
        ".worktrees",
    ] {
        skill(
            &root.path().join(excluded).join("nested"),
            "hidden",
            "hidden",
        );
    }
    fs::write(root.path().join("SKILL.md"), "not a skill root").unwrap();
    skill(&root.path().join("web"), "visible", "visible");
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].name, "visible");
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn project_walk_bounds_report_incomplete_discovery() {
    let root = tempfile::tempdir().unwrap();
    skill(root.path(), "root", "root");
    for i in 0..MAX_PROJECT_ENTRIES {
        fs::write(root.path().join(format!("file{i}")), "").unwrap();
    }
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 1);
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|d| d.contains("project discovery entry limit"))
    );
    let root = tempfile::tempdir().unwrap();
    let deepest =
        (0..MAX_PROJECT_DEPTH + 1).fold(root.path().to_path_buf(), |p, _| p.join("nested"));
    skill(&deepest, "too-deep", "too-deep");
    let catalog = discover(&policy(root.path()), None);
    assert!(catalog.skills.is_empty());
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|d| d.contains("depth limit"))
    );
}

#[cfg(unix)]
#[test]
fn project_walk_does_not_follow_directory_aliases_or_cycles() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    skill(outside.path(), "outside", "outside");
    skill(&root.path().join("web"), "inside", "inside");
    symlink(outside.path(), root.path().join("outside")).unwrap();
    symlink(root.path().join("web"), root.path().join("alias")).unwrap();
    symlink(root.path(), root.path().join("web/cycle")).unwrap();
    let catalog = discover(&policy(root.path()), None);
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].scope, Some(root.path().join("web")));
    assert!(catalog.diagnostics.is_empty());
}

#[test]
fn user_skills_have_no_project_scope() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join(".home");
    skill(&home, "user", "user");
    let catalog = discover(&policy(root.path()), Some(&home));
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].scope, None);
}

#[test]
fn boost_metadata_mapping_is_discovered_without_authority() {
    let root = tempfile::tempdir().unwrap();
    let web = root.path().join("web");
    let path = skill(&web, "boost", "boost");
    fs::write(path, "---\nname: boost\ndescription: Laravel fixture\nlicense: MIT\nmetadata:\n  author: laravel\n  name: not-the-skill-name\n---\nPRIVATE BODY MARKER\n").unwrap();
    fs::write(web.join("artisan"), "fixture").unwrap();
    skill(
        &web.join("storage/framework/views"),
        "generated",
        "generated",
    );
    let catalog = discover(&policy(root.path()), None);
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].name, "boost");
    assert_eq!(catalog.skills[0].scope.as_deref(), Some(web.as_path()));
}

#[test]
#[ignore = "manual executing-checkout verification, no provider calls"]
fn actual_boost_checkout_skills_are_discovered() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let catalog = discover(&policy(&root), None);
    let web = root.join("web");
    let skills: Vec<_> = catalog
        .skills
        .iter()
        .filter(|s| s.scope.as_ref() == Some(&web))
        .collect();
    assert_eq!(skills.len(), 7, "diagnostics: {:?}", catalog.diagnostics);
    assert!(
        !catalog
            .diagnostics
            .iter()
            .any(|d| d.starts_with(&web.display().to_string())),
        "{:?}",
        catalog.diagnostics
    );
    println!("Other workspace diagnostics: {:?}", catalog.diagnostics);
    for skill in skills {
        println!("{}: {}", skill.name, skill.path.display());
    }
}

#[test]
fn metadata_mapping_rejects_duplicates_and_deeper_structures() {
    for fields in [
        "metadata:\n  author: laravel\n  author: other",
        "metadata:\n  nested:\n    author: laravel",
        "name:\n  value: forged",
        "metadata:\n  author: [laravel]",
        "metadata:\n  author: &alias laravel",
    ] {
        let text = format!("---\nname: example\ndescription: Example\n{fields}\n---\nBody\n");
        assert!(
            crate::extensions::skills::metadata(&text).is_err(),
            "{fields}"
        );
    }
}
