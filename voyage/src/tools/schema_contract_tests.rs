use super::*;
use serde_json::json;
#[test]
fn untrusted_schema_graphs_are_offline_bounded_and_nonrecursive() {
    for schema in [
        json!({"$ref":"https://example.invalid/private"}),
        json!({"$ref":"#/missing"}),
        json!({"$ref":"#"}),
        json!({"$dynamicRef":"#x"}),
        json!({"$recursiveRef":"#"}),
        json!({"properties":{"x":{"$id":"other"}}}),
        json!({"pattern":"(?=private)"}),
        json!({"patternProperties":{"(a)\\1":{}}}),
        json!({"description":"x".repeat(MAX_SCHEMA_BYTES)}),
    ] {
        assert!(CompiledSchema::compile(&schema).is_err(), "{schema}");
    }
    let mut deep = json!({});
    for _ in 0..MAX_DEPTH + 2 {
        deep = json!({"items":deep});
    }
    assert!(CompiledSchema::compile(&deep).is_err());
    let broad = json!({"allOf":vec![json!({});MAX_NODES+1]});
    assert!(CompiledSchema::compile(&broad).is_err());
}
#[test]
fn local_references_and_schema_keywords_enforce_values_without_leaking_them() {
    let schema = json!({"type":"object","$defs":{"positive":{"type":"integer","minimum":1}},"properties":{"count":{"$ref":"#/$defs/positive"}},"required":["count"],"additionalProperties":false});
    let validator = CompiledSchema::compile(&schema).unwrap();
    assert!(validator.validate(&json!({"count":1})).is_ok());
    for value in [
        json!({}),
        json!({"count":0}),
        json!({"count":"secret-value"}),
        json!({"count":1,"private":true}),
    ] {
        let error = validator.validate(&value).unwrap_err().to_string();
        assert!(!error.contains("secret-value") && !error.contains("private"));
    }
    assert!(
        CompiledSchema::compile(&json!(true))
            .unwrap()
            .validate(&json!("x"))
            .is_ok()
    );
    assert!(
        CompiledSchema::compile(&json!(false))
            .unwrap()
            .validate(&json!(null))
            .is_err()
    );
    assert!(
        CompiledSchema::compile(&json!({}))
            .unwrap()
            .validate(&json!("x".repeat(MAX_INSTANCE_BYTES)))
            .is_err()
    );
}
#[test]
fn traversal_inspects_every_schema_bearing_keyword_not_instance_annotations() {
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
        "dependencies",
    ] {
        let schema = json!({key:{"x":{"$ref":"https://example.invalid/forbidden"}}});
        assert!(CompiledSchema::compile(&schema).is_err(), "{key}");
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        assert!(
            CompiledSchema::compile(&json!({key:[{"$ref":"https://example.invalid/forbidden"}]}))
                .is_err()
        );
    }
    for key in [
        "items",
        "additionalItems",
        "additionalProperties",
        "unevaluatedItems",
        "unevaluatedProperties",
        "contains",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
        "contentSchema",
    ] {
        assert!(
            CompiledSchema::compile(&json!({key:{"$ref":"https://example.invalid/forbidden"}}))
                .is_err(),
            "{key}"
        );
    }
    assert!(
        CompiledSchema::compile(&json!({"const":{"$ref":"https://example.invalid/data"}})).is_ok()
    );
}
