use super::secrets::*;
use crate::workflow::{Definition, Scope};
use uuid::Uuid;

fn definition() -> Definition {
    Definition {
        scope: Scope::User,
        digest: "a".repeat(64),
        document: crate::workflow::parse(
            br#"
schema_version=1
id="authenticated-check"
version="1"
description="Check an authenticated local service"
prompt="Check {{target}} using {{token}}"
[parameters.target]
type="string"
required=true
[parameters.token]
type="string"
required=true
secret=true
"#,
        )
        .unwrap(),
    }
}

#[test]
fn secret_bindings_are_literal_public_references_and_nonsecret_metadata_only() {
    for secret in ["x", "秘密🦀", "quote\"slash\\line\nnext"] {
        let d = definition();
        let inputs =
            SecretInputs::collect(&d.document, vec![("token".into(), secret.into())]).unwrap();
        let rendered = render_public(
            &d.document,
            &[("target".into(), "local service".into())],
            &inputs.names(),
        )
        .unwrap();
        assert!(!rendered.prompt.contains(secret));
        assert!(rendered.prompt.contains("HELM_WORKFLOW_TOKEN"));
        assert_eq!(
            rendered.inputs,
            [("target".into(), serde_json::json!("local service"))]
                .into_iter()
                .collect()
        );
        let invocation = d.invocation(rendered.inputs);
        assert!(!serde_json::to_string(&invocation).unwrap().contains(secret));
        assert!(!format!("{inputs:?}").contains(secret));
        let run = Uuid::new_v4();
        let bound = inputs.bind(run).unwrap();
        let environment = bound.resolve(run, &["token".into()]).unwrap();
        assert_eq!(
            environment.iter().collect::<Vec<_>>(),
            vec![("HELM_WORKFLOW_TOKEN", secret)]
        );
        assert!(!format!("{environment:?}").contains(secret));
    }
}

#[test]
fn stale_unknown_duplicate_or_nil_secret_references_fail_without_values() {
    let d = definition();
    let run = Uuid::new_v4();
    let bound = SecretInputs::collect(
        &d.document,
        vec![("token".into(), "private-material".into())],
    )
    .unwrap()
    .bind(run)
    .unwrap();
    for (id, refs) in [
        (Uuid::new_v4(), vec!["token".into()]),
        (run, vec!["missing".into()]),
        (run, vec!["token".into(), "token".into()]),
    ] {
        let error = bound.resolve(id, &refs).unwrap_err().to_string();
        assert!(!error.contains("private-material"));
    }
    assert!(
        SecretInputs::collect(
            &d.document,
            vec![("token".into(), "private-material".into())]
        )
        .unwrap()
        .bind(Uuid::nil())
        .is_err()
    );
}

#[test]
fn secret_collection_validates_types_bounds_names_and_never_accepts_public_inputs() {
    let d = definition();
    for pairs in [
        vec![("target".into(), "private-material".into())],
        vec![("unknown".into(), "private-material".into())],
        vec![("token".into(), "a\0b".into())],
        vec![("token".into(), "a".repeat(8193))],
        vec![
            ("token".into(), "first".into()),
            ("token".into(), "second".into()),
        ],
    ] {
        assert!(SecretInputs::collect(&d.document, pairs).is_err());
    }
    assert!(
        render_public(
            &d.document,
            &[
                ("target".into(), "local".into()),
                ("token".into(), "private-material".into())
            ],
            &Default::default()
        )
        .is_err()
    );
    assert!(
        render_public(
            &d.document,
            &[("target".into(), "local".into())],
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn secret_only_workflow_and_optional_unbound_secret_render_without_persisting_values() {
    let mut d = definition();
    d.document.parameters.remove("target");
    d.document.prompt = "Authenticate with {{token}}".into();
    let inputs = SecretInputs::collect(
        &d.document,
        vec![("token".into(), "private-material".into())],
    )
    .unwrap();
    let result = render_public(&d.document, &[], &inputs.names()).unwrap();
    assert!(result.inputs.is_empty());
    assert!(result.prompt.contains("HELM_WORKFLOW_TOKEN"));
    d.document.parameters.get_mut("token").unwrap().required = false;
    let result = render_public(&d.document, &[], &Default::default()).unwrap();
    assert!(result.inputs.is_empty());
    assert_eq!(result.prompt, "Authenticate with null");
}

#[test]
fn normalized_secret_environment_collisions_fail_before_collection() {
    let mut d = definition();
    let parameter = d.document.parameters["token"].clone();
    d.document
        .parameters
        .insert("api-token".into(), parameter.clone());
    d.document.parameters.insert("api_token".into(), parameter);
    d.document.prompt.push_str(" {{api-token}} {{api_token}}");
    assert!(SecretInputs::collect(&d.document, vec![]).is_err());
}

#[test]
fn workflow_without_parameters_keeps_its_real_nonempty_prompt() {
    let mut d = definition();
    d.document.parameters.clear();
    d.document.prompt = "Review local state".into();
    let rendered = render_public(&d.document, &[], &Default::default()).unwrap();
    assert_eq!(rendered.prompt, "Review local state");
    assert!(rendered.inputs.is_empty());
}
