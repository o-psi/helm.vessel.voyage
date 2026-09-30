//! Provider counting uses the actual input encoder. JSON is only an identity hash.
use super::ProviderError;
use crate::model::ModelRequest;
use serde_json::Value;
use sha2::{Digest, Sha256};
use voyage_protocol::context_accounting::{ContextScope, CountPrecision, RequestTokenCount};

pub(super) const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn scope(request: &ModelRequest, transport: &str, endpoint: Option<&str>) -> ContextScope {
    ContextScope {
        model: request.model.clone(),
        transport: transport.into(),
        endpoint_fingerprint: endpoint.map(|url| format!("{:x}", Sha256::digest(url.as_bytes()))),
        account: None,
    }
}
pub(crate) fn unknown(
    request: &ModelRequest,
    transport: &str,
    endpoint: Option<&str>,
    reason: &str,
) -> RequestTokenCount {
    RequestTokenCount {
        scope: scope(request, transport, endpoint),
        observed_at_ms: now_ms(),
        input_tokens: None,
        precision: CountPrecision::Unknown,
        method: "unavailable".into(),
        complete: false,
        input_fingerprint: None,
        limitations: vec![reason.into()],
    }
}
pub(super) fn counted(
    request: &ModelRequest,
    transport: &str,
    endpoint: &str,
    input: &Value,
    response: &Value,
    precision: CountPrecision,
    method: &str,
) -> Result<RequestTokenCount, ProviderError> {
    let tokens = response
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ProviderError::InvalidResponse("input token count missing or invalid".into())
        })?;
    let bytes = serde_json::to_vec(input)
        .map_err(|_| ProviderError::Request("input identity unavailable".into()))?;
    Ok(RequestTokenCount {
        scope: scope(request, transport, Some(endpoint)),
        observed_at_ms: now_ms(),
        input_tokens: Some(tokens),
        precision,
        method: method.into(),
        complete: true,
        input_fingerprint: Some(format!("{:x}", Sha256::digest(&bytes))),
        limitations: if precision == CountPrecision::ProviderEstimate {
            vec!["provider_count_may_differ_from_dispatch_usage".into()]
        } else {
            vec![]
        },
    })
}
/// Explicitly classify every encoder field. A future unclassified field must not
/// be silently removed and falsely described as complete accounting.
pub(super) fn responses_input(body: &Value) -> Option<Value> {
    let object = body.as_object()?;
    let input = [
        "model",
        "input",
        "instructions",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning",
        "text",
        "previous_response_id",
        "conversation",
        "truncation",
    ];
    let output = [
        "stream",
        "store",
        "include",
        "max_output_tokens",
        "service_tier",
        "temperature",
    ];
    if object
        .keys()
        .any(|key| !input.contains(&key.as_str()) && !output.contains(&key.as_str()))
    {
        return None;
    }
    Some(Value::Object(
        object
            .iter()
            .filter(|(key, _)| input.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}

/// Only explicit default metadata establishes the advertised default. A bare
/// context_window/max_context_window is retained as a maximum, never enabled.
pub(super) fn catalog_capacity(
    entry: &Value,
    model: &str,
    transport: &str,
    endpoint: &str,
) -> Option<voyage_protocol::context_accounting::ModelContextCapacity> {
    let value = |key: &str| entry.get(key).and_then(Value::as_u64).filter(|n| *n > 0);
    if [
        "default_context_window",
        "max_context_window",
        "context_window",
    ]
    .iter()
    .any(|key| {
        entry
            .get(*key)
            .is_some_and(|v| !v.is_null() && v.as_u64().is_none_or(|n| n == 0))
    }) {
        return None;
    }
    let default = value("default_context_window");
    let maximum = value("max_context_window").or_else(|| value("context_window"));
    if default.is_none() && maximum.is_none() {
        return None;
    }
    if default
        .zip(maximum)
        .is_some_and(|(default, maximum)| default > maximum)
    {
        return None;
    }
    Some(voyage_protocol::context_accounting::ModelContextCapacity {
        scope: ContextScope {
            model: model.into(),
            transport: transport.into(),
            endpoint_fingerprint: Some(format!("{:x}", Sha256::digest(endpoint.as_bytes()))),
            account: None,
        },
        observed_at_ms: now_ms(),
        source: "provider_model_catalog".into(),
        default_window_tokens: default,
        maximum_selectable_window_tokens: maximum,
        enabled_window_tokens: default,
        default_output_tokens: value("default_output_tokens"),
        maximum_output_tokens: value("max_output_tokens"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Message, Role, ToolDefinition};
    use crate::provider::{OpenAiResponsesProvider, Provider};
    use serde_json::json;
    fn request(text: &str) -> ModelRequest {
        ModelRequest {
            model: "fixture-model".into(),
            messages: vec![
                Message::new(Role::System, "Keep the earlier constraint"),
                Message::new(Role::User, text),
            ],
            tools: vec![ToolDefinition {
                name: "fixture_tool".into(),
                description: "schema is input".into(),
                input_schema: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
                output_schema: None,
                annotations: None,
            }],
            temperature: None,
            reasoning_effort: None,
            service_tier: None,
            max_tokens: Some(64),
        }
    }
    fn reply(tokens: u64) -> (u16, String) {
        (
            200,
            json!({"object":"response.input_tokens","input_tokens":tokens}).to_string(),
        )
    }
    fn body(raw: &str) -> Value {
        serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn api_tokens_are_not_calibrated_from_equal_or_different_bytes() {
        let (url, server) = crate::provider::native_http_tests::server(vec![
            reply(100),
            reply(900),
            reply(37),
            reply(37),
        ])
        .await;
        let provider = OpenAiResponsesProvider::new("synthetic-key".into(), Some(url));
        let a = provider.input_tokens(&request("cat")).await.unwrap();
        let b = provider.input_tokens(&request("dog")).await.unwrap();
        assert_eq!(a.reliable_input_tokens(), Some(100));
        assert_eq!(b.reliable_input_tokens(), Some(900));
        let c = provider.input_tokens(&request("x")).await.unwrap();
        let d = provider
            .input_tokens(&request(&"界".repeat(120)))
            .await
            .unwrap();
        assert_eq!(c.reliable_input_tokens(), Some(37));
        assert_eq!(d.reliable_input_tokens(), Some(37));
        let requests = server.await.unwrap();
        assert_eq!(
            serde_json::to_vec(&body(&requests[0])).unwrap().len(),
            serde_json::to_vec(&body(&requests[1])).unwrap().len()
        );
        assert_ne!(
            serde_json::to_vec(&body(&requests[2])).unwrap().len(),
            serde_json::to_vec(&body(&requests[3])).unwrap().len()
        );
        for raw in requests {
            assert!(raw.starts_with("POST /responses/input_tokens "));
            let counted = body(&raw);
            assert_eq!(counted["instructions"], "Keep the earlier constraint");
            assert_eq!(counted["tools"][0]["name"], "fixture_tool");
            assert!(counted.get("max_output_tokens").is_none());
        }
    }
    #[tokio::test]
    async fn missing_endpoint_is_unknown_and_malformed_count_is_not_zero() {
        let (url, server) = crate::provider::native_http_tests::server(vec![
            (404, "{}".into()),
            (200, json!({"object":"response.input_tokens"}).to_string()),
            (
                200,
                json!({"object":"response.input_tokens","input_tokens":-1}).to_string(),
            ),
            reply(0),
        ])
        .await;
        let provider = OpenAiResponsesProvider::new("synthetic-key".into(), Some(url));
        let request = request("one");
        let unknown = provider.input_tokens(&request).await.unwrap();
        assert_eq!(unknown.reliable_input_tokens(), None);
        assert!(!unknown.complete);
        assert!(provider.input_tokens(&request).await.is_err());
        assert!(provider.input_tokens(&request).await.is_err());
        assert_eq!(
            provider
                .input_tokens(&request)
                .await
                .unwrap()
                .reliable_input_tokens(),
            Some(0)
        );
        assert_eq!(server.await.unwrap().len(), 4);
    }
    #[test]
    fn catalogue_maximum_never_enables_capacity_and_unknown_encoder_fields_remain_unknown() {
        let maximum = catalog_capacity(
            &json!({"context_window":872000}),
            "model",
            "chatgpt_oauth",
            "https://fixture.invalid",
        )
        .unwrap();
        assert_eq!(maximum.enabled_window_tokens, None);
        assert_eq!(maximum.maximum_selectable_window_tokens, Some(872000));
        let known = catalog_capacity(
            &json!({"default_context_window":272000,"max_context_window":872000}),
            "model",
            "chatgpt_oauth",
            "https://fixture.invalid",
        )
        .unwrap();
        assert_eq!(known.enabled_window_tokens, Some(272000));
        assert!(
            catalog_capacity(
                &json!({"default_context_window":999,"max_context_window":100}),
                "model",
                "test",
                "https://fixture.invalid"
            )
            .is_none()
        );
        assert!(
            responses_input(&json!({"model":"m","input":"text","unclassified_state":"hidden"}))
                .is_none()
        );
        let input=responses_input(&json!({"model":"m","input":[],"previous_response_id":"saved","reasoning":{"effort":"high"},"max_output_tokens":64})).unwrap();
        assert_eq!(input["previous_response_id"], "saved");
        assert_eq!(input["reasoning"]["effort"], "high");
    }
    #[test]
    fn incomplete_or_estimated_counts_do_not_supply_an_exact_pressure_measurement() {
        let mut count = counted(
            &request("x"),
            "test",
            "https://fixture.invalid",
            &json!({"input":"x"}),
            &json!({"input_tokens":42}),
            CountPrecision::ProviderExact,
            "fixture",
        )
        .unwrap();
        assert_eq!(count.reliable_input_tokens(), Some(42));
        count.complete = false;
        assert_eq!(count.reliable_input_tokens(), None);
        count.complete = true;
        count.precision = CountPrecision::ProviderEstimate;
        assert_eq!(count.reliable_input_tokens(), None);
        assert!(
            !serde_json::to_string(&count)
                .unwrap()
                .contains("https://fixture.invalid")
        );
    }
    #[tokio::test]
    async fn counting_preserves_images_and_hidden_replay_in_the_actual_encoder_projection() {
        use voyage_protocol::content::{ContentPart, ImageAttachment};
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        let bytes = buffer.into_inner();
        let (media_type, width, height) = crate::images::validate(&bytes).unwrap();
        let id = uuid::Uuid::new_v4();
        let mut req = request("view synthetic pixel");
        req.messages[1].parts = vec![
            ContentPart::Text {
                text: "view synthetic pixel".into(),
            },
            ContentPart::Image {
                attachment: ImageAttachment {
                    id,
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    name: "synthetic".into(),
                    media_type,
                    byte_size: bytes.len() as u64,
                    width,
                    height,
                },
            },
        ];
        req.messages[1].image_data.insert(id, bytes);
        let mut assistant = Message::new(Role::Assistant, "");
        assistant.provider_state = Some(
            json!({"kind":"openai_responses_replay","version":1,"items":[{"type":"reasoning","id":"reasoning","encrypted_content":"OPAQUE_REPLAY","summary":[{"type":"summary_text","text":"fixture summary"}],"status":"completed"}]}),
        );
        req.messages.push(assistant);
        let (url, server) = crate::provider::native_http_tests::server(vec![reply(501)]).await;
        let provider = OpenAiResponsesProvider::new("synthetic-key".into(), Some(url));
        let count = provider.input_tokens(&req).await.unwrap();
        assert_eq!(count.reliable_input_tokens(), Some(501));
        let requests = server.await.unwrap();
        let counted = body(&requests[0]);
        let encoded = serde_json::to_string(&counted).unwrap();
        assert!(encoded.contains("input_image") && encoded.contains("data:image/png;base64,"));
        assert!(encoded.contains("OPAQUE_REPLAY"));
        assert_eq!(counted["tools"][0]["name"], "fixture_tool");
        let record = serde_json::to_string(&count).unwrap();
        assert!(!record.contains("OPAQUE_REPLAY") && !record.contains("data:image"));
    }
}
