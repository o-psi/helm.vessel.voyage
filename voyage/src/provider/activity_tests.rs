use super::*;
use futures_util::{FutureExt, StreamExt};

#[derive(Clone, Copy, Debug)]
enum Adapter {
    Responses,
    Chat,
    Anthropic,
}

const ADAPTERS: [Adapter; 3] = [Adapter::Responses, Adapter::Chat, Adapter::Anthropic];

fn stream(adapter: Adapter, bytes: &[u8]) -> ProviderStream {
    // Fragment every frame across reads, then leave the connection open. A
    // decoder that swallows activity will remain pending rather than fail EOF.
    let source = futures_util::stream::iter(
        bytes
            .chunks(3)
            .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>(),
    )
    .chain(futures_util::stream::pending());
    match adapter {
        Adapter::Responses => Box::pin(openai_responses::responses_stream(source)),
        Adapter::Chat => Box::pin(openai::openai_stream(source)),
        Adapter::Anthropic => Box::pin(anthropic::anthropic_stream(source)),
    }
}

fn activity_frame(adapter: Adapter) -> &'static str {
    match adapter {
        Adapter::Responses => {
            r#"{"type":"response.reasoning_summary_text.delta","delta":"PRIVATE reasoning"}"#
        }
        Adapter::Chat => {
            r#"{"choices":[{"delta":{"role":"assistant","reasoning_content":"PRIVATE reasoning"}}]}"#
        }
        Adapter::Anthropic => {
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"PRIVATE reasoning"}}"#
        }
    }
}

#[test]
fn native_reasoning_and_protocol_activity_is_visible_without_public_text() {
    for adapter in ADAPTERS {
        let frames = format!(
            "data: {}\n\ndata: {{\"type\":\"ping\"}}\n\ndata: {{\"type\":\"future_event\"}}\n\n",
            activity_frame(adapter)
        );
        let mut events = stream(adapter, frames.as_bytes());
        for _ in 0..3 {
            assert!(
                matches!(
                    events.next().now_or_never(),
                    Some(Some(Ok(ProviderStreamEvent::Activity)))
                ),
                "{adapter:?} must yield protocol progress before waiting for more bytes"
            );
        }
        assert!(events.next().now_or_never().is_none(), "{adapter:?}");
    }
}

#[test]
fn comments_and_incomplete_frames_do_not_extend_stream_idle() {
    for adapter in ADAPTERS {
        let mut events = stream(adapter, b": heartbeat\n\n: heartbeat\n\ndata: {\"type\":");
        assert!(events.next().now_or_never().is_none(), "{adapter:?}");
        for data in [
            "invalid json",
            "null",
            "[]",
            "[{}]",
            "true",
            "42",
            "\"text\"",
        ] {
            let frame = format!("data: {data}\n\n");
            let mut events = stream(adapter, frame.as_bytes());
            let Some(Some(Err(error))) = events.next().now_or_never() else {
                panic!("{adapter:?} must reject non-event data without yielding activity");
            };
            assert_eq!(error.category(), "invalid_response");
            assert!(!error.is_retryable());
        }
    }
}

#[test]
fn activity_never_changes_completed_assistant_content() {
    for adapter in ADAPTERS {
        let completion = match adapter {
            Adapter::Responses => concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n"
            ),
            Adapter::Chat => concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            ),
            Adapter::Anthropic => concat!(
                "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"answer\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            ),
        };
        let frames = format!("data: {}\n\n{completion}", activity_frame(adapter));
        let mut events = stream(adapter, frames.as_bytes());
        let mut text = String::new();
        loop {
            let Some(Some(Ok(event))) = events.next().now_or_never() else {
                panic!("{adapter:?} did not complete");
            };
            match event {
                ProviderStreamEvent::Activity | ProviderStreamEvent::UsageReported(_) => {}
                ProviderStreamEvent::Delta(ProviderDelta::Text(delta)) => text.push_str(&delta),
                ProviderStreamEvent::Completed(response) => {
                    assert_eq!(text, "answer");
                    assert_eq!(response.message.content, "answer");
                    assert!(response.message.tool_calls.is_empty());
                    break;
                }
                other => panic!("unexpected {adapter:?} event: {other:?}"),
            }
        }
    }
}

#[test]
fn only_known_interruption_io_kinds_enable_stream_recovery() {
    for kind in [
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::ConnectionAborted,
        std::io::ErrorKind::BrokenPipe,
        std::io::ErrorKind::UnexpectedEof,
    ] {
        assert!(interrupted_stream_io(&std::io::Error::new(kind, "PRIVATE")));
    }
    for kind in [
        std::io::ErrorKind::InvalidData,
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::Other,
    ] {
        assert!(!interrupted_stream_io(&std::io::Error::new(
            kind, "PRIVATE"
        )));
    }
}
