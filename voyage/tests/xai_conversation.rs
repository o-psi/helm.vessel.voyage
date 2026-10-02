//! Opt-in executing-host SuperGrok acceptance. Three streamed calls, no retries.
use futures_util::StreamExt;
use voyage::{
    Config, Message, Role,
    model::{ModelRequest, ToolDefinition},
    provider::ProviderStreamEvent,
};
use voyage_protocol::accounts::Transport;

#[tokio::test]
#[ignore = "requires an explicitly approved SuperGrok account and usage budget"]
async fn supergrok_conversation_and_tool_round_trip() {
    assert_eq!(
        std::env::var("VOYAGE_LIVE_XAI_APPROVED").as_deref(),
        Ok("yes")
    );
    let account = std::env::var("VOYAGE_TEST_XAI_ACCOUNT")
        .expect("set the named account UUID")
        .parse()
        .unwrap();
    let model = std::env::var("VOYAGE_TEST_MODEL").expect("explicit model ID required");
    let registry = voyage::accounts::Registry::default_host().unwrap();
    let mut config = Config::default();
    config
        .select_account(registry.freeze(account, Transport::XaiOauth).unwrap())
        .unwrap();
    let provider = voyage::provider::from_config(&config).unwrap();
    let marker = format!("voyage-{}", uuid::Uuid::new_v4());
    let mut history = vec![Message::new(
        Role::System,
        "Follow the requested exact response. Use the supplied echo_marker tool when asked. This test has no file or shell tools.",
    )];
    for index in 0..3 {
        if index == 0 {
            history.push(Message::new(
                Role::User,
                format!("Remember {marker}. Reply only with that marker."),
            ));
        }
        if index == 1 {
            history.push(Message::new(
                Role::User,
                "Call echo_marker with the marker I gave you.",
            ));
        }
        let request = ModelRequest {
            model: model.clone(),
            messages: history.clone(),
            tools: if index == 1 {
                vec![ToolDefinition {
                    output_schema: None,
                    annotations: None,
                    name: "echo_marker".into(),
                    description: "Echo the conversation marker.".into(),
                    input_schema: serde_json::json!({"type":"object","properties":{"marker":{"type":"string"}},"required":["marker"],"additionalProperties":false}),
                }]
            } else {
                vec![]
            },
            temperature: None,
            reasoning_effort: None,
            service_tier: None,
            max_tokens: Some(256),
        };
        let reply = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            let mut stream = provider.stream(request).await?;
            while let Some(event) = stream.next().await {
                if let ProviderStreamEvent::Completed(response) = event? {
                    return Ok(response.message);
                }
            }
            Err(voyage::provider::ProviderError::Incomplete)
        })
        .await
        .expect("bounded live reply timeout")
        .unwrap();
        assert_eq!(reply.role, Role::Assistant);
        if index == 1 {
            assert_eq!(reply.tool_calls.len(), 1);
            let call = reply.tool_calls[0].clone();
            assert_eq!(call.name, "echo_marker");
            assert_eq!(call.arguments["marker"], marker);
            history.push(reply);
            history.push(Message::tool(call.id, marker.clone()));
            history.push(Message::new(
                Role::User,
                "Repeat the tool result, only the marker.",
            ));
        } else {
            assert!(reply.tool_calls.is_empty());
            assert_eq!(reply.content.trim(), marker);
            history.push(reply);
        }
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        println!(
            "SuperGrok turn {} completed; canonical history/tool contract verified",
            index + 1
        );
    }
}

#[tokio::test]
#[ignore = "requires an explicitly approved SuperGrok account and usage budget"]
async fn supergrok_standard_native_tool_schemas() {
    assert_eq!(
        std::env::var("VOYAGE_LIVE_XAI_APPROVED").as_deref(),
        Ok("yes")
    );
    let account = std::env::var("VOYAGE_TEST_XAI_ACCOUNT")
        .unwrap()
        .parse()
        .unwrap();
    let registry = voyage::accounts::Registry::default_host().unwrap();
    let mut config = Config::default();
    config
        .select_account(registry.freeze(account, Transport::XaiOauth).unwrap())
        .unwrap();
    let tools = voyage::tools::ToolRegistry::standard().definitions();
    if let Ok(path) = std::env::var("VOYAGE_TEST_SCHEMA_EXPORT") {
        std::fs::write(path, serde_json::to_vec(&tools).unwrap()).unwrap();
    }
    let request = ModelRequest {
        model: std::env::var("VOYAGE_TEST_MODEL").unwrap(),
        messages: vec![Message::new(
            Role::User,
            "Call write_file once with path test.txt and content marker. Do not call other tools.",
        )],
        tools,
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: Some(512),
    };
    let provider = voyage::provider::from_config(&config).unwrap();
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        provider.complete(request),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(reply.message.tool_calls.len(), 1);
    assert_eq!(reply.message.tool_calls[0].name, "write_file");
}
