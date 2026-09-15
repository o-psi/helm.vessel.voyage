//! Final transport bytes, not token estimates or provider billing.
use super::ProviderError;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Default, Serialize)]
struct Components {
    total_bytes: usize,
    instructions_bytes: usize,
    schemas_bytes: usize,
    history_bytes: usize,
    envelope_bytes: usize,
}
fn measure(body: &Value, bytes: &[u8]) -> Components {
    let mut c = Components {
        total_bytes: bytes.len(),
        ..Default::default()
    };
    if let Some(object) = body.as_object() {
        for (key, value) in object {
            let count = serde_json::to_vec(value).map_or(0, |v| v.len());
            match key.as_str() {
                "instructions" | "system" => c.instructions_bytes += count,
                "tools" | "functions" => c.schemas_bytes += count,
                "messages" | "input" => c.history_bytes += count,
                _ => (),
            }
        }
    }
    c.envelope_bytes = c.total_bytes - c.instructions_bytes - c.schemas_bytes - c.history_bytes;
    c
}
tokio::task_local! {
    static OBSERVED: std::sync::Arc<std::sync::Mutex<Option<voyage_protocol::provider_attempt::RequestBytes>>>;
}

/// Scope observations to one logical provider attempt, including dispatch failures.
/// More than one encoded body is accumulated (e.g. authentication refresh retry).
pub(crate) async fn observe<T>(
    future: impl std::future::Future<Output = T>,
) -> (T, Option<voyage_protocol::provider_attempt::RequestBytes>) {
    let observations = std::sync::Arc::new(std::sync::Mutex::new(None));
    let result = OBSERVED.scope(observations.clone(), future).await;
    let bytes = observations.lock().ok().and_then(|v| v.clone());
    (result, bytes)
}

fn retain(c: &Components) {
    let _ = OBSERVED.try_with(|slot| {
        if let Ok(mut value) = slot.lock() {
            let value = value.get_or_insert_with(Default::default);
            value.total = value.total.saturating_add(c.total_bytes as u64);
            value.instructions = value
                .instructions
                .saturating_add(c.instructions_bytes as u64);
            value.schemas = value.schemas.saturating_add(c.schemas_bytes as u64);
            value.history = value.history.saturating_add(c.history_bytes as u64);
            value.envelope = value.envelope.saturating_add(c.envelope_bytes as u64);
        }
    });
}

/// Encode once and send these same bytes. Logs contain counts only, never payloads.
/// Events describe construction, not successful dispatch; retries produce new events.
pub(super) fn body(
    request: reqwest::RequestBuilder,
    value: &Value,
) -> Result<reqwest::RequestBuilder, ProviderError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| ProviderError::InvalidResponse("cannot encode provider request".into()))?;
    let c = measure(value, &bytes);
    retain(&c);
    tracing::info!(target: "voyage::request_accounting", total_bytes=c.total_bytes,
        instructions_bytes=c.instructions_bytes, schemas_bytes=c.schemas_bytes,
        history_bytes=c.history_bytes, envelope_bytes=c.envelope_bytes,
        "provider request encoded (bytes, not tokens; dispatch unconfirmed)");
    Ok(request
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn disjoint_components_include_escaping_and_envelope() {
        for value in [
            json!({"instructions":"é\\\"\n", "tools":[{"name":"x"}],"input":[{"type":"function_call_output","output":"secret-not-logged"}],"model":"m"}),
            json!({"system":[{"text":"s"}],"messages":[{"role":"user","content":"x"}],"stream":true}),
            json!({"model":"m"}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            let c = measure(&value, &bytes);
            assert_eq!(
                c.total_bytes,
                c.instructions_bytes + c.schemas_bytes + c.history_bytes + c.envelope_bytes
            );
            assert_eq!(c.total_bytes, bytes.len());
            assert!(
                !serde_json::to_string(&c)
                    .unwrap()
                    .contains("secret-not-logged")
            );
        }
    }
    #[test]
    fn transmitted_bytes_equal_measured_encoding() {
        let value = json!({"messages":[{"content":"é\n\\"}],"tools":[]});
        let req = body(
            reqwest::Client::new().post("http://localhost/unused"),
            &value,
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            req.body().unwrap().as_bytes().unwrap(),
            serde_json::to_vec(&value).unwrap()
        );
    }
    #[tokio::test]
    async fn scoped_observations_preserve_missing_and_failures() {
        let (_, missing) = observe(async { 1 }).await;
        assert!(missing.is_none());
        let (result, observed) = observe(async {
            let _request = body(
                reqwest::Client::new().post("http://localhost/unused"),
                &json!({"input":"é"}),
            )
            .unwrap();
            Err::<(), _>("simulated dispatch failure")
        })
        .await;
        assert!(result.is_err());
        let measured = observed.unwrap();
        assert_eq!(
            measured.total,
            measured.instructions + measured.schemas + measured.history + measured.envelope
        );
        let (_, again) = observe(async {}).await;
        assert!(again.is_none());
    }
}
