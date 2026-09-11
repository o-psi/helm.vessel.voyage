//! Native image inputs. Runtime hydration must authorize references and verify
//! decoded format/dimensions before these encoders see bytes.
//!
//! Wire references:
//! https://platform.openai.com/docs/guides/images-vision
//! https://platform.openai.com/docs/api-reference/chat/create
//! https://docs.anthropic.com/en/docs/build-with-claude/vision
use super::{ModelInfo, Provider, ProviderError, ProviderStream};
use crate::{
    config::ProviderKind,
    model::{Message, ModelRequest, Role},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    io::{self, Write},
};
use voyage_protocol::content::ImageAttachment;
use voyage_protocol::content::{ContentLimits, ContentPart, ImageMediaType, validate_content};
use voyage_protocol::tool_result::{ArtifactReference, ToolContent};

pub(crate) const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
pub(crate) enum Wire {
    Chat,
    Responses,
    Anthropic,
}
fn invalid(message: &'static str) -> ProviderError {
    ProviderError::Request(message.into())
}
pub(crate) fn has_images(request: &ModelRequest) -> bool {
    request.messages.iter().any(|m| {
        tool_images(m)
            || !m.image_data.is_empty()
            || m.parts
                .iter()
                .any(|p| matches!(p, ContentPart::Image { .. }))
    })
}
pub(crate) fn tool_images(message: &Message) -> bool {
    message.tool_output.as_ref().is_some_and(|o| {
        o.content
            .iter()
            .any(|p| matches!(p, ToolContent::Image { .. }))
    })
}
/// Artifact MIME declarations are not proof: verify the hash and real bounded raster.
pub(crate) fn tool_image(
    artifact: &ArtifactReference,
    bytes: &[u8],
) -> Result<ImageAttachment, ProviderError> {
    let (media_type, width, height) = crate::images::validate(bytes)
        .map_err(|_| invalid("tool image is not a supported bounded raster"))?;
    if media_type_name(media_type) != artifact.mime_type
        || bytes.len() as u64 != artifact.byte_size
        || hex::encode(Sha256::digest(bytes)) != artifact.sha256
    {
        return Err(invalid("tool image does not match artifact metadata"));
    }
    Ok(ImageAttachment {
        id: artifact.id,
        sha256: artifact.sha256.clone(),
        name: artifact.name.clone(),
        media_type,
        byte_size: artifact.byte_size,
        width,
        height,
    })
}
fn parts(message: &Message) -> Result<std::borrow::Cow<'_, [ContentPart]>, ProviderError> {
    if !tool_images(message) {
        return Ok(std::borrow::Cow::Borrowed(&message.parts));
    }
    if message.role != Role::Tool
        || !message.parts.is_empty()
        || message.tool_call_id.as_ref().is_none_or(|id| id.is_empty())
        || !message.tool_calls.is_empty()
        || message.provider_state.is_some()
    {
        return Err(invalid(
            "visual tool output requires original tool-call provenance",
        ));
    }
    let output = message.tool_output.as_ref().unwrap();
    if output
        .content
        .len()
        .saturating_add(usize::from(output.structured_content.is_some()))
        > 16
    {
        return Err(invalid("visual tool output exceeds sixteen content parts"));
    }
    let mut count = 0usize;
    let mut size = 0u64;
    for part in &output.content {
        if let ToolContent::Image { artifact } = part {
            count += 1;
            size = size
                .checked_add(artifact.byte_size)
                .ok_or_else(|| invalid("tool image size overflow"))?;
            if count > 4 || size > MAX_IMAGE_BYTES as u64 {
                return Err(invalid(
                    "visual tool output exceeds four images or aggregate 2 MiB limit",
                ));
            }
        }
    }
    if output.text_fallback() != message.content {
        return Err(invalid("tool result text integrity failure"));
    }
    let mut result = Vec::new();
    for part in &output.content {
        result.push(match part {
            ToolContent::Text { text } => ContentPart::Text { text: text.clone() },
            ToolContent::Image { artifact } => ContentPart::Image {
                attachment: tool_image(
                    artifact,
                    message
                        .image_data
                        .get(&artifact.id)
                        .ok_or_else(|| invalid("tool image unavailable in authorized session"))?,
                )?,
            },
            other => ContentPart::Text {
                text: serde_json::to_string(other).map_err(|_| invalid("invalid tool content"))?,
            },
        });
    }
    if let Some(value) = &output.structured_content {
        result.push(ContentPart::Text {
            text: json!({"structuredContent":value}).to_string(),
        });
    }
    Ok(std::borrow::Cow::Owned(result))
}
fn builtin_images(provider: &ProviderKind, model: &str) -> bool {
    match provider {
        ProviderKind::OpenaiChat | ProviderKind::OpenaiResponses | ProviderKind::ChatGptOauth => {
            matches!(
                model,
                "gpt-5"
                    | "gpt-5-mini"
                    | "gpt-5-nano"
                    | "gpt-5-2025-08-07"
                    | "gpt-5-mini-2025-08-07"
                    | "gpt-5-nano-2025-08-07"
                    | "gpt-4o"
                    | "gpt-4o-2024-05-13"
                    | "gpt-4o-2024-08-06"
                    | "gpt-4o-2024-11-20"
                    | "gpt-4o-mini"
                    | "gpt-4o-mini-2024-07-18"
                    | "gpt-4.1"
                    | "gpt-4.1-2025-04-14"
                    | "gpt-4.1-mini"
                    | "gpt-4.1-mini-2025-04-14"
                    | "gpt-4.1-nano"
                    | "gpt-4.1-nano-2025-04-14"
            )
        }
        ProviderKind::Anthropic => matches!(
            model,
            "claude-3-haiku-20240307"
                | "claude-3-sonnet-20240229"
                | "claude-3-opus-20240229"
                | "claude-3-5-sonnet-20240620"
                | "claude-3-5-sonnet-20241022"
                | "claude-3-5-haiku-20241022"
                | "claude-3-7-sonnet-20250219"
                | "claude-sonnet-4-0"
                | "claude-sonnet-4-20250514"
                | "claude-opus-4-20250514"
        ),
    }
}
/// Selected endpoint/model metadata overrides the conservative exact-ID fallback.
/// Empty modalities are unknown; nonempty modalities (including text-only) are
/// authoritative. A record for a different model is never a capability grant.
pub fn validate_image_capability(
    provider: &ProviderKind,
    model: &str,
    known: Option<&ModelInfo>,
) -> Result<(), ProviderError> {
    let supported = match known {
        Some(info) if info.id != model => false,
        Some(info) if !info.input_modalities.is_empty() => {
            info.input_modalities.iter().any(|m| m == "image")
        }
        _ => builtin_images(provider, model),
    };
    if supported {
        Ok(())
    } else {
        Err(invalid(
            "selected model has no known image input support; select an image-capable model or remove attachments",
        ))
    }
}
/// Refresh endpoint-scoped discovery at image dispatch. Discovery absence/errors
/// can only fall back to the narrow built-in list, never enable an unknown model.
pub(crate) async fn preflight(
    provider: &impl Provider,
    kind: &ProviderKind,
    request: &ModelRequest,
) -> Result<(), ProviderError> {
    validate_adapter(kind, request)?;
    validate_request(request)?;
    if has_images(request) {
        // Do not let a metadata lookup hold image dispatch indefinitely. Malformed
        // metadata is not absence and must not downgrade to the built-in list.
        let models =
            match tokio::time::timeout(std::time::Duration::from_secs(10), provider.models()).await
            {
                Ok(Ok(models)) => Some(models),
                Ok(Err(error @ ProviderError::InvalidResponse(_))) => return Err(redact(error)),
                _ => None,
            };
        let known = models
            .as_ref()
            .and_then(|models| models.iter().find(|m| m.id == request.model));
        validate_image_capability(kind, &request.model, known)?;
    }
    Ok(())
}
pub(crate) fn validate_adapter(
    kind: &ProviderKind,
    request: &ModelRequest,
) -> Result<(), ProviderError> {
    // These are local compatibility refusals, not provider diagnostics. Do not
    // silently demote images to metadata or relabel tool output as authored User input.
    if request.messages.iter().any(tool_images) && matches!(kind, ProviderKind::OpenaiChat) {
        return Err(invalid(
            "selected adapter does not support visual tool outputs with tool-call provenance; use native Responses, ChatGPT OAuth or Anthropic",
        ));
    }
    Ok(())
}
/// Counts image occurrences across the entire request, including repeated UUIDs.
pub(crate) fn validate_request(request: &ModelRequest) -> Result<(), ProviderError> {
    let mut total = 0usize;
    let mut images = 0usize;
    for message in &request.messages {
        validate_message(message)?;
        for part in parts(message)?.iter() {
            if let ContentPart::Image { attachment } = part {
                images += 1;
                if images > 4 {
                    return Err(invalid(
                        "request exceeds four image occurrences; compact older image turns",
                    ));
                }
                let bytes = message
                    .image_data
                    .get(&attachment.id)
                    .ok_or_else(|| invalid("image data unavailable; attach the image again"))?;
                total = total
                    .checked_add(bytes.len())
                    .ok_or_else(|| invalid("request images exceed aggregate 2 MiB limit"))?;
                if total > MAX_IMAGE_BYTES {
                    return Err(invalid("request images exceed aggregate 2 MiB limit"));
                }
            }
        }
    }
    Ok(())
}
fn validate_message(message: &Message) -> Result<(), ProviderError> {
    let parts = parts(message)?;
    if parts.is_empty() {
        return if message.image_data.is_empty() {
            Ok(())
        } else {
            Err(invalid("unreferenced image data in provider request"))
        };
    }
    if !tool_images(message)
        && (message.role != Role::User
            || message.tool_call_id.is_some()
            || !message.tool_calls.is_empty()
            || message.provider_state.is_some())
    {
        return Err(invalid(
            "structured content is supported only on ordinary user messages",
        ));
    }
    validate_content(
        &parts,
        ContentLimits {
            max_parts: 16,
            max_text_bytes: 64 * 1024,
            max_images: 4,
            max_image_bytes: MAX_IMAGE_BYTES as u64,
            max_total_image_bytes: MAX_IMAGE_BYTES as u64,
            max_dimension: 8192,
            max_pixels: 16 * 1024 * 1024,
        },
        true,
    )
    .map_err(|e| ProviderError::Request(e.to_string()))?;
    for part in parts.iter() {
        if let ContentPart::Image { attachment } = part {
            let bytes = message
                .image_data
                .get(&attachment.id)
                .ok_or_else(|| invalid("image data unavailable; attach the image again"))?;
            if bytes.is_empty()
                || bytes.len() > MAX_IMAGE_BYTES
                || bytes.len() as u64 != attachment.byte_size
            {
                return Err(invalid("image data does not match validated metadata"));
            }
            let (media, width, height) = crate::images::validate(bytes)
                .map_err(|_| invalid("image data is not a supported bounded raster"))?;
            if media != attachment.media_type
                || width != attachment.width
                || height != attachment.height
                || hex::encode(Sha256::digest(bytes)) != attachment.sha256
            {
                return Err(invalid("image raster or hash does not match metadata"));
            }
        }
    }
    for id in message.image_data.keys() {
        if !parts
            .iter()
            .any(|p| matches!(p, ContentPart::Image { attachment } if attachment.id == *id))
        {
            return Err(invalid("unreferenced image data in provider request"));
        }
    }
    Ok(())
}
fn media_type_name(value: ImageMediaType) -> &'static str {
    match value {
        ImageMediaType::Png => "image/png",
        ImageMediaType::Jpeg => "image/jpeg",
        ImageMediaType::WebP => "image/webp",
    }
}
/// Nonempty parts are canonical: never prepend the legacy content projection.
pub(crate) fn content(message: &Message, wire: Wire) -> Result<Option<Value>, ProviderError> {
    validate_message(message)?;
    if tool_images(message) && matches!(wire, Wire::Chat) {
        return Err(invalid(
            "OpenAI Chat tool messages do not support images; original tool-call provenance retained",
        ));
    }
    let parts = parts(message)?;
    if parts.is_empty() {
        return Ok(None);
    }
    let mut blocks = Vec::with_capacity(parts.len());
    for part in parts.iter() {
        blocks.push(match part {
            ContentPart::Text { text } => match wire {
                Wire::Chat | Wire::Anthropic => json!({"type":"text", "text":text}),
                Wire::Responses => json!({"type":"input_text", "text":text}),
            },
            ContentPart::Image { attachment } => {
                let bytes = message.image_data.get(&attachment.id).ok_or_else(|| invalid("image data unavailable; attach the image again"))?;
                let mime = media_type_name(attachment.media_type);
                let data = STANDARD.encode(bytes);
                match wire {
                    Wire::Chat => json!({"type":"image_url", "image_url":{"url":format!("data:{mime};base64,{data}"), "detail":"auto"}}),
                    Wire::Responses => json!({"type":"input_image", "image_url":format!("data:{mime};base64,{data}"), "detail":"auto"}),
                    Wire::Anthropic => json!({"type":"image", "source":{"type":"base64", "media_type":mime, "data":data}}),
                }
            }
        });
    }
    Ok(Some(Value::Array(blocks)))
}
/// Count final serialized JSON, including escaping, tools and OAuth mutations.
/// reqwest .json uses this same serde_json encoding. No second full allocation.
pub(crate) fn check_body(body: &Value) -> Result<(), ProviderError> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let size = self
                .0
                .checked_add(buf.len())
                .filter(|n| *n <= MAX_REQUEST_BYTES)
                .ok_or_else(|| io::Error::other("request size limit"))?;
            self.0 = size;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), body)
        .map_err(|_| invalid("encoded provider request exceeds 8 MiB limit"))
}
pub(crate) fn redact(error: ProviderError) -> ProviderError {
    const MESSAGE: &str = "image-bearing provider request failed; provider diagnostic omitted";
    match error {
        ProviderError::Authentication(_) => ProviderError::Authentication(MESSAGE.into()),
        ProviderError::UsageLimit => ProviderError::UsageLimit,
        ProviderError::ContextLength => ProviderError::ContextLength,
        ProviderError::Incomplete => ProviderError::Incomplete,
        ProviderError::RetryAfter { source, delay } => ProviderError::RetryAfter {
            source: Box::new(redact(*source)),
            delay,
        },
        ProviderError::RateLimit { retry_after, .. } => ProviderError::RateLimit {
            message: MESSAGE.into(),
            retry_after,
        },
        ProviderError::Unavailable(_) => ProviderError::Unavailable(MESSAGE.into()),
        ProviderError::Timeout(_) => ProviderError::Timeout(MESSAGE.into()),
        ProviderError::Request(_) => ProviderError::Request(MESSAGE.into()),
        ProviderError::InvalidResponse(_) => ProviderError::InvalidResponse(MESSAGE.into()),
    }
}
pub(crate) async fn guard<T>(
    images: bool,
    operation: impl Future<Output = Result<T, ProviderError>>,
) -> Result<T, ProviderError> {
    operation
        .await
        .map_err(|e| if images { redact(e) } else { e })
}
pub(crate) fn guard_stream(images: bool, stream: ProviderStream) -> ProviderStream {
    if images {
        Box::pin(stream.map(|item| item.map_err(redact)))
    } else {
        stream
    }
}
pub(crate) fn discovered_modalities(item: &Value) -> Result<Vec<String>, ProviderError> {
    let Some(value) = item.get("input_modalities") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .filter(|v| v.len() <= 16)
        .ok_or_else(|| ProviderError::InvalidResponse("invalid model input modalities".into()))?;
    values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 32
                        && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                })
                .map(str::to_owned)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("invalid model input modalities".into())
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use voyage_protocol::content::ImageAttachment;

    fn image_message(_size: usize) -> Message {
        image_message_format(image::ImageFormat::Png)
    }
    fn image_message_format(format: image::ImageFormat) -> Message {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut buffer, format)
            .unwrap();
        let bytes = buffer.into_inner();
        let (media_type, width, height) = crate::images::validate(&bytes).unwrap();
        let id = Uuid::from_u128(1);
        let mut message = Message::new(Role::User, "legacy projection");
        message.parts = vec![ContentPart::Image {
            attachment: ImageAttachment {
                id,
                sha256: hex::encode(Sha256::digest(&bytes)),
                name: "fixture".into(),
                media_type,
                byte_size: bytes.len() as u64,
                width,
                height,
            },
        }];
        message.image_data.insert(id, bytes);
        message
    }
    fn request(messages: Vec<Message>) -> ModelRequest {
        ModelRequest {
            model: "gpt-4o".into(),
            messages,
            tools: vec![],
            temperature: None,
            reasoning_effort: None,
            service_tier: None,
            max_tokens: None,
        }
    }
    #[test]
    fn preserves_native_order_without_duplicate_projection() {
        let mut m = image_message(3);
        m.parts.insert(
            0,
            ContentPart::Text {
                text: "before".into(),
            },
        );
        m.parts.push(ContentPart::Text {
            text: "after".into(),
        });
        let data = STANDARD.encode(m.image_data.values().next().unwrap());
        let url = format!("data:image/png;base64,{data}");
        let chat = content(&m, Wire::Chat).unwrap().unwrap();
        assert_eq!(
            chat,
            json!([
                {"type":"text","text":"before"},
                {"type":"image_url","image_url":{"url":url,"detail":"auto"}},
                {"type":"text","text":"after"}
            ])
        );
        let responses = content(&m, Wire::Responses).unwrap().unwrap();
        assert_eq!(
            responses,
            json!([
                {"type":"input_text","text":"before"},
                {"type":"input_image","image_url":url,"detail":"auto"},
                {"type":"input_text","text":"after"}
            ])
        );
        let anthropic = content(&m, Wire::Anthropic).unwrap().unwrap();
        assert_eq!(
            anthropic,
            json!([
                {"type":"text","text":"before"},
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":data}},
                {"type":"text","text":"after"}
            ])
        );
    }
    #[test]
    fn image_only_and_all_media_types() {
        for (kind, mime) in [
            (ImageMediaType::Png, "image/png"),
            (ImageMediaType::Jpeg, "image/jpeg"),
            (ImageMediaType::WebP, "image/webp"),
        ] {
            let mut m = image_message_format(match kind {
                ImageMediaType::Png => image::ImageFormat::Png,
                ImageMediaType::Jpeg => image::ImageFormat::Jpeg,
                ImageMediaType::WebP => image::ImageFormat::WebP,
            });
            m.content.clear();
            let ContentPart::Image { attachment } = &mut m.parts[0] else {
                unreachable!()
            };
            attachment.media_type = kind;
            for wire in [Wire::Chat, Wire::Responses, Wire::Anthropic] {
                let value = content(&m, wire).unwrap().unwrap();
                assert_eq!(value.as_array().unwrap().len(), 1);
                assert!(value.to_string().contains(mime));
            }
        }
    }
    #[test]
    fn capability_is_selected_model_specific_and_fails_closed() {
        let p = ProviderKind::OpenaiResponses;
        assert!(validate_image_capability(&p, "gpt-4o", None).is_ok());
        assert!(validate_image_capability(&p, "unknown", None).is_err());
        assert!(validate_image_capability(&p, "gpt-4o-audio-preview", None).is_err());
        let mut known = ModelInfo::minimal("gpt-4o");
        assert!(validate_image_capability(&p, "gpt-4o", Some(&known)).is_err());
        known.input_modalities = discovered_modalities(&json!({"id":"gpt-4o"})).unwrap();
        assert!(validate_image_capability(&p, "gpt-4o", Some(&known)).is_ok());
        known.id = "custom".into();
        known.input_modalities = vec!["image".into()];
        assert!(validate_image_capability(&p, "custom", Some(&known)).is_ok());
        assert!(validate_image_capability(&p, "other", Some(&known)).is_err());
    }
    #[test]
    fn full_history_aggregate_counts_repeated_images() {
        let m = image_message(MAX_IMAGE_BYTES);
        assert!(validate_request(&request(vec![m.clone()])).is_ok());
        assert!(validate_request(&request(vec![m; 5])).is_err());
    }
    #[test]
    fn rejects_missing_mismatched_unreferenced_and_nonuser_images() {
        let mut m = image_message(3);
        m.image_data.clear();
        assert!(content(&m, Wire::Chat).is_err());
        let mut m = image_message(3);
        m.image_data.get_mut(&Uuid::from_u128(1)).unwrap().push(1);
        assert!(content(&m, Wire::Chat).is_err());
        let mut m = image_message(3);
        m.image_data.insert(Uuid::from_u128(2), vec![1]);
        assert!(content(&m, Wire::Chat).is_err());
        for role in [Role::Assistant, Role::Tool, Role::System] {
            let mut m = image_message(3);
            m.role = role;
            assert!(content(&m, Wire::Responses).is_err());
        }
    }
    #[test]
    fn visual_tool_blocks_keep_order_and_refuse_chat() {
        let image = image_message(3);
        let ContentPart::Image { attachment } = &image.parts[0] else {
            unreachable!()
        };
        let mut m = Message::tool("call-original", "");
        m.image_data = image.image_data.clone();
        let output = voyage_protocol::tool_result::ToolOutput {
            content: vec![
                ToolContent::Text {
                    text: "before".into(),
                },
                ToolContent::Image {
                    artifact: ArtifactReference {
                        id: attachment.id,
                        sha256: attachment.sha256.clone(),
                        name: attachment.name.clone(),
                        mime_type: "image/png".into(),
                        byte_size: attachment.byte_size,
                    },
                },
                ToolContent::Text {
                    text: "after".into(),
                },
            ],
            ..Default::default()
        };
        m.content = output.text_fallback();
        m.tool_output = Some(Box::new(output));
        assert!(has_images(&request(vec![m.clone()])));
        assert!(validate_request(&request(vec![m.clone()])).is_ok());
        for wire in [Wire::Responses, Wire::Anthropic] {
            let blocks = content(&m, wire).unwrap().unwrap();
            assert_eq!(blocks[0]["text"], "before");
            assert_eq!(blocks[2]["text"], "after");
            assert_eq!(blocks.as_array().unwrap().len(), 3);
        }
        assert!(content(&m, Wire::Chat).is_err());
        assert!(validate_adapter(&ProviderKind::ChatGptOauth, &request(vec![m.clone()])).is_ok());
        assert!(validate_adapter(&ProviderKind::OpenaiChat, &request(vec![m.clone()])).is_err());
        let mut assistant = Message::new(Role::Assistant, "");
        assistant.tool_calls.push(crate::model::ToolCall {
            id: "call-original".into(),
            name: "screenshot".into(),
            arguments: json!({}),
        });
        let body = super::super::openai_responses::request_body(
            request(vec![assistant, m.clone()]),
            false,
        )
        .unwrap();
        assert_eq!(body["input"][1]["type"], "function_call_output");
        assert_eq!(body["input"][1]["call_id"], "call-original");
        assert_eq!(body["input"][1]["output"][1]["type"], "input_image");
        assert!(
            super::super::openai_responses::request_body(request(vec![m.clone()]), false).is_err()
        );
        let canonical = serde_json::to_string(&m).unwrap();
        assert!(!canonical.contains("base64"));
        assert_eq!(m.role, Role::Tool);
        assert_eq!(m.tool_call_id.as_deref(), Some("call-original"));
        m.image_data.values_mut().next().unwrap()[0] ^= 1;
        assert!(content(&m, Wire::Responses).is_err());
    }
    #[test]
    fn encoded_limit_counts_whole_body_and_escaping() {
        // Include tools, instructions and input, not only the content array.
        let mut body = json!({"model":"gpt-4o","instructions":"trusted","input":[],"tools":[{"description":""}]});
        let overhead = serde_json::to_vec(&body).unwrap().len();
        body["tools"][0]["description"] = json!("a".repeat(MAX_REQUEST_BYTES - overhead));
        assert_eq!(serde_json::to_vec(&body).unwrap().len(), MAX_REQUEST_BYTES);
        assert!(check_body(&body).is_ok());
        body["stream"] = json!(true);
        assert!(check_body(&body).is_err());
        assert!(check_body(&json!("\0".repeat(MAX_REQUEST_BYTES / 6 + 1))).is_err());
    }
    #[test]
    fn error_variants_never_retain_provider_text() {
        let secret = "data:image/png;base64,PRIVATE";
        let delay = std::time::Duration::from_secs(7);
        for error in [
            ProviderError::Authentication(secret.into()),
            ProviderError::Unavailable(secret.into()),
            ProviderError::Timeout(secret.into()),
            ProviderError::Request(secret.into()),
            ProviderError::InvalidResponse(secret.into()),
            ProviderError::RetryAfter {
                source: Box::new(ProviderError::Unavailable(secret.into())),
                delay,
            },
            ProviderError::RateLimit {
                message: secret.into(),
                retry_after: Some(delay),
            },
        ] {
            let retryable = error.is_retryable();
            let retry_after = error.retry_after();
            let error = redact(error);
            assert_eq!(error.is_retryable(), retryable);
            assert_eq!(error.retry_after(), retry_after);
            assert!(!format!("{error:?}").contains("PRIVATE"));
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }
    #[tokio::test]
    async fn stream_and_initial_error_paths_are_guarded() {
        let initial: Result<(), ProviderError> = guard(true, async {
            Err(ProviderError::InvalidResponse("PRIVATE".into()))
        })
        .await;
        assert!(!format!("{initial:?}").contains("PRIVATE"));
        let stream: ProviderStream = Box::pin(futures_util::stream::iter(vec![Err(
            ProviderError::Request("PRIVATE".into()),
        )]));
        let mut stream = guard_stream(true, stream);
        let result = stream.next().await.unwrap();
        assert!(!format!("{result:?}").contains("PRIVATE"));
    }
    #[test]
    fn malformed_discovery_is_not_absence() {
        assert!(discovered_modalities(&json!({})).unwrap().is_empty());
        assert_eq!(
            discovered_modalities(&json!({"input_modalities":["text"]})).unwrap(),
            vec!["text".to_string()]
        );
        assert!(discovered_modalities(&json!({"input_modalities":"image"})).is_err());
    }
}
