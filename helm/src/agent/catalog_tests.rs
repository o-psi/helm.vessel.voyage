use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Catalog {
    calls: Arc<AtomicUsize>,
    values: Arc<std::sync::Mutex<Vec<ModelInfo>>>,
}
#[async_trait]
impl Provider for Catalog {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.values.lock().unwrap().clone())
    }
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        panic!("catalog refresh must not perform inference")
    }
}

#[tokio::test]
async fn catalog_secrets_are_rejected_before_cache_without_changing_model() {
    let directory = tempfile::tempdir().unwrap();
    let values = Arc::new(std::sync::Mutex::new(vec![ModelInfo::minimal("good")]));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut agent = super::tests::agent(
        Box::new(Catalog {
            calls: calls.clone(),
            values: values.clone(),
        }),
        &directory,
    );
    let secret = "configured-\"秘密\"";
    agent.context.redactor = Arc::new(crate::tools::Redactor::new([secret.into()]));
    let safe = agent.models(false).await.unwrap();
    assert!(safe.iter().any(|model| model.id == "test"));
    for field in 0..5 {
        let mut model = ModelInfo::minimal("good");
        match field {
            0 => model.id = secret.into(),
            1 => model.display_name = secret.into(),
            2 => model.description = secret.into(),
            3 => model.reasoning_efforts = vec![secret.into()],
            _ => model.input_modalities = vec![secret.into()],
        }
        *values.lock().unwrap() = vec![model];
        let error = agent.models(true).await.unwrap_err().to_string();
        assert!(!error.contains(secret));
        assert_eq!(agent.model(), "test");
        assert_eq!(agent.models(false).await.unwrap(), safe);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    // A secret equal to the redaction marker is still secret-bearing metadata.
    agent.context.redactor = Arc::new(crate::tools::Redactor::new(["[REDACTED]".into()]));
    *values.lock().unwrap() = vec![ModelInfo::minimal("[REDACTED]")];
    assert!(agent.models(true).await.is_err());
}

#[tokio::test]
async fn current_model_fallback_cannot_expand_catalog_count_or_byte_bounds() {
    let directory = tempfile::tempdir().unwrap();
    let values = Arc::new(std::sync::Mutex::new(Vec::new()));
    let agent = super::tests::agent(
        Box::new(Catalog {
            calls: Arc::new(AtomicUsize::new(0)),
            values: values.clone(),
        }),
        &directory,
    );
    let original = agent.models(false).await.unwrap();
    *values.lock().unwrap() = (0..1024)
        .map(|index| ModelInfo::minimal(format!("model-{index}")))
        .collect();
    assert!(agent.models(true).await.is_err());
    assert_eq!(agent.models(false).await.unwrap(), original);
    values.lock().unwrap().pop();
    assert_eq!(agent.models(true).await.unwrap().len(), 1024);
    // 1020 * (512-byte ID + 512-byte name + "text") = 1 MiB minus 16 bytes.
    let mut metadata = vec![ModelInfo::minimal("x".repeat(512)); 1020];
    metadata[0].description = "12345".into();
    crate::provider::validate_models(&metadata, &[]).unwrap();
    *values.lock().unwrap() = metadata;
    assert!(agent.models(true).await.is_err()); // The 12-byte "test" fallback exceeds it.
    assert_eq!(agent.model(), "test");
}
