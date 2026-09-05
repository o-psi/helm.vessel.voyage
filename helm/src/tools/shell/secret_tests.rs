use super::*;
use crate::{config::{AccessMode,Config},policy::Policy,tools::{InteractionMode,Redactor,ToolRegistry,UnattendedApprover},workflow::secrets::SecretInputs};
use std::{collections::BTreeMap,sync::Arc,time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn context(path:&std::path::Path)->ToolContext {
    ToolContext {completion:None,policy:Arc::new(Policy::new(&Config {access:Some(AccessMode::Unrestricted),..Config::default()},path.into()).unwrap()),approver:Arc::new(UnattendedApprover{allow:false}),timeout:Duration::from_secs(3),max_output_bytes:3,environment:[("PATH".into(),std::env::var("PATH").unwrap_or_default())].into_iter().collect(),cancellation:CancellationToken::new(),execution_id:Uuid::new_v4(),interaction:InteractionMode::Unattended,redactor:Arc::new(Redactor::new(["status".into(),"exited".into()]))}
}
fn bindings(run:Uuid,value:&str)->crate::workflow::secrets::RunBindings {
    let d=crate::workflow::parse(b"schema_version=1\nid='secret-check'\nversion='1'\ndescription='check'\nprompt='Use {{token}}'\n[parameters.token]\ntype='string'\nsecret=true\nrequired=true\n").unwrap();
    SecretInputs::collect(&d,vec![("token".into(),value.into())]).unwrap().bind(run).unwrap()
}

#[tokio::test]
async fn bound_shell_uses_environment_but_suppresses_short_unicode_split_and_encoded_output() {
    for value in ["x", "秘密🦀", "quotes\"slash\\newline\n"] {
        let dir=tempfile::tempdir().unwrap();
        let ctx=context(dir.path());
        let bound=bindings(ctx.execution_id,value);
        let mut registry=ToolRegistry::default();registry.register(Shell);
        // Child proves it received the value by writing its digest, not the
        // value. Both raw/split and encoded stdout/stderr must be null before
        // the ordinary three-byte output truncation could leak a prefix.
        let script="python3 -c 'import os,sys,hashlib,base64; v=os.environ[\"HELM_WORKFLOW_TOKEN\"].encode(); open(\"digest\",\"w\").write(hashlib.sha256(v).hexdigest()); [os.write(1,bytes([c])) for c in v]; os.write(2,base64.b64encode(v)); os.write(1,v.hex().encode())'";
        let result=registry.execute_with_workflow_secrets("shell",json!({"command":script,"workflow_secrets":["token"]}),&ctx,Some(&bound)).await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&result).unwrap(),json!({"status":"exited","code":0}));
        assert!(!result.contains(value));
        use sha2::Digest;
        assert_eq!(std::fs::read_to_string(dir.path().join("digest")).unwrap(),hex::encode(sha2::Sha256::digest(value.as_bytes())));
        assert!(ctx.environment.keys().all(|k|!k.starts_with("HELM_WORKFLOW_")));
    }
}

#[tokio::test]
async fn secret_references_require_exact_current_run_and_explicit_shell_opt_in_before_effects() {
    let dir=tempfile::tempdir().unwrap();let ctx=context(dir.path());
    let bound=bindings(ctx.execution_id,"synthetic-private-material");
    let mut registry=ToolRegistry::standard();
    registry.register(Shell);
    for (name,args,provided) in [
        ("shell",json!({"command":"touch forbidden","workflow_secrets":["token"]}),None),
        ("shell",json!({"command":"touch forbidden","workflow_secrets":["unknown"]}),Some(&bound)),
        ("shell",json!({"command":"touch forbidden","workflow_secrets":["token","token"]}),Some(&bound)),
        ("process",json!({"action":"start","command":"touch forbidden","workflow_secrets":["token"]}),Some(&bound)),
        ("read_file",json!({"path":"missing","workflow_secrets":["token"]}),Some(&bound)),
        ("mcp_missing",json!({"workflow_secrets":["token"]}),Some(&bound)),
        ("subagent",json!({"action":"spawn","task":"read secret","workflow_secrets":["token"]}),Some(&bound)),
    ] {
        assert!(registry.execute_with_workflow_secrets(name,args,&ctx,provided).await.is_err());
        assert!(!dir.path().join("forbidden").exists());
    }
    let mut later=ctx.clone();later.execution_id=Uuid::new_v4();
    assert!(registry.execute_with_workflow_secrets("shell",json!({"command":"touch forbidden","workflow_secrets":["token"]}),&later,Some(&bound)).await.is_err());
    let ordinary=registry.execute_with_workflow_secrets("shell",json!({"command":"test -z \"$HELM_WORKFLOW_TOKEN\""}),&ctx,Some(&bound)).await.unwrap();
    assert!(ordinary.starts_with("exi")); // ordinary output remains bounded at three bytes
    assert!(!dir.path().join("forbidden").exists());
}

#[tokio::test]
async fn bound_shell_preserves_read_only_denial_and_cancel_before_spawn() {
    let dir=tempfile::tempdir().unwrap();let mut ctx=context(dir.path());
    let bound=bindings(ctx.execution_id,"synthetic-private-material");
    let mut registry=ToolRegistry::default();registry.register(Shell);
    ctx.policy=Arc::new(Policy::new(&Config {access:Some(AccessMode::ReadOnly),..Config::default()},dir.path().into()).unwrap());
    let args=json!({"command":"touch forbidden","workflow_secrets":["token"]});
    assert!(matches!(registry.execute_with_workflow_secrets("shell",args.clone(),&ctx,Some(&bound)).await,Err(ToolError::Denied(_))));
    ctx.policy=context(dir.path()).policy;
    ctx.cancellation.cancel();
    assert!(matches!(registry.execute_with_workflow_secrets("shell",args,&ctx,Some(&bound)).await,Err(ToolError::Cancelled)));
    assert!(!dir.path().join("forbidden").exists());
}
