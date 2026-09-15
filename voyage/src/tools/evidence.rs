//! Immutable, redacted tool evidence. Retrieval never dispatches the original tool.
use super::{Tool, ToolContext, ToolError, ToolReport};
use crate::{artifacts::Store, model::ToolDefinition};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use voyage_protocol::tool_result::{ToolContent, ToolOutcome};

pub(crate) const MIME: &str = "application/vnd.helm.tool-evidence+json";
const THRESHOLD: usize = 16 * 1024;
const PAGE: usize = 4096;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    version: u32,
    tool: String,
    call_id: Option<String>,
    execution_id: Uuid,
    outcome: ToolOutcome,
    capture: String,
    text: String,
}

/// Best-effort additive evidence: never replace canonical output or its outcome on failure.
/// Store publication precedes the reference. Unreferenced crash leftovers remain quota-bound.
pub(crate) fn retain(name: &str, report: &mut ToolReport, ctx: &ToolContext) {
    if name == "result" || ctx.cancellation.is_cancelled() {
        return;
    }
    let Some(scope) = &ctx.artifact_scope else {
        return;
    };
    let text = report.output.text_fallback();
    if text.len() < THRESHOLD || text.len() > crate::artifacts::MAX_BYTES / 2 {
        return;
    }
    let evidence = Evidence {
        version: 1,
        tool: name.into(),
        call_id: ctx.tool_call_id.clone(),
        execution_id: ctx.execution_id,
        outcome: report.outcome.clone(),
        // Legacy tool adapters do not attest source capture completeness.
        capture: if report.outcome.incomplete.is_some() {
            "incomplete"
        } else {
            "unknown"
        }
        .into(),
        text,
    };
    let saved = (|| -> anyhow::Result<_> {
        ctx.policy.check_execution_authority()?;
        let bytes = serde_json::to_vec(&evidence)?;
        Store::open(&scope.directory, scope.session)?.put(
            Uuid::new_v4(),
            "tool-evidence.json",
            MIME,
            &bytes,
        )
    })();
    if let Ok(artifact) = saved {
        report.output.content.push(ToolContent::Resource {
            uri: format!("helm-evidence:{}", artifact.id),
            mime_type: Some(MIME.into()),
            text: None,
            artifact: Some(artifact),
        });
        if report.output.text_fallback().len() > ctx.max_output_bytes {
            report.output.content.pop();
        }
    }
}

pub struct ResultTool;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    action: Action,
    id: Uuid,
    #[serde(default)]
    offset: usize,
    #[serde(default = "page")]
    limit: usize,
    query: Option<String>,
}
fn page() -> usize {
    PAGE
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Describe,
    Read,
    Search,
}

#[async_trait]
impl Tool for ResultTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "result".into(),
            description: "Read or literal-search immutable saved tool evidence in this session; never reruns tools. Offsets address UTF-8 bytes of retained text. Follow next_offset until null; no match before scanned_to reaches total_bytes is not absence. Capture may be incomplete or unknown. References confer no cross-session authority.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"properties":{
                "action":{"enum":["describe","read","search"]},"id":{"type":"string","format":"uuid"},
                "offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":4096},
                "query":{"type":"string","minLength":1,"maxLength":256}},"required":["action","id"]}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let args: Args =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        ctx.policy
            .check_execution_authority()
            .map_err(|_| ToolError::Denied("evidence authority unavailable".into()))?;
        if ctx.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let scope = ctx
            .artifact_scope
            .as_ref()
            .ok_or_else(|| ToolError::Failed("evidence requires a session-owned runtime".into()))?;
        let (metadata, bytes) = Store::open(&scope.directory, scope.session)
            .and_then(|store| store.get(args.id))
            .map_err(|_| ToolError::Failed("evidence unavailable or integrity check failed; do not rerun the original tool".into()))?;
        if metadata.mime_type != MIME {
            return Err(ToolError::InvalidArguments(
                "not saved tool evidence".into(),
            ));
        }
        let evidence: Evidence = serde_json::from_slice(&bytes)
            .map_err(|_| ToolError::Failed("invalid evidence manifest".into()))?;
        // Do not disclose new credentials that became known after capture, even across pages.
        if ctx.redactor.contains_secret(&evidence.text) {
            return Err(ToolError::Denied(
                "saved evidence withheld by current confidentiality policy".into(),
            ));
        }
        let response = query(&evidence, &args)?;
        let text = serde_json::to_string(&response)
            .map_err(|_| ToolError::Failed("cannot encode evidence".into()))?;
        if text.len() > ctx.max_output_bytes {
            return Err(ToolError::Failed(
                "evidence page exceeds output budget; request a smaller limit".into(),
            ));
        }
        Ok(text)
    }
}

fn query(e: &Evidence, args: &Args) -> Result<Value, ToolError> {
    let invalid = |s: &str| ToolError::InvalidArguments(s.into());
    if e.version != 1 {
        return Err(invalid("unsupported evidence version"));
    }
    if args.limit == 0
        || args.limit > PAGE
        || args.offset > e.text.len()
        || !e.text.is_char_boundary(args.offset)
    {
        return Err(invalid("invalid byte offset or limit"));
    }
    if !matches!(args.action, Action::Search) && args.query.is_some() {
        return Err(invalid("query is only valid for search"));
    }
    let mut out = json!({"version":1,"id":args.id,"tool":e.tool,"call_id":e.call_id,
        "execution_id":e.execution_id,"outcome":e.outcome,"capture":e.capture,
        "representation":"redacted retained tool text","total_bytes":e.text.len(),
        "text_sha256":hex::encode(Sha256::digest(e.text.as_bytes()))});
    if matches!(args.action, Action::Describe) {
        return Ok(out);
    }
    let mut end = args.offset.saturating_add(args.limit).min(e.text.len());
    while end < e.text.len() && !e.text.is_char_boundary(end) {
        end += 1;
    }
    out["offset"] = json!(args.offset);
    out["next_offset"] = if end < e.text.len() {
        json!(end)
    } else {
        Value::Null
    };
    match args.action {
        Action::Read => {
            out["end"] = json!(end);
            out["text"] = json!(&e.text[args.offset..end]);
        }
        Action::Search => {
            let needle = args
                .query
                .as_deref()
                .ok_or_else(|| invalid("search requires query"))?;
            if needle.is_empty() || needle.len() > 256 {
                return Err(invalid("query must contain 1..256 UTF-8 bytes"));
            }
            // Search starts in this page, extending the scan to find cross-page matches.
            let mut scan_end = end.saturating_add(needle.len()).min(e.text.len());
            while scan_end < e.text.len() && !e.text.is_char_boundary(scan_end) {
                scan_end += 1;
            }
            let mut hits = Vec::new();
            for (local, _) in e.text[args.offset..scan_end].match_indices(needle) {
                let start = args.offset + local;
                if start >= end {
                    break;
                }
                if hits.len() == 32 {
                    end = start;
                    out["next_offset"] = json!(start);
                    break;
                }
                hits.push(json!({"start":start,"end":start+needle.len()}));
            }
            out["matches"] = json!(hits);
            out["scanned_to"] = json!(end);
        }
        Action::Describe => unreachable!(),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(text: &str) -> Evidence {
        Evidence {
            version: 1,
            tool: "shell".into(),
            call_id: Some("call-1".into()),
            execution_id: Uuid::nil(),
            outcome: ToolOutcome::default(),
            capture: "unknown".into(),
            text: text.into(),
        }
    }
    fn args(action: Action, offset: usize, limit: usize, query: Option<&str>) -> Args {
        Args {
            action,
            id: Uuid::nil(),
            offset,
            limit,
            query: query.map(str::to_owned),
        }
    }
    #[test]
    fn exact_utf8_and_unknown_capture() {
        let e = fixture("éSHA=abc failure:test_middle refusal");
        let v = query(&e, &args(Action::Read, 0, 1, None)).unwrap();
        assert_eq!(v["text"], "é");
        assert_eq!(v["next_offset"], 2);
        assert_eq!(v["capture"], "unknown");
        assert!(query(&e, &args(Action::Read, 1, 4, None)).is_err());
        assert!(query(&e, &args(Action::Read, 0, 0, None)).is_err());
    }
    #[test]
    fn search_crosses_page_and_bounds_matches() {
        let e = fixture("000SHA=abcd");
        let v = query(&e, &args(Action::Search, 0, 4, Some("SHA=abcd"))).unwrap();
        assert_eq!(v["matches"][0]["start"], 3);
        let e = fixture(&"x".repeat(100));
        let v = query(&e, &args(Action::Search, 0, 100, Some("x"))).unwrap();
        assert_eq!(v["matches"].as_array().unwrap().len(), 32);
        assert_eq!(v["next_offset"], 32);
    }
    #[test]
    fn empty_page_is_not_absence() {
        let e = fixture("0000refused");
        let v = query(&e, &args(Action::Search, 0, 4, Some("refused"))).unwrap();
        assert_eq!(v["matches"], json!([]));
        assert_eq!(v["next_offset"], 4);
        let v = query(&e, &args(Action::Search, 4, 20, Some("refused"))).unwrap();
        assert_eq!(v["matches"][0]["start"], 4);
        assert!(v["next_offset"].is_null());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn retained_evidence_survives_projection_restart_and_checks_authority() {
        use crate::context::WorkingContext;
        use crate::model::{Message, Role};
        use crate::tools::{Redactor, reliability_tests::context};
        use std::sync::Arc;
        let root = tempfile::tempdir().unwrap();
        let mut ctx = context(root.path());
        ctx.max_output_bytes = 64 * 1024;
        let session = Uuid::new_v4();
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("artifacts"),
            session,
        });
        let text = format!(
            "{}SHA=0123456789 failed:test_middle refused{}",
            "a".repeat(10000),
            "z".repeat(10000)
        );
        let mut report = ToolReport::command(text.clone(), Some(1), true);
        retain("shell", &mut report, &ctx);
        let id = report.output.artifacts().next().unwrap().id;
        let result = ResultTool
            .execute(
                json!({"action":"read","id":id,"offset":10000,"limit":44}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.contains("SHA=0123456789 failed:test_middle refused"));
        assert!(result.contains("incomplete"));
        let mut message = Message::new(Role::Tool, report.output.text_fallback());
        message.tool_call_id = Some("call-1".into());
        message.tool_outcome = Some(report.outcome.clone());
        message.tool_success = Some(false);
        message.tool_output = Some(Box::new(report.output));
        let canonical = vec![message];
        let mut working = WorkingContext::default();
        assert_eq!(working.prepare(&canonical).unwrap(), 1);
        let projection = working.project(&canonical).unwrap();
        assert!(projection[0].content.len() < 6000);
        assert!(projection[0].content.contains(&id.to_string()));
        assert_eq!(projection[0].tool_outcome, canonical[0].tool_outcome);
        assert!(canonical[0].content.contains(&text));
        let restored: WorkingContext =
            serde_json::from_str(&serde_json::to_string(&working).unwrap()).unwrap();
        restored.validate(&canonical).unwrap();
        assert_eq!(working.prepare(&canonical).unwrap(), 0);
        assert!(
            ResultTool
                .execute(json!({"action":"describe","id":Uuid::new_v4()}), &ctx)
                .await
                .is_err()
        );
        ctx.redactor = Arc::new(Redactor::new(vec!["0123456789".into()]));
        assert!(
            ResultTool
                .execute(json!({"action":"read","id":id}), &ctx)
                .await
                .is_err()
        );
        ctx.redactor = Arc::new(Redactor::default());
        ctx.artifact_scope.as_mut().unwrap().session = Uuid::new_v4();
        assert!(
            ResultTool
                .execute(json!({"action":"describe","id":id}), &ctx)
                .await
                .is_err()
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn registry_redacts_before_retention_and_does_not_reexecute() {
        use crate::tools::{Redactor, ToolRegistry, reliability_tests::context};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Counting(Arc<AtomicUsize>);
        #[async_trait]
        impl Tool for Counting {
            fn definition(&self) -> ToolDefinition {
                ToolDefinition {
                    name: "fixture".into(),
                    description: "fixture".into(),
                    input_schema: json!({"type":"object"}),
                    output_schema: None,
                    annotations: None,
                }
            }
            async fn execute(&self, _: Value, _: &ToolContext) -> Result<String, ToolError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(format!(
                    "{}PRIVATE_SECRET{}",
                    "x".repeat(9000),
                    "y".repeat(9000)
                ))
            }
        }
        let root = tempfile::tempdir().unwrap();
        let mut ctx = context(root.path());
        ctx.max_output_bytes = 64000;
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("store"),
            session: Uuid::new_v4(),
        });
        ctx.redactor = Arc::new(Redactor::new(vec!["PRIVATE_SECRET".into()]));
        let count = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::standard();
        registry.register(Counting(count.clone()));
        let report = registry
            .execute_report_with_workflow_secrets("fixture", json!({}), &ctx, None)
            .await
            .unwrap();
        let id = report.output.artifacts().next().unwrap().id;
        let scope = ctx.artifact_scope.as_ref().unwrap();
        let (_, bytes) = Store::open(&scope.directory, scope.session)
            .unwrap()
            .get(id)
            .unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("PRIVATE_SECRET"));
        for _ in 0..2 {
            let result = registry
                .execute_report_with_workflow_secrets(
                    "result",
                    json!({"action":"read","id":id,"offset":9000,"limit":30}),
                    &ctx,
                    None,
                )
                .await
                .unwrap();
            assert!(result.output.text_fallback().contains("[REDACTED]"));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        ctx.cancellation.cancel();
        assert!(
            ResultTool
                .execute(json!({"action":"describe","id":id}), &ctx)
                .await
                .is_err()
        );
    }
}
