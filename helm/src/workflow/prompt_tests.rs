use super::*;

fn definition() -> Definition {
    Definition {
        scope: Scope::User,
        digest: "a".repeat(64),
        document: parse(
            br#"
schema_version=1
id='collect-inputs'
version='1'
description='Collection fixture'
prompt='Use {{text}} {{count}} {{mode}} {{optional}} {{token}}'
[parameters.text]
type='string'
required=true
max_length=64
[parameters.count]
type='integer'
required=true
minimum=1
maximum=3
[parameters.mode]
type='string'
required=true
default='safe'
choices=['safe','fast']
[parameters.optional]
type='boolean'
[parameters.token]
type='string'
required=true
secret=true
max_length=32
"#,
        )
        .unwrap(),
    }
}
fn input() -> InputArgs {
    InputArgs {
        selection: Selection {
            id: "collect-inputs".into(),
            scope: None,
        },
        inputs: vec![],
        secret_env: vec![],
        trust_repository: None,
        no_save: false,
        prompt_missing: true,
        input_timeout_seconds: 120,
    }
}
#[test]
fn missing_collection_preserves_defaults_optional_and_preview_secret_privacy() {
    let definition = definition();
    let names = |fields: Vec<prompt::Field>| fields.into_iter().map(|f| f.name).collect::<Vec<_>>();
    assert_eq!(
        names(missing_fields(&definition, &input(), true).unwrap()),
        ["count", "text", "token"]
    );
    assert_eq!(
        names(missing_fields(&definition, &input(), false).unwrap()),
        ["count", "text"]
    );
    let mut args = input();
    args.inputs = vec!["text=雪 literal $(touch never)".into(), "count=2".into()];
    args.secret_env = vec!["token=SOURCE_VARIABLE".into()];
    assert!(missing_fields(&definition, &args, true).unwrap().is_empty());
    let prepared = prepare_inputs(&definition, args, false, |_| {
        panic!("preview looked up secret")
    })
    .unwrap();
    assert_eq!(prepared.invocation.inputs["mode"], "safe");
    assert!(prepared.invocation.inputs["optional"].is_null());
    assert!(prepared.prompt.contains("$(touch never)"));
}
#[test]
fn invalid_supplied_values_and_untrusted_repository_fail_before_collection() {
    let definition = definition();
    for values in [
        vec!["count=0"],
        vec!["count=4"],
        vec!["count=oops"],
        vec!["mode=unknown"],
        vec!["optional=not-bool"],
        vec!["unknown=value"],
        vec!["token=PRIVATE"],
        vec!["text=one", "text=two"],
    ] {
        let mut args = input();
        args.inputs = values.into_iter().map(str::to_owned).collect();
        assert!(missing_fields(&definition, &args, true).is_err());
    }
    for values in [
        vec!["unknown=NAME"],
        vec!["token=invalid-name"],
        vec!["token=A", "token=B"],
    ] {
        let mut args = input();
        args.secret_env = values.into_iter().map(str::to_owned).collect();
        assert!(missing_fields(&definition, &args, true).is_err());
    }
    let mut repository = definition;
    repository.scope = Scope::Repository;
    assert!(missing_fields(&repository, &input(), true).is_err());
    let mut args = input();
    args.trust_repository = Some(repository.digest.clone());
    assert!(missing_fields(&repository, &args, true).is_ok());
}
#[test]
fn collected_secret_never_enters_public_rendering_or_invocation() {
    let definition = definition();
    let mut args = input();
    args.inputs = vec!["text=public".into(), "count=1".into()];
    let value = "private-雪-λ";
    let prepared = prepare_inputs_collected(
        &definition,
        args,
        true,
        |_| panic!("unexpected environment"),
        vec![("token".into(), zeroize::Zeroizing::new(value.into()))],
    )
    .unwrap();
    for public in [
        prepared.prompt.clone(),
        serde_json::to_string(&prepared.invocation).unwrap(),
        format!("{:?}", prepared.secrets),
    ] {
        assert!(!public.contains(value));
    }
    assert!(!prepared.invocation.inputs.contains_key("token"));
    assert!(prepared.prompt.contains("HELM_WORKFLOW_TOKEN"));
    let run = uuid::Uuid::new_v4();
    let environment = prepared
        .secrets
        .bind(run)
        .unwrap()
        .resolve(run, &["token".into()])
        .unwrap();
    assert_eq!(
        environment.iter().collect::<Vec<_>>(),
        [("HELM_WORKFLOW_TOKEN", value)]
    );
}
