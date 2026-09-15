use super::*;
fn source(id: &str) -> String {
    format!(
        r#"schema_version = 1
id = "{id}"
version = "1.0"
description = "Offline workflow"
prompt = "Review {{{{topic}}}} with {{{{count}}}} samples; enabled={{{{enabled}}}}"
[parameters.topic]
type = "string"
required = true
max_length = 20
[parameters.count]
type = "integer"
default = 2
minimum = 1
maximum = 3
[parameters.enabled]
type = "boolean"
default = true
"#
    )
}
#[test]
fn parameter_types_defaults_choices_and_template_rendering() {
    let document = parse(source("offline-review").as_bytes()).unwrap();
    let rendered = document.render(&[("topic".into(), "Rust".into())]).unwrap();
    assert_eq!(
        rendered.prompt,
        "Review \"Rust\" with 2 samples; enabled=true"
    );
    assert_eq!(rendered.inputs["count"], serde_json::json!(2));
    let rendered = document
        .render(&[
            ("topic".into(), "{{count}}".into()),
            ("count".into(), "3".into()),
            ("enabled".into(), "false".into()),
        ])
        .unwrap();
    assert_eq!(
        rendered.prompt,
        "Review \"{{count}}\" with 3 samples; enabled=false"
    );
    for inputs in [
        vec![],
        vec![("unknown".into(), "x".into())],
        vec![("topic".into(), "x".into()), ("topic".into(), "x".into())],
        vec![("topic".into(), "x".repeat(21))],
        vec![("topic".into(), "x".into()), ("count".into(), "4".into())],
        vec![
            ("topic".into(), "x".into()),
            ("enabled".into(), "TRUE".into()),
        ],
    ] {
        assert!(document.render(&inputs).is_err());
    }
    let mut choices = document.parameters["count"].clone();
    choices.choices = vec![serde_json::json!(1), serde_json::json!(3)];
    assert!(choices.decode("1").is_ok());
    assert!(choices.decode("2").is_err());
    assert!(choices.decode("-1").is_err());
    assert!(choices.decode("1.0").is_err());
    assert!(!choices.accepts(&serde_json::json!(true)));
    let mut secret = document.clone();
    secret.parameters.get_mut("topic").unwrap().secret = true;
    secret.validate().unwrap();
    assert!(secret.render(&[("topic".into(), "value".into())]).is_err());
}
#[test]
fn document_and_template_validation_fail_closed() {
    let valid = parse(source("offline-review").as_bytes()).unwrap();
    let mutations: Vec<Box<dyn Fn(&mut Document)>> = vec![
        Box::new(|d| d.schema_version = 2),
        Box::new(|d| d.id = "../escape".into()),
        Box::new(|d| d.version = String::new()),
        Box::new(|d| d.version = "invalid version".into()),
        Box::new(|d| d.description = " ".into()),
        Box::new(|d| d.prompt = " ".into()),
        Box::new(|d| d.prompt = "{{missing}}".into()),
        Box::new(|d| d.prompt = "{{ topic }}".into()),
        Box::new(|d| d.prompt = "{{topic".into()),
        Box::new(|d| d.prompt = "orphan }}".into()),
        Box::new(|d| d.prompt = "}} {{topic}}".into()),
        Box::new(|d| d.parameters.get_mut("topic").unwrap().max_length = Some(0)),
        Box::new(|d| d.parameters.get_mut("topic").unwrap().minimum = Some(0)),
        Box::new(|d| d.parameters.get_mut("count").unwrap().max_length = Some(5)),
        Box::new(|d| d.parameters.get_mut("count").unwrap().minimum = Some(5)),
        Box::new(|d| d.parameters.get_mut("count").unwrap().default = Some(serde_json::json!(99))),
        Box::new(|d| d.parameters.get_mut("count").unwrap().secret = true),
        Box::new(|d| {
            d.parameters.get_mut("topic").unwrap().choices = vec![serde_json::json!(true)]
        }),
        Box::new(|d| d.recommended.model = Some("x".repeat(257))),
    ];
    for mutation in mutations {
        let mut bad = valid.clone();
        mutation(&mut bad);
        assert!(bad.validate().is_err());
    }
    assert!(parse(&[255]).is_err());
    assert!(parse(b"not toml").is_err());
    assert!(parse(&vec![b' '; MAX_DOCUMENT + 1]).is_err());
    assert!(parse(format!("{}\nunknown = 1", source("offline-review")).as_bytes()).is_err());
}
#[test]
fn private_discovery_requires_exact_repository_trust_and_keeps_scopes_distinct() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("user");
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(workspace.join(".helm/workflows")).unwrap();
    std::fs::write(user.join("offline-review.toml"), source("offline-review")).unwrap();
    std::fs::write(
        workspace.join(".helm/workflows/offline-review.toml"),
        source("offline-review"),
    )
    .unwrap();
    std::fs::write(user.join("ignored.txt"), "not a workflow").unwrap();
    let definitions = discover(&workspace, Some(&user)).unwrap();
    assert_eq!(definitions.len(), 2);
    let local = definitions.iter().find(|d| d.scope == Scope::User).unwrap();
    local.authorize(None).unwrap();
    let repository = definitions
        .iter()
        .find(|d| d.scope == Scope::Repository)
        .unwrap();
    assert!(repository.authorize(None).is_err());
    assert!(repository.authorize(Some("wrong")).is_err());
    repository.authorize(Some(&repository.digest)).unwrap();
    assert_eq!(local.digest, repository.digest);
    let invocation = repository.invocation(BTreeMap::from([(
        "topic".into(),
        Value::String("Rust".into()),
    )]));
    assert_eq!(invocation.digest, repository.digest);
    assert_eq!(invocation.scope, Scope::Repository);
    std::fs::write(user.join("duplicate.toml"), source("offline-review")).unwrap();
    assert!(discover(&workspace, Some(&user)).is_err());
}
#[cfg(unix)]
#[test]
fn discovery_rejects_symlinked_documents_and_scope_directories() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let user = root.path().join("user");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&user).unwrap();
    let external = root.path().join("external.toml");
    std::fs::write(&external, source("offline-review")).unwrap();
    symlink(&external, user.join("linked.toml")).unwrap();
    assert!(discover(&workspace, Some(&user)).is_err());
    std::fs::remove_file(user.join("linked.toml")).unwrap();
    symlink(&user, workspace.join(".helm")).unwrap();
    assert!(discover(&workspace, Some(&user)).is_err());
}
