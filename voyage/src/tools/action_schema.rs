//! Exact action contracts: every branch declares its entire accepted key set.
use serde_json::{Value, json};

pub(crate) type Action<'a> = (&'a str, &'a [&'a str], &'a [&'a str]);
pub(crate) fn schema(properties: Value, common: &[&str], actions: &[Action<'_>]) -> Value {
    let branches: Vec<_> = actions.iter().map(|(action, required, optional)| {
        let mut fields = serde_json::Map::new();
        fields.insert("action".into(), json!({"const":action}));
        for key in common.iter().chain(required.iter()).chain(optional.iter()) {
            assert!(!properties[*key].is_null(), "missing native action property");
            fields.insert((*key).into(), properties[*key].clone());
        }
        let required: Vec<_> = std::iter::once("action").chain(required.iter().copied()).collect();
        json!({"type":"object","properties":fields,"required":required,"additionalProperties":false})
    }).collect();
    let mut properties = properties;
    properties["action"] =
        json!({"type":"string","enum":actions.iter().map(|a|a.0).collect::<Vec<_>>()});
    json!({"type":"object","properties":properties,"required":["action"],"additionalProperties":false,"oneOf":branches})
}

/// Serde unit variants accept trailing object fields even with deny_unknown_fields.
/// Compare with the parsed action's full field set before any effects as well.
pub(crate) fn reject_extra_fields(
    arguments: &Value,
    parsed: &impl serde::Serialize,
) -> Result<(), super::ToolError> {
    let allowed = serde_json::to_value(parsed)
        .map_err(|_| super::ToolError::InvalidArguments("invalid action".into()))?;
    if arguments
        .as_object()
        .is_none_or(|fields| fields.keys().any(|key| allowed.get(key).is_none()))
    {
        return Err(super::ToolError::InvalidArguments(
            "field is not accepted by this action; use its declared schema".into(),
        ));
    }
    Ok(())
}
