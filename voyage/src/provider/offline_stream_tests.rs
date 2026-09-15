//! Pure byte streams: no listeners, HTTP servers, credentials or provider calls.
use super::*;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};

fn wire(events: &[Value], terminal: &str) -> Vec<u8> {
    let mut bytes = b": keepalive\r\n\r\n".to_vec();
    for event in events {
        bytes.extend_from_slice(format!("data: {event}\r\n\r\n").as_bytes());
    }
    bytes.extend_from_slice(terminal.as_bytes());
    bytes
}
async fn collect(
    adapter: usize,
    bytes: &[u8],
    width: usize,
) -> Vec<Result<ProviderStreamEvent, ProviderError>> {
    let chunks: Vec<_> = bytes
        .chunks(width)
        .map(|b| Ok(bytes::Bytes::copy_from_slice(b)))
        .collect();
    let source = stream::iter(chunks);
    match adapter {
        0 => openai::openai_stream(source).boxed().collect().await,
        1 => anthropic::anthropic_stream(source).boxed().collect().await,
        _ => {
            openai_responses::responses_stream(source)
                .boxed()
                .collect()
                .await
        }
    }
}
fn completed(events: &[Result<ProviderStreamEvent, ProviderError>]) -> &ModelResponse {
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Ok(ProviderStreamEvent::Completed(_))))
            .count(),
        1
    );
    match events.last().unwrap().as_ref().unwrap() {
        ProviderStreamEvent::Completed(response) => response,
        other => panic!("expected completion, got {other:?}"),
    }
}

#[tokio::test]
async fn chat_fragmentation_usage_and_parallel_tool_arguments() {
    let events = [
        json!({"choices":[{"delta":{"content":"hé"}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lookup","arguments":"{\"x\":"}},{"index":1,"id":"b","function":{"name":"ping","arguments":""}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":4},"service_tier":"default"}),
    ];
    for terminal in ["", "data: [DONE]\r\n\r\n"] {
        let bytes = wire(&events, terminal);
        for width in [1, 7, bytes.len()] {
            let events = collect(0, &bytes, width).await;
            let response = completed(&events);
            assert_eq!(response.message.content, "hé");
            assert_eq!(response.usage.input_tokens, 9);
            assert_eq!(response.usage.output_tokens, 4);
            assert_eq!(response.message.tool_calls.len(), 2);
            assert_eq!(response.message.tool_calls[0].arguments, json!({"x":1}));
            assert_eq!(response.message.tool_calls[1].arguments, json!({}));
        }
    }
}

#[tokio::test]
async fn anthropic_sparse_blocks_thinking_and_usage() {
    let bytes = wire(
        &[
            json!({"type":"message_start","message":{"usage":{"input_tokens":12,"service_tier":"standard"}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"consider"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"résultat"}}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"call","name":"lookup"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"ok\":"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"true}"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}),
            json!({"type":"message_stop"}),
        ],
        "",
    );
    for width in [1, 19, bytes.len()] {
        let events = collect(1, &bytes, width).await;
        assert!(events.iter().any(|e| matches!(e, Ok(ProviderStreamEvent::Delta(ProviderDelta::Reasoning {text, ..})) if text == "consider")));
        let response = completed(&events);
        assert_eq!(response.message.content, "résultat");
        assert_eq!(response.usage.input_tokens, 12);
        assert_eq!(response.usage.output_tokens, 8);
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].arguments, json!({"ok":true}));
    }
}

#[tokio::test]
async fn responses_final_output_reconciles_deltas_and_retains_replay() {
    let call =
        json!({"type":"function_call","call_id":"c","name":"lookup","arguments":"{\"x\":2}"});
    let message =
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"hé"}]});
    let reasoning = json!({"type":"reasoning","id":"r","encrypted_content":"offline-ciphertext","summary":[{"type":"summary_text","text":"summary"}]});
    let bytes = wire(
        &[
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"summary"}),
            json!({"type":"response.output_text.delta","delta":"hé"}),
            json!({"type":"response.output_item.added","output_index":2,"item":call}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"x\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"delta":"2}"}),
            json!({"type":"response.output_item.done","output_index":2,"item":call}),
            json!({"type":"response.completed","response":{"status":"completed","output":[reasoning,message,call],"usage":{"input_tokens":15,"output_tokens":7}}}),
        ],
        "",
    );
    for width in [1, 31, bytes.len()] {
        let events = collect(2, &bytes, width).await;
        let response = completed(&events);
        assert_eq!(response.message.content, "hé");
        assert_eq!(response.message.tool_calls[0].arguments, json!({"x":2}));
        assert_eq!(response.usage.input_tokens, 15);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(
            response.message.provider_state.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
}

#[tokio::test]
async fn all_adapters_fail_closed_on_bad_frames_and_truncation() {
    for adapter in 0..3 {
        for bytes in [
            b"data: {\n\n".as_slice(),
            b"data: []\n\n",
            b"data: null\n\n",
        ] {
            let events = collect(adapter, bytes, 1).await;
            assert!(matches!(
                events.last(),
                Some(Err(ProviderError::InvalidResponse(_)))
            ));
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Ok(ProviderStreamEvent::Completed(_))))
            );
        }
        for bytes in [b"".as_slice(), b"data: {", b": heartbeat\n\n"] {
            let events = collect(adapter, bytes, 8).await;
            assert!(matches!(
                events.last(),
                Some(Err(ProviderError::StreamInterrupted))
            ));
        }
        let oversized = vec![b'x'; 4 * 1024 * 1024 + 1];
        let events = collect(adapter, &oversized, oversized.len()).await;
        assert!(matches!(
            events.last(),
            Some(Err(ProviderError::InvalidResponse(_)))
        ));
    }
}

#[tokio::test]
async fn provider_rejections_never_publish_completion_or_private_messages() {
    for (adapter, event) in [
        (
            0,
            json!({"error":{"code":"insufficient_quota","message":"PRIVATE"}}),
        ),
        (
            1,
            json!({"type":"error","error":{"type":"overloaded_error","message":"PRIVATE"}}),
        ),
        (
            2,
            json!({"type":"response.failed","response":{"error":{"code":"insufficient_quota","message":"PRIVATE"}}}),
        ),
        (
            2,
            json!({"type":"response.incomplete","response":{"status":"incomplete"}}),
        ),
        (2, json!({"type":"response.cancelled"})),
    ] {
        let events = collect(adapter, &wire(&[event], ""), 3).await;
        let error = events.last().unwrap().as_ref().unwrap_err();
        assert!(!error.to_string().contains("PRIVATE"));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Ok(ProviderStreamEvent::Completed(_))))
        );
    }
}
