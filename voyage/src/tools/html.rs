//! Explicit session-owned visual publication. HTML is untrusted viewer content.
use super::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use voyage_protocol::tool_result::{ToolContent, ToolOutput};

pub const MAX_HTML: usize = 128 * 1024;
pub struct HtmlRender;
pub struct HtmlPreview(pub Arc<crate::host_browser::HostBrowser>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderArgs {
    html: String,
    title: String,
    height: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewArgs {
    html: String,
    #[serde(default = "width")]
    width: u32,
    #[serde(default = "appearance")]
    appearance: String,
}
fn width() -> u32 {
    640
}
fn appearance() -> String {
    "dark".into()
}
fn publication_id(ctx: &ToolContext, kind: &str) -> Result<uuid::Uuid, ToolError> {
    use sha2::{Digest, Sha256};
    let call = ctx
        .tool_call_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ToolError::Denied("visual replies require a canonical tool call".into()))?;
    let digest = Sha256::digest(
        serde_json::to_vec(&(ctx.execution_id, call, kind))
            .map_err(|_| ToolError::Failed("publication identity unavailable".into()))?,
    );
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid::Uuid::from_bytes(bytes))
}
fn document(html: &str, ctx: &ToolContext) -> Result<String, ToolError> {
    if html.trim().is_empty() || html.len() > MAX_HTML {
        return Err(ToolError::InvalidArguments(
            "HTML must be nonempty and at most 128 KiB UTF-8".into(),
        ));
    }
    if ctx.cancellation.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    ctx.policy
        .check_execution_authority()
        .map_err(|_| ToolError::Denied("execution authority unavailable".into()))?;
    if ctx.artifact_scope.is_none() {
        return Err(ToolError::Denied(
            "visual replies require a session-owned artifact store".into(),
        ));
    }
    Ok(ctx.redactor.redact(html))
}
fn schema(fields: Value, required: Vec<&str>) -> Value {
    json!({"type":"object","additionalProperties":false,"properties":fields,"required":required})
}
#[async_trait]
impl Tool for HtmlRender {
    fn definition(&self) -> crate::model::ToolDefinition {
        crate::model::ToolDefinition {name:"html_render".into(),description:"Publish a finished interactive HTML page inline in this conversation before your final reply. Use html_preview first to inspect the screenshot and fix errors. Supply a self-contained document with inline styles/scripts and data images; external resources are unavailable. The reader sees the page immediately, so add only information the page does not convey. Use responsive layout and theme variables --background, --foreground, --muted, --muted-foreground, --border, --primary, --primary-foreground, --card and --card-foreground. It is an isolated page, not an app/session/tool bridge. Ordinary code fences never publish a page.".into(),input_schema:schema(json!({"html":{"type":"string","minLength":1,"maxLength":MAX_HTML},"title":{"type":"string","minLength":1,"maxLength":100},"height":{"type":"integer","minimum":180,"maximum":640}}),vec!["html","title","height"]),output_schema:None,annotations:None}
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        Ok(self.execute_output(args, ctx).await?.text_fallback())
    }
    async fn execute_output(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: RenderArgs = serde_json::from_value(args)
            .map_err(|_| ToolError::InvalidArguments("invalid HTML render arguments".into()))?;
        let html = document(&args.html, ctx)?;
        if !(180..=640).contains(&args.height)
            || args.title.trim().is_empty()
            || args.title.chars().count() > 100
            || args.title.chars().any(char::is_control)
        {
            return Err(ToolError::InvalidArguments(
                "invalid visual title or height".into(),
            ));
        }
        let id = publication_id(ctx, "html_render")?;
        let scope = ctx.artifact_scope.as_ref().unwrap();
        let artifact = crate::artifacts::Store::open(&scope.directory, scope.session)
            .and_then(|mut store| store.put(id, "visual-reply.html", "text/html", html.as_bytes()))
            .map_err(|_| ToolError::Failed("HTML artifact publication refused".into()))?;
        let mut output = ToolOutput::text(
            "Visual reply published. The reader can interact with it directly in chat.",
        );
        output.structured_content = Some(
            json!({"htmlRender":{"version":1,"artifact":artifact,"title":ctx.redactor.redact(args.title.trim()),"height":args.height}}),
        );
        output.content.push(ToolContent::Resource {
            uri: format!("artifact:{}", artifact.id),
            mime_type: Some("text/html".into()),
            text: None,
            artifact: Some(artifact),
        });
        Ok(output)
    }
}
#[async_trait]
impl Tool for HtmlPreview {
    fn definition(&self) -> crate::model::ToolDefinition {
        crate::model::ToolDefinition {name:"html_preview".into(),description:"Check a self-contained HTML page in an isolated headless preview context before html_render. Returns a PNG screenshot, contentHeight, capturedHeight and bounded console messages. No task-browser cookies, website state, local files or network resources are available. Inline scripts/styles and data images work. Width 240–1600; appearance dark or light. Browser distribution must already be provisioned on the executing host; this tool never installs it.".into(),input_schema:schema(json!({"html":{"type":"string","minLength":1,"maxLength":MAX_HTML},"width":{"type":"integer","minimum":240,"maximum":1600},"appearance":{"enum":["dark","light"]}}),vec!["html"]),output_schema:None,annotations:None}
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, ToolError> {
        Ok(self.execute_output(args, ctx).await?.text_fallback())
    }
    async fn execute_output(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: PreviewArgs = serde_json::from_value(args)
            .map_err(|_| ToolError::InvalidArguments("invalid HTML preview arguments".into()))?;
        let html = document(&args.html, ctx)?;
        if !(240..=1600).contains(&args.width)
            || !matches!(args.appearance.as_str(), "dark" | "light")
        {
            return Err(ToolError::InvalidArguments(
                "invalid preview width or appearance".into(),
            ));
        }
        if ctx.policy.sandbox().settings.mode == crate::sandbox::Mode::Required {
            return Err(ToolError::Denied(
                "host preview does not support required OS sandbox policy".into(),
            ));
        }
        let value=self.0.agent(publication_id(ctx,"html_preview")?,json!({"kind":"html_preview","html":html,"width":args.width,"appearance":args.appearance}),ctx).await.map_err(|_|ToolError::Failed("HTML preview unavailable or outcome unknown; do not replay uncertain effects".into()))?;
        let bytes = value["data_base64"]
            .as_str()
            .ok_or_else(|| ToolError::Failed("preview screenshot unavailable".into()))?;
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(bytes)
            .map_err(|_| ToolError::Failed("invalid preview screenshot".into()))?;
        crate::images::validate(&bytes)
            .map_err(|_| ToolError::Failed("invalid preview raster".into()))?;
        let scope = ctx.artifact_scope.as_ref().unwrap();
        let artifact = crate::artifacts::Store::open(&scope.directory, scope.session)
            .and_then(|mut store| {
                store.put(
                    publication_id(ctx, "html_preview")
                        .map_err(|_| anyhow::anyhow!("preview identity"))?,
                    "html-preview.png",
                    "image/png",
                    &bytes,
                )
            })
            .map_err(|_| ToolError::Failed("preview screenshot storage refused".into()))?;
        let mut output = ToolOutput::text(
            "Inspect the screenshot and console messages before publishing with html_render.",
        );
        output.content.push(ToolContent::Image { artifact });
        output.structured_content = Some(
            json!({"width":value["width"],"contentHeight":value["contentHeight"],"capturedHeight":value["capturedHeight"],"consoleMessages":value["consoleMessages"]}),
        );
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn publication_is_explicit_redacted_immutable_and_session_scoped() {
        let root = tempfile::tempdir().unwrap();
        let mut ctx = crate::tools::reliability_tests::context(root.path());
        ctx.tool_call_id = Some("call-fixture".into());
        let args = json!({"html":"<button>fixture-secret</button>","title":"Mock","height":320});
        assert!(HtmlRender.execute_output(args.clone(), &ctx).await.is_err());
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("artifacts"),
            session: uuid::Uuid::new_v4(),
        });
        ctx.redactor = Arc::new(super::super::Redactor::new(["fixture-secret".into()]));
        let output = HtmlRender.execute_output(args.clone(), &ctx).await.unwrap();
        let artifact = output.artifacts().next().unwrap();
        let scope = ctx.artifact_scope.as_ref().unwrap();
        let store = crate::artifacts::Store::open(&scope.directory, scope.session).unwrap();
        let bytes = store.resolve(artifact).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("fixture-secret"));
        assert_eq!(
            output.structured_content.as_ref().unwrap()["htmlRender"]["height"],
            320
        );
        assert_eq!(HtmlRender.execute_output(args, &ctx).await.unwrap(), output);
        assert!(
            HtmlRender
                .execute_output(
                    json!({"html":"different","title":"Mock","height":320}),
                    &ctx
                )
                .await
                .is_err()
        );
        let foreign = crate::artifacts::Store::open(&scope.directory, uuid::Uuid::new_v4());
        assert!(foreign.is_err() || foreign.unwrap().resolve(artifact).is_err());
        let cancelled = ctx.clone();
        cancelled.cancellation.cancel();
        assert!(matches!(
            HtmlRender
                .execute_output(
                    json!({"html":"<p>x</p>","title":"Mock","height":320}),
                    &cancelled
                )
                .await,
            Err(ToolError::Cancelled)
        ));
    }
    #[tokio::test]
    async fn publication_rejects_invalid_bounds_without_storing_bytes() {
        let root = tempfile::tempdir().unwrap();
        let mut ctx = crate::tools::reliability_tests::context(root.path());
        ctx.tool_call_id = Some("call-fixture".into());
        ctx.artifact_scope = Some(crate::artifacts::Scope {
            directory: root.path().join("artifacts"),
            session: uuid::Uuid::new_v4(),
        });
        for args in [
            json!({"html":"é".repeat(MAX_HTML),"title":"x","height":320}),
            json!({"html":" ","title":"x","height":320}),
            json!({"html":"x","title":"x","height":641}),
            json!({"html":"x","title":"\n","height":320}),
            json!({"html":"x","title":"x","height":320,"session":"other"}),
        ] {
            assert!(HtmlRender.execute_output(args, &ctx).await.is_err());
        }
        assert!(!ctx.artifact_scope.unwrap().directory.exists());
    }
}
