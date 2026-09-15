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
