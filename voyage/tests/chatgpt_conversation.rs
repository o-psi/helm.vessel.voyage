//! Explicit live check: uses the executing user's native ChatGPT OAuth login.
//! Run only with provider/budget approval; makes at most three requests, no tools.
use futures_util::StreamExt;
use voyage::{
    Message, Role,
    model::ModelRequest,
    provider::{
        ChatGptOauthProvider, ChatGptTokenStore, OAuthEndpoints, Provider, ProviderStreamEvent,
    },
};

#[tokio::test]
#[ignore = "requires explicit approval for three live ChatGPT OAuth replies"]
async fn three_message_chatgpt_conversation() {
    let model = std::env::var("VOYAGE_TEST_MODEL").expect("set VOYAGE_TEST_MODEL explicitly");
    let provider = ChatGptOauthProvider::from_store(
        ChatGptTokenStore::new(ChatGptTokenStore::default_path().unwrap()),
        OAuthEndpoints::default(),
    );
    let marker = format!("voyage-{}", uuid::Uuid::new_v4());
    let mut history = vec![Message::new(
        Role::System,
        "This is a short conversation check. Follow the user's requested exact reply. Do not use tools.",
    )];
    let prompts = [
        format!("Remember this marker: {marker}. Reply with only that marker."),
        "What marker did I ask you to remember? Reply with only the marker.".into(),
        "Repeat the same marker once more, with no other text.".into(),
    ];
    for (index, prompt) in prompts.into_iter().enumerate() {
        history.push(Message::new(Role::User, prompt));
        let request = ModelRequest {
            model: model.clone(),
            messages: history.clone(),
            tools: vec![],
            temperature: None,
            max_tokens: None,
        };
        let reply = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut stream = provider.stream(request).await?;
            while let Some(event) = stream.next().await {
                if let ProviderStreamEvent::Completed(response) = event? {
                    return Ok(response.message);
                }
            }
            Err(voyage::provider::ProviderError::InvalidResponse(
                "stream ended without a completed reply".into(),
            ))
        })
        .await
        .expect("reply exceeded 60 seconds")
        .unwrap_or_else(|error| panic!("turn {} failed: {error}", index + 1));
        assert_eq!(reply.role, Role::Assistant);
        assert!(reply.tool_calls.is_empty());
        assert_eq!(
            reply.content.trim(),
            marker,
            "turn {} lost context",
            index + 1
        );
        assert!(
            reply.provider_state.is_some(),
            "missing Responses continuation"
        );
        // Persist/reload the complete canonical history between turns.
        history.push(reply);
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        println!("turn {}: completed, remembered marker", index + 1);
    }
    assert_eq!(history.len(), 7);
}
