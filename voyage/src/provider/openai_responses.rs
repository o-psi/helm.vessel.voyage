use super::CompatibleAuthentication;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ModelInfo, Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent,
    checked_json, checked_stream_response,
};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};

/// Native OpenAI Responses API transport. This is separate from the compatible
/// Chat Completions adapter because the wire formats and streaming events differ.
pub struct OpenAiResponsesProvider {
    client: reqwest::Client,
    api_key: super::api_credential::ApiCredential,
    base_url: String,
}

impl OpenAiResponsesProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: super::native_http_client(),
            api_key: super::api_credential::ApiCredential::Legacy(api_key),
            base_url: base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .into(),
        }
    }
    pub(super) fn with_account(
        mut self,
        config: &crate::Config,
        redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
    ) -> Self {
        if let Some(binding) = &config.account {
            self.api_key = super::api_credential::ApiCredential::Account {
                binding: binding.clone(),
                authority: config.provider_authority.clone(),
                redactor,
            };
        }
        self
    }
}

#[async_trait]
impl Provider for OpenAiResponsesProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        super::discovery::models(&self.client, &self.base_url, &self.api_key.resolve()?)
            .await?
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "model-list unavailable; use a manual model ID".into(),
                )
            })
    }

    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        super::validate_native_endpoint(&self.base_url)?;
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(
            self,
            &crate::config::ProviderKind::OpenaiResponses,
            &request,
        )
        .await?;
        let result: Result<ModelResponse, ProviderError> =
            super::multimodal::guard(images, async {
                let response = super::endpoint_http_client(&self.client, &self.base_url)
                    .post(format!("{}/responses", self.base_url))
                    .apply_key(&self.api_key.resolve()?)
                    .json(&request_body(request, false)?)
                    .send()
                    .await
                    .map_err(map_transport)?;
                decode_response(checked_json(response).await?)
            })
            .await;
        result
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        super::validate_native_endpoint(&self.base_url)?;
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(
            self,
            &crate::config::ProviderKind::OpenaiResponses,
            &request,
        )
        .await?;
        let result: Result<ProviderStream, ProviderError> =
            super::multimodal::guard(images, async {
                let response = super::endpoint_http_client(&self.client, &self.base_url)
                    .post(format!("{}/responses", self.base_url))
                    .apply_key(&self.api_key.resolve()?)
                    .json(&request_body(request, true)?)
                    .send()
                    .await
                    .map_err(map_transport)?;
                let response = checked_stream_response(response).await?;
                Ok(Box::pin(responses_stream(response.bytes_stream())) as ProviderStream)
            })
            .await;
        result.map(|stream| super::multimodal::guard_stream(images, stream))
    }
}

pub(crate) fn request_body(request: ModelRequest, stream: bool) -> Result<Value, ProviderError> {
    request_body_for(
        &crate::config::ProviderKind::OpenaiResponses,
        request,
        stream,
    )
}
pub(crate) fn request_body_for(
    provider: &crate::ProviderKind,
    request: ModelRequest,
    stream: bool,
) -> Result<Value, ProviderError> {
    super::inference::validate_request(provider, &request)?;
    super::multimodal::validate_adapter(provider, &request)?;
    super::multimodal::validate_request(&request)?;
    let instructions = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut input = request
        .messages
        .iter()
        .filter(|message| message.role != Role::System)
        .map(encode_message)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    // Older compacted sessions can begin with results whose calls were lost.
    // Preserve their context without sending an invalid function_call_output.
    let mut calls = std::collections::HashSet::new();
    for item in &mut input {
        match item["type"].as_str() {
            Some("function_call") => {
                if let Some(id) = item["call_id"].as_str() {
                    calls.insert(id.to_owned());
                }
            }
            Some("function_call_output")
                if !item["call_id"].as_str().is_some_and(|id| calls.remove(id)) =>
            {
                if item["output"].is_array() {
                    return Err(ProviderError::Request("visual tool result has no matching original function call; cannot relabel it as User input".into()));
                }
                *item = json!({"type":"message","role":"user","content":format!(
                    "[Tool result retained from earlier history; original call unavailable]\n{}",
                    item["output"].as_str().unwrap_or_default()
                )});
            }
            _ => {}
        }
    }
    let tools=request.tools.into_iter().map(|tool|json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.input_schema,"strict":false})).collect::<Vec<_>>();
    let mut body = json!({"model":request.model,"input":input,"stream":stream,"store":false,"include":["reasoning.encrypted_content"]});
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions)
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(true);
    }
    if let Some(max) = request.max_tokens {
        body["max_output_tokens"] = json!(max)
    }
    if let Some(effort) = request.reasoning_effort {
        body["reasoning"] = json!({"effort": effort});
    }
    if let Some(tier) = request.service_tier {
        body["service_tier"] = json!(tier);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature)
    }
    super::multimodal::check_body(&body)?;
    Ok(body)
}

// Preserve opaque reasoning/message records, while an exact repeated function
// call in one provider response has one outgoing call/result correlation.
fn unique_replay_calls(items: Vec<Value>) -> Result<Vec<Value>, ProviderError> {
    let mut seen = std::collections::BTreeMap::new();
    let mut result = Vec::with_capacity(items.len());
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            let id = item["call_id"].as_str().unwrap_or_default().to_owned();
            let name = item["name"].as_str().unwrap_or_default().to_owned();
            let arguments: Value = serde_json::from_str(
                item["arguments"].as_str().unwrap_or_default(),
            )
            .map_err(|_| {
                ProviderError::InvalidResponse(
                    "invalid function arguments in Responses continuation".into(),
                )
            })?;
            let identity = (name, arguments);
            if let Some(previous) = seen.insert(id, identity.clone()) {
                if previous != identity {
                    return Err(ProviderError::InvalidResponse(
                        "conflicting function call identities in Responses continuation".into(),
                    ));
                }
                continue;
            }
        }
        result.push(item);
    }
    Ok(result)
}

fn encode_message(message: &Message) -> Result<Vec<Value>, ProviderError> {
    if let Some(content) = super::multimodal::content(message, super::multimodal::Wire::Responses)?
    {
        return Ok(vec![if message.role == Role::Tool {
            json!({"type":"function_call_output", "call_id":message.tool_call_id, "output":content})
        } else {
            json!({"type":"message", "role":"user", "content":content})
        }]);
    }
    if message.role == Role::Tool {
        return Ok(message
            .tool_call_id
            .as_ref()
            .map(|id| {
                vec![json!({"type":"function_call_output","call_id":id,"output":message.content})]
            })
            .unwrap_or_default());
    }
    if message.role == Role::Assistant
        && let Some(state) = message.provider_state.as_ref()
    {
        let mut replay = decode_replay(state)?;
        // Some Responses-compatible streaming endpoints omit `response.output`
        // from the terminal event. Older Helm sessions consequently contain an
        // empty replay envelope even though the neutral message still has text
        // or tool calls. Reconstruct any omitted neutral items so a following
        // function_call_output can never be orphaned.
        if !message.content.is_empty()
            && !replay
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        {
            let index = replay
                .iter()
                .position(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
                .unwrap_or(replay.len());
            replay.insert(
                index,
                json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":message.content}]}),
            );
        }
        for call in &message.tool_calls {
            let present = replay.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("function_call")
                    && item.get("call_id").and_then(Value::as_str) == Some(call.id.as_str())
            });
            if !present {
                replay.push(json!({
                    "type":"function_call",
                    "call_id":call.id,
                    "name":call.name,
                    "arguments":call.arguments.to_string()
                }));
            }
        }
        return unique_replay_calls(replay);
    }
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => unreachable!(),
    };
    let mut values = Vec::new();
    if !message.content.is_empty() {
        values.push(json!({"type":"message","role":role,"content":message.content}));
    }
    values.extend(message.tool_calls.iter().map(|call|json!({"type":"function_call","call_id":call.id,"name":call.name,"arguments":call.arguments.to_string()})));
    Ok(values)
}

const REPLAY_VERSION: u8 = 1;
const MAX_REPLAY_BYTES: usize = 2 * 1024 * 1024;
const MAX_REPLAY_ITEMS: usize = 256;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayEnvelope {
    kind: String,
    version: u8,
    items: Vec<ReplayItem>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ReplayItem {
    Reasoning {
        id: String,
        encrypted_content: String,
        #[serde(default)]
        summary: Vec<ReplaySummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
    Message {
        role: String,
        content: Vec<ReplayContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ReplaySummary {
    SummaryText { text: String },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ReplayContent {
    OutputText { text: String },
}

fn make_replay(output: &[Value]) -> Result<Value, ProviderError> {
    let items = output
        .iter()
        .map(parse_replay_item)
        .collect::<Result<Vec<_>, _>>()?;
    validate_replay(&items)?;
    let value = serde_json::to_value(ReplayEnvelope {
        kind: "openai_responses_replay".into(),
        version: REPLAY_VERSION,
        items,
    })
    .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
    if serde_json::to_vec(&value)
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?
        .len()
        > MAX_REPLAY_BYTES
    {
        return Err(ProviderError::InvalidResponse(
            "Responses replay state exceeds 2 MiB".into(),
        ));
    }
    Ok(value)
}
fn parse_replay_item(value: &Value) -> Result<ReplayItem, ProviderError> {
    let string = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    match value.get("type").and_then(Value::as_str) {
        Some("reasoning") => {
            let summary = value
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("summary_text"))
                .map(|part| ReplaySummary::SummaryText {
                    text: part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                })
                .collect();
            Ok(ReplayItem::Reasoning {
                id: string("id").unwrap_or_default(),
                encrypted_content: string("encrypted_content").unwrap_or_default(),
                summary,
                status: string("status"),
            })
        }
        Some("function_call") => Ok(ReplayItem::FunctionCall {
            call_id: string("call_id").unwrap_or_default(),
            name: string("name").unwrap_or_default(),
            arguments: string("arguments").unwrap_or_else(|| "{}".into()),
            id: string("id"),
            status: string("status"),
        }),
        Some("message") => {
            let content = value
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
                .map(|part| ReplayContent::OutputText {
                    text: part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                })
                .collect();
            Ok(ReplayItem::Message {
                role: string("role").unwrap_or_default(),
                content,
                id: string("id"),
                status: string("status"),
            })
        }
        Some(kind) => Err(ProviderError::InvalidResponse(format!(
            "unsupported Responses replay item type `{kind}`"
        ))),
        None => Err(ProviderError::InvalidResponse(
            "Responses replay item omitted type".into(),
        )),
    }
}
fn decode_replay(value: &Value) -> Result<Vec<Value>, ProviderError> {
    if serde_json::to_vec(value)
        .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?
        .len()
        > MAX_REPLAY_BYTES
    {
        return Err(ProviderError::InvalidResponse(
            "Responses replay state exceeds 2 MiB".into(),
        ));
    }
    let envelope: ReplayEnvelope = serde_json::from_value(value.clone()).map_err(|e| {
        ProviderError::InvalidResponse(format!("invalid Responses replay state: {e}"))
    })?;
    if envelope.kind != "openai_responses_replay" || envelope.version != REPLAY_VERSION {
        return Err(ProviderError::InvalidResponse(
            "unsupported Responses replay state kind/version".into(),
        ));
    }
    validate_replay(&envelope.items)?;
    envelope
        .items
        .into_iter()
        .map(|item| {
            serde_json::to_value(item).map_err(|e| ProviderError::InvalidResponse(e.to_string()))
        })
        .collect()
}
fn validate_replay(items: &[ReplayItem]) -> Result<(), ProviderError> {
    if items.len() > MAX_REPLAY_ITEMS {
        return Err(ProviderError::InvalidResponse(
            "Responses replay state has too many items".into(),
        ));
    }
    for item in items {
        match item {
            ReplayItem::Reasoning {
                id,
                encrypted_content,
                ..
            } if id.is_empty()
                || encrypted_content.is_empty()
                || encrypted_content.len() > MAX_REPLAY_BYTES =>
            {
                return Err(ProviderError::InvalidResponse(
                    "reasoning replay omitted id or encrypted_content".into(),
                ));
            }
            ReplayItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } if call_id.is_empty() || name.is_empty() || arguments.len() > MAX_REPLAY_BYTES => {
                return Err(ProviderError::InvalidResponse(
                    "invalid function-call replay item".into(),
                ));
            }
            ReplayItem::Message { role, content, .. }
                if role != "assistant" || content.len() > MAX_REPLAY_ITEMS =>
            {
                return Err(ProviderError::InvalidResponse(
                    "invalid message replay item".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Default)]
struct Assembly {
    service_tier: Option<String>,
    content: String,
    calls: std::collections::BTreeMap<usize, Call>,
    usage: Usage,
    output: std::collections::BTreeMap<usize, Value>,
}
#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}
const MAX_SSE_BUFFER_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn responses_stream<S>(
    mut source: S,
) -> impl futures_util::Stream<Item = Result<ProviderStreamEvent, ProviderError>> + Send
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    use futures_util::StreamExt;
    async_stream::try_stream! {
        let mut pending=Vec::new();let mut assembly=Assembly::default();
        while let Some(chunk)=source.next().await {
            pending.extend_from_slice(&chunk.map_err(map_transport)?);
            if pending.len()>MAX_SSE_BUFFER_BYTES{Err(ProviderError::InvalidResponse("OpenAI Responses stream event exceeded 4 MiB".into()))?;}
            while let Some(frame)=super::openai::take_sse_frame(&mut pending){let data=super::openai::sse_data(&frame);if data.is_empty(){continue}let event:Value=serde_json::from_slice(data).map_err(|e|ProviderError::InvalidResponse(format!("invalid OpenAI Responses stream event: {e}")))?;
                if let Some(usage) = event.pointer("/response/usage") {
                    yield ProviderStreamEvent::UsageReported(super::reported_usage(usage, "input_tokens", "output_tokens")?);
                }
                let kind=event.get("type").and_then(Value::as_str).unwrap_or_default();
                match kind {
                    "response.output_text.delta"=>if let Some(delta)=event.get("delta").and_then(Value::as_str){assembly.content.push_str(delta);yield ProviderStreamEvent::Delta(ProviderDelta::Text(delta.into()));},
                    "response.output_item.added"=>if event.pointer("/item/type").and_then(Value::as_str)==Some("function_call"){let index=output_index(&event);let call=assembly.calls.entry(index).or_default();call.id=event.pointer("/item/call_id").and_then(Value::as_str).unwrap_or_default().into();call.name=event.pointer("/item/name").and_then(Value::as_str).unwrap_or_default().into();yield ProviderStreamEvent::Delta(ProviderDelta::ToolCall{index,id:Some(call.id.clone()),name:Some(call.name.clone()),arguments:String::new()});},
                    "response.function_call_arguments.delta"=>{let index=output_index(&event);let call=assembly.calls.entry(index).or_default();let delta=event.get("delta").and_then(Value::as_str).unwrap_or_default();call.arguments.push_str(delta);yield ProviderStreamEvent::Delta(ProviderDelta::ToolCall{index,id:None,name:None,arguments:delta.into()});},
                    "response.output_item.done"=>merge_output_item(&event,&mut assembly)?,
                    "response.completed"=>{if let Some(response)=event.get("response"){validate_status(response)?;merge_final(response,&mut assembly)?;}yield ProviderStreamEvent::Completed(finish(assembly)?);return},
                    "response.failed"|"response.incomplete"|"response.cancelled"|"error"=>Err(decode_stream_error(&event))?,
                    _=>{}
                }
            }
        }
        Err(ProviderError::InvalidResponse("OpenAI Responses stream ended before response.completed".into()))?;
    }
}
fn merge_output_item(event: &Value, assembly: &mut Assembly) -> Result<(), ProviderError> {
    let item = event.get("item").cloned().ok_or_else(|| {
        ProviderError::InvalidResponse("Responses output_item.done omitted item".into())
    })?;
    let index = output_index(event);
    merge_call(index, &item, assembly);
    assembly.output.insert(index, item);
    Ok(())
}
fn output_index(event: &Value) -> usize {
    event
        .get("output_index")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}
fn merge_final(response: &Value, assembly: &mut Assembly) -> Result<(), ProviderError> {
    assembly.service_tier = super::reported_service_tier(response.get("service_tier"));
    if let Some(usage) = response.get("usage") {
        assembly.usage = decode_usage(usage)
    }
    if assembly.content.is_empty() {
        assembly.content = output_text(response)
    }
    for (index, item) in response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        merge_call(index, item, assembly);
        assembly.output.insert(index, item.clone());
    }
    Ok(())
}
fn merge_call(index: usize, item: &Value, assembly: &mut Assembly) {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return;
    }
    let call = assembly.calls.entry(index).or_default();
    if call.id.is_empty() {
        call.id = item
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into()
    }
    if call.name.is_empty() {
        call.name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into()
    }
    if call.arguments.is_empty() {
        call.arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("{}")
            .into()
    }
}
fn finish(assembly: Assembly) -> Result<ModelResponse, ProviderError> {
    let output = assembly.output.into_values().collect::<Vec<_>>();
    let provider_state = (!output.is_empty())
        .then(|| make_replay(&output))
        .transpose()?;
    let calls = assembly
        .calls
        .into_values()
        .map(|call| {
            if call.id.is_empty() || call.name.is_empty() {
                return Err(ProviderError::InvalidResponse(
                    "Responses function call omitted call_id or name".into(),
                ));
            }
            Ok(ToolCall {
                id: call.id,
                name: call.name,
                arguments: serde_json::from_str(if call.arguments.is_empty() {
                    "{}"
                } else {
                    &call.arguments
                })
                .map_err(|e| {
                    ProviderError::InvalidResponse(format!("bad Responses function arguments: {e}"))
                })?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ModelResponse {
        service_tier: assembly.service_tier,
        message: Message {
            coordination: None,
            tool_outcome: None,
            tool_output: None,
            parts: Vec::new(),
            image_data: Default::default(),
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role: Role::Assistant,
            content: assembly.content,
            tool_call_id: None,
            tool_calls: calls,
            tool_success: None,
            provider_state,
            steering: None,
        },
        usage: assembly.usage,
    })
}

pub(crate) fn decode_response(value: Value) -> Result<ModelResponse, ProviderError> {
    validate_status(&value)?;
    let mut assembly = Assembly {
        service_tier: super::reported_service_tier(value.get("service_tier")),
        usage: value.get("usage").map(decode_usage).unwrap_or_default(),
        ..Assembly::default()
    };
    for (index, item) in value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        assembly.output.insert(index, item.clone());
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for content in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if content.get("type").and_then(Value::as_str) == Some("output_text") {
                        assembly.content.push_str(
                            content
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                    }
                }
            }
            Some("function_call") => {
                assembly.calls.insert(
                    index,
                    Call {
                        id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        arguments: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .into(),
                    },
                );
            }
            _ => {}
        }
    }
    finish(assembly)
}
fn decode_usage(value: &Value) -> Usage {
    Usage {
        input_tokens: value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}
fn output_text(response: &Value) -> String {
    response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}
fn decode_stream_error(value: &Value) -> ProviderError {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // Output exhaustion is not a rejected input, even if a contradictory code is present.
    if kind == "response.incomplete" {
        return ProviderError::Incomplete;
    }
    if let Some(error) = super::rejection::classify(value, None) {
        return error;
    }
    let error = value
        .pointer("/response/error")
        .or_else(|| value.get("error"))
        .unwrap_or(value);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
        })
        .unwrap_or(match kind {
            "response.incomplete" => "OpenAI response was incomplete",
            "response.cancelled" => "OpenAI response was cancelled",
            _ => "OpenAI Responses stream failed",
        })
        .to_owned();
    match code {
        "usage_limit_reached" | "insufficient_quota" => ProviderError::UsageLimit,
        "rate_limit_exceeded" => ProviderError::RateLimit {
            message,
            retry_after: None,
        },
        "server_error" | "service_unavailable" => ProviderError::Unavailable(message),
        _ => ProviderError::Request(message),
    }
}

fn validate_status(value: &Value) -> Result<(), ProviderError> {
    match value.get("status").and_then(Value::as_str) {
        None | Some("completed") => Ok(()),
        Some("incomplete") => Err(ProviderError::Incomplete),
        Some("failed") => Err(super::rejection::classify(value, None)
            .unwrap_or_else(|| ProviderError::InvalidResponse("response did not complete".into()))),
        _ => Err(ProviderError::InvalidResponse(
            "response did not complete".into(),
        )),
    }
}
fn map_transport(error: reqwest::Error) -> ProviderError {
    super::map_transport(error)
}

#[cfg(test)]
mod context_rejection_tests {
    use super::*;
    use futures_util::StreamExt;

    // ChatGPT OAuth and native Responses both use this stream decoder.
    #[tokio::test]
    async fn sse_body_transport_failure_is_distinct_from_clean_eof() {
        let response = super::super::failure_tests::http_response(200, "0", "data: {", 100).await;
        let stream = responses_stream(response.bytes_stream());
        futures_util::pin_mut!(stream);
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.category(), "transport");
        assert_eq!(error.http_status(), None);
        assert!(!error.is_retryable());
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn truncated_sse_is_not_context_rejection_or_retryable() {
        let source = futures_util::stream::iter(vec![Ok(bytes::Bytes::from_static(b"data: {"))]);
        let stream = responses_stream(source);
        futures_util::pin_mut!(stream);
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.category(), "invalid_response");
        assert_eq!(error.http_status(), None);
        assert!(!error.is_context_length());
        assert!(!error.is_retryable());
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn responses_and_oauth_sse_classify_nested_and_flat_errors() {
        for event in [
            json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"PRIVATE"}}}),
            json!({"type":"error","code":"context_length_exceeded","message":"PRIVATE"}),
            json!({"type":"error","error":{"code":"context_length_exceeded","message":"PRIVATE"}}),
        ] {
            let frame = format!("data: {event}\n\n");
            let source = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(frame))]);
            let stream = responses_stream(source);
            futures_util::pin_mut!(stream);
            let error = stream.next().await.unwrap().unwrap_err();
            assert!(matches!(error, ProviderError::ContextLength));
            assert!(!error.is_retryable());
            assert_eq!(error.to_string(), super::super::CONTEXT_LENGTH_MESSAGE);
            assert!(stream.next().await.is_none());
        }
    }

    #[test]
    fn failed_response_classification_preserves_output_and_account_limits() {
        let error = decode_response(json!({"status":"failed","error":{
            "code":"context_length_exceeded","message":"PRIVATE"
        }}))
        .unwrap_err();
        assert!(matches!(error, ProviderError::ContextLength));
        for reason in ["max_output_tokens", "content_filter"] {
            let event = json!({"type":"response.incomplete","response":{
                "status":"incomplete","incomplete_details":{"reason":reason},
                "error":{"code":"context_length_exceeded"}
            }});
            assert!(matches!(
                decode_stream_error(&event),
                ProviderError::Incomplete
            ));
            assert!(matches!(
                decode_response(event["response"].clone()),
                Err(ProviderError::Incomplete)
            ));
        }
        for code in ["usage_limit_reached", "insufficient_quota"] {
            let event = json!({"type":"response.failed","response":{"error":{
                "code":code,"message":"PRIVATE"
            }}});
            assert!(matches!(
                decode_stream_error(&event),
                ProviderError::UsageLimit
            ));
        }
        let unknown = json!({"type":"error","code":"rate_limit_exceeded"});
        assert!(matches!(
            decode_stream_error(&unknown),
            ProviderError::RateLimit { .. }
        ));
        assert!(decode_stream_error(&unknown).is_retryable());
    }
}
