use super::*;
fn document() -> &'static str {
    r#"
schema_version = 1
id = "review-change"
version = "1.0"
description = "Review a change"
prompt = "Review {{target}} with {{count}} checks. Strict={{strict}}"
[parameters.target]
type = "string"
required = true
[parameters.count]
type = "integer"
default = 2
minimum = 1
maximum = 5
[parameters.strict]
type = "boolean"
default = true
"#
}
#[test]
fn typed_literal_rendering_and_metadata() {
    let workflow = parse(document().as_bytes()).unwrap();
    let inputs = vec![("target".into(), "$(touch nope) {{strict}}\n雪".into())];
    let rendered = workflow.render(&inputs).unwrap();
    assert_eq!(
        rendered.prompt,
        "Review \"$(touch nope) {{strict}}\\n雪\" with 2 checks. Strict=true"
    );
    assert_eq!(rendered.inputs["count"], serde_json::json!(2));
    assert_eq!(rendered.inputs["target"], serde_json::json!(inputs[0].1));
}
#[test]
fn rejects_invalid_inputs_and_documents() {
    let workflow = parse(document().as_bytes()).unwrap();
    for inputs in [
        vec![],
        vec![("unknown".into(), "x".into())],
        vec![("target".into(), "x".into()), ("target".into(), "y".into())],
        vec![("target".into(), "x".into()), ("count".into(), "6".into())],
        vec![
            ("target".into(), "x".into()),
            ("strict".into(), "yes".into()),
        ],
    ] {
        assert!(workflow.render(&inputs).is_err());
    }
    for text in [
        document().replace("schema_version = 1", "schema_version = 2"),
        document().replace("{{target}}", "{{missing}}"),
        document().replace("id = \"review-change\"", "id = \"run\""),
        document().replace("prompt =", "unknown = 5\nprompt ="),
        document().replace("default = 2", "default = 9"),
    ] {
        assert!(parse(text.as_bytes()).is_err(), "{text}");
    }
}
#[test]
fn secret_execution_is_rejected_without_echoing_values() {
    let text = document().replace("required = true", "required = true\nsecret = true");
    let workflow = parse(text.as_bytes()).unwrap();
    let error = workflow
        .render(&[("target".into(), "SECRET_CANARY".into())])
        .unwrap_err()
        .to_string();
    assert!(error.contains("secret parameters are not supported for execution"));
    assert!(!error.contains("SECRET_CANARY"));
}
#[test]
fn validation_bounds_and_choices() {
    let text = document().replace(
        "required = true",
        "required = true\nchoices = [\"one\", \"two\"]\nmax_length = 3",
    );
    let workflow = parse(text.as_bytes()).unwrap();
    assert!(workflow.render(&[("target".into(), "one".into())]).is_ok());
    assert!(
        workflow
            .render(&[("target".into(), "three".into())])
            .is_err()
    );
    assert!(parse(&vec![b'x'; MAX_DOCUMENT + 1]).is_err());
}

#[test]
fn scopes_trust_digest_and_changed_definitions() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let user = root.path().join("user");
    std::fs::create_dir_all(workspace.join(".helm/workflows")).unwrap();
    std::fs::create_dir(&user).unwrap();
    std::fs::write(user.join("review-change.toml"), document()).unwrap();
    let repo = workspace.join(".helm/workflows/review-change.toml");
    std::fs::write(&repo, document().replace("1.0", "2.0")).unwrap();
    let definitions = discover(&workspace, Some(&user)).unwrap();
    assert_eq!(definitions.len(), 2);
    let selected = select(definitions.clone(), "review-change", None).unwrap();
    assert_eq!(selected.scope, Scope::Repository);
    assert!(selected.authorize(None).is_err());
    assert!(selected.authorize(Some(&selected.digest)).is_ok());
    assert_eq!(
        select(definitions, "review-change", Some(Scope::User))
            .unwrap()
            .document
            .version,
        "1.0"
    );
    std::fs::write(repo, document().replace("1.0", "3.0")).unwrap();
    let changed = select(
        discover(&workspace, Some(&user)).unwrap(),
        "review-change",
        None,
    )
    .unwrap();
    assert!(changed.authorize(Some(&selected.digest)).is_err());
}
#[cfg(unix)]
#[test]
fn rejects_symlink_directories_and_nonregular_or_oversized_files() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join(".helm")).unwrap();
    assert!(discover(&workspace, Some(&root.path().join("absent"))).is_err());
    std::fs::remove_file(workspace.join(".helm")).unwrap();
    std::fs::create_dir_all(workspace.join(".helm/workflows")).unwrap();
    std::fs::write(outside.join("secret.toml"), document()).unwrap();
    let file = workspace.join(".helm/workflows/review-change.toml");
    std::os::unix::fs::symlink(outside.join("secret.toml"), &file).unwrap();
    assert!(discover(&workspace, Some(&root.path().join("absent"))).is_err());
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(discover(&workspace, Some(&root.path().join("absent"))).is_err());
    std::fs::remove_dir(&file).unwrap();
    std::fs::write(file, vec![b'x'; MAX_DOCUMENT + 1]).unwrap();
    assert!(discover(&workspace, Some(&root.path().join("absent"))).is_err());
}
#[tokio::test]
async fn invocation_survives_session_roundtrip_and_branch_without_prompt_rewriting() {
    let root = tempfile::tempdir().unwrap();
    let store = crate::session::SessionStore::new(root.path().join("sessions"));
    let mut session = crate::session::Session::new(root.path().into(), "selected-model".into());
    let definition = Definition {
        scope: Scope::User,
        digest: "a".repeat(64),
        document: parse(document().as_bytes()).unwrap(),
    };
    let rendered = definition
        .document
        .render(&[("target".into(), "data".into())])
        .unwrap();
    session
        .workflow_runs
        .push(definition.invocation(rendered.inputs));
    session.messages.push(crate::model::Message::new(
        crate::model::Role::User,
        rendered.prompt.clone(),
    ));
    store.save(&mut session).await.unwrap();
    let saved = store.load(session.id).await.unwrap();
    assert_eq!(saved.workflow_runs[0].version, "1.0");
    assert_eq!(saved.messages[0].content, rendered.prompt);
    let branch = store.branch(&saved, None).await.unwrap();
    assert_eq!(branch.workflow_runs[0].digest, "a".repeat(64));
    let mut legacy = serde_json::to_value(&session).unwrap();
    legacy.as_object_mut().unwrap().remove("workflow_runs");
    assert!(
        serde_json::from_value::<crate::session::Session>(legacy)
            .unwrap()
            .workflow_runs
            .is_empty()
    );
}
#[test]
fn all_existing_builtin_commands_remain_reserved() {
    for line in include_str!("../tui/palette.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("name: \""))
    {
        let name = line.split('"').next().unwrap();
        assert!(RESERVED.contains(&name), "{name}");
    }
}

#[test]
fn aggregate_render_limit_and_future_fields_fail_closed() {
    let mut workflow = parse(document().as_bytes()).unwrap();
    workflow.prompt = "{{target}}".repeat(32);
    assert!(
        workflow
            .render(&[("target".into(), "x".repeat(8192))])
            .is_err()
    );
    assert!(
        parse(
            document()
                .replace("type = \"boolean\"", "type = \"boolean\"\nfuture = true")
                .as_bytes()
        )
        .is_err()
    );
    for value in [
        "\"quoted\"",
        "line\r\n\u{1b}[31m",
        "$HOME; touch x",
        "{{unknown}}",
        "}",
        "雪",
    ] {
        let rendered = parse(document().as_bytes())
            .unwrap()
            .render(&[("target".into(), value.into())])
            .unwrap();
        assert!(
            rendered
                .prompt
                .contains(&serde_json::to_string(value).unwrap())
        );
    }
}

#[test]
fn public_render_revalidates_constructed_documents_and_human_controls() {
    let mut workflow = parse(document().as_bytes()).unwrap();
    workflow.prompt = "{{unknown}}".into();
    assert!(workflow.render(&[("target".into(), "x".into())]).is_err());
    assert_eq!(
        safe_text("before\u{1b}[2J\r after"),
        "before\\u{1b}[2J\\r after"
    );
}

#[test]
fn optional_unset_is_explicit_null_and_secret_defaults_fail_without_echo() {
    let text = document().replace("required = true", "required = false");
    let result = parse(text.as_bytes()).unwrap().render(&[]).unwrap();
    assert_eq!(result.inputs["target"], Value::Null);
    assert!(result.prompt.starts_with("Review null"));
    let secret_default = document().replace(
        "required = true",
        "required = true\nsecret = true\ndefault = \"KNOWN_SECRET_CANARY\"",
    );
    let error = parse(secret_default.as_bytes()).unwrap_err().to_string();
    assert!(!error.contains("KNOWN_SECRET_CANARY"));
}
