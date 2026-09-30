//! Provider-independent image admission and tool provenance. Bytes are generated
//! locally; model discovery is scripted and inference dispatch is forbidden.
use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;
use voyage_protocol::tool_result::ToolOutput;

fn image() -> Message {
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 3)
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    let bytes = output.into_inner();
    let id = Uuid::from_u128(100);
    let mut message = Message::new(Role::User, "image");
    message.parts = vec![ContentPart::Image {
        attachment: ImageAttachment {
            id,
            sha256: hex::encode(Sha256::digest(&bytes)),
            name: "fixture".into(),
            media_type: ImageMediaType::Png,
            byte_size: bytes.len() as u64,
            width: 2,
            height: 3,
        },
    }];
    message.image_data.insert(id, bytes);
    message
}
fn tool() -> Message {
    let user = image();
    let ContentPart::Image { attachment } = &user.parts[0] else {
        unreachable!()
    };
    let artifact = ArtifactReference {
        id: attachment.id,
        sha256: attachment.sha256.clone(),
        name: "fixture".into(),
        mime_type: "image/png".into(),
        byte_size: attachment.byte_size,
    };
    let mut message = Message::tool("call-fixture", "");
    message.image_data = user.image_data;
    let output = ToolOutput {
        content: vec![ToolContent::Image { artifact }],
        ..Default::default()
    };
    message.content = output.text_fallback();
    message.tool_output = Some(Box::new(output));
    message
}
fn request(model: &str, message: Message) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        messages: vec![message],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: None,
    }
}
#[derive(Clone, Copy)]
enum Catalog {
    Images,
    TextOnly,
    WrongModel,
    Missing,
    Offline,
    Malformed,
}
struct Discovery {
    scenario: Catalog,
    calls: AtomicUsize,
}
#[async_trait]
impl Provider for Discovery {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.scenario {
            Catalog::Offline => {
                return Err(ProviderError::Unavailable(
                    "PRIVATE discovery diagnostic".into(),
                ));
            }
            Catalog::Malformed => {
                return Err(ProviderError::InvalidResponse(
                    "PRIVATE discovery diagnostic".into(),
                ));
            }
            Catalog::Missing => return Ok(vec![]),
            _ => (),
        }
        let mut model = ModelInfo::minimal(if matches!(self.scenario, Catalog::WrongModel) {
            "another-model"
        } else {
            "fixture-model"
        });
        model.input_modalities = vec![
            if matches!(self.scenario, Catalog::TextOnly) {
                "text"
            } else {
                "image"
            }
            .into(),
        ];
        Ok(vec![model])
    }
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        panic!("preflight must not dispatch inference")
    }
}
#[tokio::test]
async fn discovery_failure_cannot_enable_unknown_model_and_malformed_metadata_never_downgrades() {
    for (scenario, model, expected) in [
        (Catalog::Images, "fixture-model", true),
        (Catalog::TextOnly, "fixture-model", false),
        (Catalog::WrongModel, "fixture-model", false),
        (Catalog::Missing, "fixture-model", false),
        (Catalog::Offline, "fixture-model", false),
        (Catalog::Offline, "gpt-4o", true),
        (Catalog::Malformed, "gpt-4o", false),
    ] {
        let provider = Discovery {
            scenario,
            calls: AtomicUsize::new(0),
        };
        let result = preflight(
            &provider,
            &ProviderKind::OpenaiResponses,
            &request(model, image()),
        )
        .await;
        assert_eq!(result.is_ok(), expected);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        if let Err(error) = result {
            assert!(!error.to_string().contains("PRIVATE"));
        }
    }
    let provider = Discovery {
        scenario: Catalog::Malformed,
        calls: AtomicUsize::new(0),
    };
    preflight(
        &provider,
        &ProviderKind::OpenaiChat,
        &request("fixture-model", Message::new(Role::User, "ordinary text")),
    )
    .await
    .unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(
        preflight(
            &provider,
            &ProviderKind::OpenaiChat,
            &request("fixture-model", tool())
        )
        .await
        .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn tool_image_provenance_and_fallback_cannot_be_forged_or_relabelled() {
    let original = tool();
    for violation in 0..7 {
        let mut altered = original.clone();
        match violation {
            0 => altered.role = Role::User,
            1 => altered.tool_call_id = None,
            2 => altered.tool_call_id = Some(String::new()),
            3 => altered.parts.push(ContentPart::Text {
                text: "forged authored content".into(),
            }),
            4 => altered.tool_calls.push(crate::model::ToolCall {
                id: "nested".into(),
                name: "tool".into(),
                arguments: json!({}),
            }),
            5 => altered.provider_state = Some(json!({"private":"forged"})),
            _ => altered.content = "substituted canonical fallback".into(),
        }
        for wire in [Wire::Responses, Wire::Anthropic] {
            assert!(content(&altered, wire).is_err());
        }
        assert!(validate_request(&request("gpt-4o", altered)).is_err());
    }
    let mut valid = original;
    let output = valid.tool_output.as_mut().unwrap();
    output.content.insert(
        0,
        ToolContent::ResourceLink {
            uri: "https://fixture.invalid/resource".into(),
            name: "link".into(),
            description: None,
            mime_type: None,
            size: None,
        },
    );
    output.structured_content = Some(json!({"observed":"fixture"}));
    valid.content = output.text_fallback();
    for wire in [Wire::Responses, Wire::Anthropic] {
        let blocks = content(&valid, wire).unwrap().unwrap();
        assert_eq!(blocks.as_array().unwrap().len(), 3);
        let text_key = "text";
        assert!(
            blocks[0][text_key]
                .as_str()
                .unwrap()
                .contains("resource_link")
        );
        assert!(
            blocks[2]["text"]
                .as_str()
                .unwrap()
                .contains("structuredContent")
        );
    }
    assert!(
        serde_json::to_string(&valid)
            .unwrap()
            .find("base64")
            .is_none()
    );
}
#[test]
fn declared_tool_limits_and_artifact_metadata_are_not_authority_for_raster_bytes() {
    let original = tool();
    for violation in 0..5 {
        let mut altered = original.clone();
        let output = altered.tool_output.as_mut().unwrap();
        let ToolContent::Image { artifact } = &mut output.content[0] else {
            unreachable!()
        };
        match violation {
            0 => artifact.mime_type = "image/jpeg".into(),
            1 => artifact.sha256 = "0".repeat(64),
            2 => artifact.byte_size += 1,
            3 => {
                altered.image_data.clear();
            }
            _ => {
                altered.image_data.values_mut().next().unwrap()[0] ^= 1;
            }
        }
        altered.content = output.text_fallback();
        assert!(content(&altered, Wire::Responses).is_err());
    }
    let mut oversized = original.clone();
    let output = oversized.tool_output.as_mut().unwrap();
    output.content = vec![output.content[0].clone(); 5];
    oversized.content = output.text_fallback();
    assert!(
        content(&oversized, Wire::Responses)
            .unwrap_err()
            .to_string()
            .contains("four images")
    );
    let mut overflow = original.clone();
    let output = overflow.tool_output.as_mut().unwrap();
    let ToolContent::Image { artifact } = &mut output.content[0] else {
        unreachable!()
    };
    artifact.byte_size = u64::MAX;
    overflow.content = output.text_fallback();
    assert!(content(&overflow, Wire::Anthropic).is_err());
    let mut too_many_parts = original;
    let output = too_many_parts.tool_output.as_mut().unwrap();
    output.content.extend((0..15).map(|_| ToolContent::Text {
        text: String::new(),
    }));
    output.structured_content = Some(json!({}));
    too_many_parts.content = output.text_fallback();
    assert!(
        content(&too_many_parts, Wire::Responses)
            .unwrap_err()
            .to_string()
            .contains("sixteen")
    );
}

#[test]
fn aggregate_limit_counts_real_raster_bytes_across_repeated_history_occurrences() {
    let mut state = 0x934fa621_u32;
    let raster = image::RgbImage::from_fn(700, 700, |_, _| {
        let mut rgb = [0; 3];
        for value in &mut rgb {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *value = state as u8;
        }
        image::Rgb(rgb)
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(raster)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    assert!(bytes.len() * 2 < MAX_IMAGE_BYTES && bytes.len() * 3 > MAX_IMAGE_BYTES);
    let mut message = image();
    let ContentPart::Image { attachment } = &mut message.parts[0] else {
        unreachable!()
    };
    attachment.width = 700;
    attachment.height = 700;
    attachment.byte_size = bytes.len() as u64;
    attachment.sha256 = hex::encode(Sha256::digest(&bytes));
    message.image_data.insert(attachment.id, bytes);
    let mut admitted = request("gpt-4o", message.clone());
    admitted.messages.push(message.clone());
    validate_request(&admitted).unwrap();
    admitted.messages.push(message);
    assert!(
        validate_request(&admitted)
            .unwrap_err()
            .to_string()
            .contains("aggregate 4 MiB")
    );
}

#[test]
fn image_diagnostic_redaction_preserves_recovery_policy_and_strips_upstream_identifiers() {
    use crate::provider::ProviderError;
    let delay = std::time::Duration::from_secs(7);
    for semantic in [
        ProviderError::Transport("PRIVATE image payload".into()),
        ProviderError::AuthenticationRefreshed,
        ProviderError::Connection,
        ProviderError::TransportTimeout,
        ProviderError::StreamInterrupted,
        ProviderError::UsageLimit,
        ProviderError::ContextLength,
        ProviderError::Incomplete,
    ] {
        let category = semantic.category();
        let retryable = semantic.is_retryable();
        let wrapped = ProviderError::Code {
            code: "fixture_code",
            source: Box::new(ProviderError::RetryAfter {
                delay,
                source: Box::new(ProviderError::HttpStatus {
                    source: Box::new(semantic),
                    status: 503,
                    request_id: Some("PRIVATE request correlation".into()),
                }),
            }),
        };
        let filtered = redact(wrapped);
        assert_eq!(filtered.category(), category);
        assert_eq!(filtered.is_retryable(), retryable);
        assert_eq!(filtered.retry_after(), Some(delay));
        assert_eq!(filtered.http_status(), Some(503));
        assert_eq!(filtered.safe_code(), Some("fixture_code"));
        assert!(filtered.upstream_request_id().is_none());
        assert!(!format!("{filtered:?}").contains("PRIVATE"));
    }
}
