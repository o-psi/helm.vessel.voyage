use super::*;
fn document() -> &'static str { r#"
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
"# }
#[test]
fn typed_literal_rendering_and_metadata() {
 let workflow = parse(document().as_bytes()).unwrap();
 let inputs = vec![("target".into(),"$(touch nope) {{strict}}\n雪".into())];
 let rendered = workflow.render(&inputs).unwrap();
 assert_eq!(rendered.prompt,"Review \"$(touch nope) {{strict}}\\n雪\" with 2 checks. Strict=true");
 assert_eq!(rendered.inputs["count"],serde_json::json!(2));
 assert_eq!(rendered.inputs["target"],serde_json::json!(inputs[0].1));
}
#[test]
fn rejects_invalid_inputs_and_documents() {
 let workflow=parse(document().as_bytes()).unwrap();
 for inputs in [vec![],vec![("unknown".into(),"x".into())],vec![("target".into(),"x".into()),("target".into(),"y".into())],vec![("target".into(),"x".into()),("count".into(),"6".into())],vec![("target".into(),"x".into()),("strict".into(),"yes".into())]] { assert!(workflow.render(&inputs).is_err()); }
 for text in [document().replace("schema_version = 1","schema_version = 2"),document().replace("{{target}}","{{missing}}"),document().replace("id = \"review-change\"","id = \"run\""),document().replace("prompt =", "unknown = 5\nprompt ="),document().replace("default = 2","default = 9")] { assert!(parse(text.as_bytes()).is_err(),"{text}"); }
}
#[test]
fn secret_execution_is_rejected_without_echoing_values() {
 let text=document().replace("required = true","required = true\nsecret = true");
 let workflow=parse(text.as_bytes()).unwrap();
 let error=workflow.render(&[("target".into(),"SECRET_CANARY".into())]).unwrap_err().to_string();
 assert!(error.contains("secret parameters are not supported for execution"));
 assert!(!error.contains("SECRET_CANARY"));
}
#[test]
fn validation_bounds_and_choices() {
 let text=document().replace("required = true","required = true\nchoices = [\"one\", \"two\"]\nmax_length = 3");
 let workflow=parse(text.as_bytes()).unwrap();
 assert!(workflow.render(&[("target".into(),"one".into())]).is_ok());
 assert!(workflow.render(&[("target".into(),"three".into())]).is_err());
 assert!(parse(&vec![b'x';MAX_DOCUMENT+1]).is_err());
}
