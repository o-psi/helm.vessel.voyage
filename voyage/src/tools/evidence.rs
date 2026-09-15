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
    // Reserve the exact reference before any durable write; near-budget output
    // must not consume unreachable artifact quota.
    let Ok(bytes) = serde_json::to_vec(&evidence) else {
        return;
    };
    let id = Uuid::new_v4();
    let candidate = voyage_protocol::tool_result::ArtifactReference {
        id,
        name: "tool-evidence.json".into(),
        mime_type: MIME.into(),
        byte_size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(&bytes)),
    };
    let mut projected = report.output.clone();
    projected.content.push(ToolContent::Resource {
        uri: format!("helm-evidence:{id}"),
        mime_type: Some(MIME.into()),
        text: None,
        artifact: Some(candidate),
    });
    if projected.text_fallback().len() > ctx.max_output_bytes {
        return;
    }
    let saved = (|| -> anyhow::Result<_> {
        ctx.policy.check_execution_authority()?;

        Store::open(&scope.directory, scope.session)?.put_evidence(id, "tool-evidence.json", &bytes)
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

/// Resolve provenance under current session authority before a task-state mutation.
pub(crate) fn link(
    ctx: &ToolContext,
    id: Uuid,
    start: usize,
    end: usize,
) -> Result<crate::todo::EvidenceLink, ToolError> {
    ctx.policy
        .check_execution_authority()
        .map_err(|_| ToolError::Denied("evidence authority unavailable".into()))?;
    if ctx.cancellation.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    let scope = ctx
        .artifact_scope
        .as_ref()
        .ok_or_else(|| ToolError::Failed("evidence requires a session".into()))?;
    let (artifact, bytes) = Store::open(&scope.directory, scope.session)
        .and_then(|s| s.get_evidence(id))
        .map_err(|_| ToolError::Failed("evidence unavailable or corrupt".into()))?;
    if artifact.mime_type != MIME {
        return Err(ToolError::InvalidArguments("not tool evidence".into()));
    }
    let e: Evidence =
        serde_json::from_slice(&bytes).map_err(|_| ToolError::Failed("invalid evidence".into()))?;
    if e.version != 1
        || !matches!(e.capture.as_str(), "unknown" | "incomplete")
        || start >= end
        || end > e.text.len()
        || !e.text.is_char_boundary(start)
        || !e.text.is_char_boundary(end)
    {
        return Err(ToolError::InvalidArguments(
            "invalid evidence range/version".into(),
        ));
    }
    if ctx.redactor.contains_secret(&e.text) {
        return Err(ToolError::Denied(
            "evidence withheld by confidentiality policy".into(),
        ));
    }
    Ok(crate::todo::EvidenceLink {
        session: scope.session,
        artifact,
        start,
        end,
        text_sha256: hex::encode(Sha256::digest(e.text.as_bytes())),
        capture: e.capture,
    })
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
            .and_then(|store| store.get_evidence(args.id))
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
    if e.version != 1 || !matches!(e.capture.as_str(), "unknown" | "incomplete") {
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
            for (local, _) in e.text[args.offset..scan_end]
                .char_indices()
                .filter(|(i, _)| e.text[args.offset + *i..scan_end].starts_with(needle))
            {
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
    #[cfg(unix)]
    #[tokio::test]
    async fn linked_task_state_forgery_and_quota_regressions() {
        use crate::todo::{TodoScope, TodoStore};
        use crate::tools::{reliability_tests::context, todo::TodoTool};
        use std::sync::Arc;
        let root = tempfile::tempdir().unwrap();
        let mut ctx = context(root.path());
        ctx.max_output_bytes = 64000;
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("artifacts"),
            session: Uuid::new_v4(),
        });
        let scope = ctx.artifact_scope.as_ref().unwrap().clone();
        let mut store = Store::open(&scope.directory, scope.session).unwrap();
        assert!(
            store
                .put(Uuid::new_v4(), "forged.json", MIME, b"{}")
                .is_err()
        );
        let forged = Uuid::new_v4();
        store
            .put_runtime(
                forged,
                "old-forgery.json",
                MIME,
                &serde_json::to_vec(&fixture("untrusted")).unwrap(),
            )
            .unwrap();
        assert!(
            ResultTool
                .execute(json!({"action":"read","id":forged}), &ctx)
                .await
                .is_err()
        );
        // Near-budget references must not leave unreachable rows behind.
        let mut near = ToolReport::text("x".repeat(17000));
        ctx.max_output_bytes = 17001;
        for _ in 0..260 {
            retain("shell", &mut near, &ctx);
        }
        assert_eq!(near.output.artifacts().count(), 0);
        ctx.max_output_bytes = 64000;
        let mut report = ToolReport::command(
            format!("{}failed:test_one{}", "a".repeat(9000), "z".repeat(9000)),
            Some(1),
            true,
        );
        retain("shell", &mut report, &ctx);
        let id = report.output.artifacts().next().unwrap().id;
        let tool = TodoTool::new(Arc::new(TodoStore::new(
            root.path().join("todo.json"),
            TodoScope::workspace(root.path().into()),
        )));
        let item: Value = serde_json::from_str(
            &tool
                .execute(json!({"action":"create","title":"Verify failure"}), &ctx)
                .await
                .unwrap(),
        )
        .unwrap();
        let todo = item["id"].clone();
        let linked:Value=serde_json::from_str(&tool.execute(json!({"action":"evidence","id":todo,"text":"test_one failed; not fixed","sources":[{"id":id,"start":9000,"end":9015}]}),&ctx).await.unwrap()).unwrap();
        assert_eq!(linked["evidence"][0]["sources"][0]["capture"], "incomplete");
        assert_eq!(linked["status"], "pending");
        assert!(tool.execute(json!({"action":"evidence","id":todo,"text":"invalid","sources":[{"id":id,"start":999999,"end":1000000}]}),&ctx).await.is_err());
        let restarted = TodoTool::new(Arc::new(TodoStore::new(
            root.path().join("todo.json"),
            TodoScope::workspace(root.path().into()),
        )));
        assert!(
            restarted
                .execute(json!({"action":"list"}), &ctx)
                .await
                .unwrap()
                .contains(&id.to_string())
        );
        let (metadata, _) = store.get(id).unwrap();
        let branch = root.path().join("branch");
        let mut dest = Store::open(&branch, Uuid::new_v4()).unwrap();
        store.copy_to(&mut dest, &metadata).unwrap();
        assert_eq!(dest.get(id).unwrap().0, metadata);
        // Unavailable references do not become successful empty reads.
        drop(store);
        std::fs::remove_dir_all(&scope.directory).unwrap();
        assert!(
            ResultTool
                .execute(json!({"action":"read","id":id}), &ctx)
                .await
                .is_err()
        );
    }

    /// Offline fixed-workflow benchmark: counts all model-visible retrieval pages,
    /// not only the first preview. It is NOT a provider-token or cache experiment.
    #[cfg(unix)]
    #[test]
    fn representative_retrieval_benchmark() {
        use crate::{
            context::WorkingContext,
            model::{Message, Role},
            tools::reliability_tests::context,
        };
        let mut rows = Vec::new();
        for (label, pad, replays) in [
            ("short", 100, 1),
            ("coding", 24000, 8),
            ("prose", 18000, 6),
            ("structured", 32000, 5),
            ("recovery", 20000, 7),
            ("long_branch", 48000, 10),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut ctx = context(root.path());
            ctx.max_output_bytes = 256000;
            ctx.artifact_scope = Some(crate::artifacts::Scope {
                directory: root.path().join("a"),
                session: Uuid::new_v4(),
            });
            let text = format!(
                "{}EXACT_SHA=0123 failed:test_hidden refusal{}",
                "x".repeat(pad),
                "y".repeat(pad)
            );
            let mut report = ToolReport::command(text.clone(), Some(1), false);
            retain("fixture", &mut report, &ctx);
            let mut m = Message::new(Role::Tool, report.output.text_fallback());
            m.tool_output = Some(Box::new(report.output.clone()));
            m.tool_outcome = Some(report.outcome.clone());
            let canonical = vec![m];
            let mut working = WorkingContext::default();
            working.prepare(&canonical).unwrap();
            let projected = working.project(&canonical).unwrap();
            let preview = projected[0].content.len();
            let mut retrieved = 0;
            let mut calls = 0;
            let mut found = pad < 8192;
            if let Some(artifact) = report.output.artifacts().next() {
                let scope = ctx.artifact_scope.as_ref().unwrap();
                let (_, bytes) = Store::open(&scope.directory, scope.session)
                    .unwrap()
                    .get(artifact.id)
                    .unwrap();
                let e: Evidence = serde_json::from_slice(&bytes).unwrap();
                let mut offset = 0;
                loop {
                    let v =
                        query(&e, &args(Action::Search, offset, 4096, Some("EXACT_SHA="))).unwrap();
                    calls += 1;
                    retrieved += serde_json::to_vec(&v).unwrap().len();
                    if let Some(hit) = v["matches"].as_array().unwrap().first() {
                        let v = query(
                            &e,
                            &args(
                                Action::Read,
                                hit["start"].as_u64().unwrap() as usize,
                                64,
                                None,
                            ),
                        )
                        .unwrap();
                        calls += 1;
                        retrieved += serde_json::to_vec(&v).unwrap().len();
                        found = v["text"]
                            .as_str()
                            .unwrap()
                            .contains("EXACT_SHA=0123 failed:test_hidden refusal");
                        break;
                    }
                    offset = v["next_offset"].as_u64().expect("must find marker") as usize;
                }
            }
            assert!(found);
            let baseline = text.len() * replays;
            // Conservative replay of every retrieved page for each remaining request.
            let treatment = (preview + retrieved) * replays;
            if label == "short" {
                assert!(treatment <= baseline);
            } else {
                assert!(treatment < baseline);
            }
            rows.push(json!({"stratum":label,"baseline_text_bytes":baseline,"treatment_text_bytes":treatment,"retrieval_calls":calls,"exact_recovery":found}));
        }
        println!(
            "ISSUE282_BENCHMARK={}",
            json!({"version":1,"measurement":"offline model-visible text bytes; not provider tokens or cache savings","rows":rows})
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn shell_capture_is_independent_of_preview_budget() {
        use crate::tools::{ToolRegistry, reliability_tests::context};
        let root = tempfile::tempdir().unwrap();
        let mut ctx = context(root.path());
        ctx.max_output_bytes = 4096;
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("a"),
            session: Uuid::new_v4(),
        });
        let registry = ToolRegistry::standard();
        let report = registry
            .execute_report_with_workflow_secrets(
                "shell",
                json!({"command":"printf '%20000s' x; printf 'TAIL_SHA=1234'"}),
                &ctx,
                None,
            )
            .await
            .unwrap();
        let id = report.output.artifacts().next().unwrap().id;
        let v=ResultTool.execute(json!({"action":"search","id":id,"offset":19000,"limit":2000,"query":"TAIL_SHA=1234"}),&ctx).await.unwrap();
        assert!(
            !serde_json::from_str::<Value>(&v).unwrap()["matches"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
