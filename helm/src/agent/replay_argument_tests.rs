//! Responses replay stores function arguments as JSON inside a JSON string.
//! Neutral tool calls are not evidence that historical replay strings are safe.
use super::*;
use crate::model::{ModelResponse, Role, ToolCall};
use serde_json::json;
use std::sync::Mutex;

const SECRET: &str = "p秘密\"\\\n\u{1b}z";
const ENCODED: &str = r#"{ "value" : "p\u79d8\u5bc6\"\\\n\u001bz" }"#;
const HARMLESS: &str = r#"{ "value" : "public \u79d8\u5bc6", "escaped": "\n", "number" : 1.00 }"#;

struct Capture(Arc<Mutex<Vec<ModelRequest>>>);
#[async_trait]
impl Provider for Capture {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.0.lock().unwrap().push(request);
        Ok(ModelResponse {
            message: Message::new(Role::Assistant, "finished"),
            usage: Usage::default(),
        })
    }
}
fn historical(arguments: &str, mismatched_neutral: bool) -> Message {
    let mut message = Message::new(Role::Assistant, "historical function call");
    message.provider_state = Some(
        json!({"kind":"openai_responses_replay","version":1,"items":[
            {"type":"function_call","id":"fc_history","call_id":"call_history","name":"effect","arguments":arguments}
        ]}),
    );
    if mismatched_neutral {
        message.tool_calls.push(ToolCall {
            id: "different_call".into(),
            name: "effect".into(),
            arguments: json!({"value":"public"}),
        });
    }
    message
}

#[tokio::test]
async fn encoded_replay_secrets_refuse_without_matching_neutral_tool_calls() {
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(ENCODED).unwrap()["value"],
        SECRET
    );
    assert!(
        !ENCODED.contains(SECRET),
        "fixture must require parsing the supported argument encoding"
    );
    for mismatched in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let requests = Arc::new(Mutex::new(vec![]));
        let mut agent = tests::agent(Box::new(Capture(requests.clone())), &directory);
        agent.context.redactor = Arc::new(crate::tools::Redactor::new(vec![SECRET.into()]));
        let history = vec![historical(ENCODED, mismatched)];
        let before = serde_json::to_value(&history).unwrap();
        let result = agent.run(history.clone(), "public follow-up".into()).await;
        assert!(
            result.is_err(),
            "encoded secret-bearing replay reached provider continuation"
        );
        assert!(
            requests.lock().unwrap().is_empty(),
            "historical replay must be checked before dispatch"
        );
        assert_eq!(
            serde_json::to_value(history).unwrap(),
            before,
            "refusal rewrote original historical evidence"
        );
    }
}

#[tokio::test]
async fn harmless_replay_argument_string_is_preserved_byte_for_byte() {
    let directory = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(vec![]));
    let mut agent = tests::agent(Box::new(Capture(requests.clone())), &directory);
    agent.context.redactor = Arc::new(crate::tools::Redactor::new(vec![SECRET.into()]));
    let history = vec![historical(HARMLESS, false)];
    let before = serde_json::to_value(&history).unwrap();
    let outcome = agent
        .run(history.clone(), "public follow-up".into())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(history).unwrap(), before);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let replay = requests[0]
        .messages
        .iter()
        .find_map(|message| message.provider_state.as_ref())
        .unwrap();
    assert_eq!(
        replay["items"][0]["arguments"].as_str().unwrap().as_bytes(),
        HARMLESS.as_bytes()
    );
    assert_eq!(
        outcome.messages[0].provider_state.as_ref().unwrap()["items"][0]["arguments"],
        HARMLESS
    );
}
