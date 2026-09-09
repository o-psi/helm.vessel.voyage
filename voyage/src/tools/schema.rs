//! Offline, bounded compilation of untrusted tool contracts.
//!
//! Drafts 4, 6, 7, 2019-09 and 2020-12 are supported. Recursive/dynamic
//! references and non-local references are refused, as are regex extensions
//! requiring backtracking. Formats are annotations, per JSON Schema defaults.
use serde_json::Value;

use super::ToolError;

const MAX_SCHEMA_BYTES: usize = 256 * 1024;
const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 48;
const MAX_INSTANCE_BYTES: usize = 1024 * 1024;

pub(crate) struct CompiledSchema(jsonschema::Validator);

impl CompiledSchema {
    pub(crate) fn compile(schema: &Value) -> Result<Self, ToolError> {
        bounded(schema, MAX_SCHEMA_BYTES).map_err(schema_error)?;
        let mut remaining = MAX_NODES;
        inspect(schema, schema, 0, &mut remaining).map_err(schema_error)?;
        jsonschema::options()
            .with_retriever(Offline)
            .should_validate_formats(false)
            .with_pattern_options(
                jsonschema::PatternOptions::regex()
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024),
            )
            .build(schema)
            .map(Self)
            // Library errors can contain schema constants, URLs or credentials.
            .map_err(|_| schema_error("invalid or unsupported JSON Schema"))
    }

    pub(crate) fn validate(&self, value: &Value) -> Result<(), ToolError> {
        bounded(value, MAX_INSTANCE_BYTES).map_err(|_| {
            ToolError::InvalidArguments("tool value exceeds validation limits".into())
        })?;
        if self.0.is_valid(value) {
            Ok(())
        } else {
            // Neither instance values nor attacker-authored property names belong
            // in diagnostics; providers already receive the full input contract.
            Err(ToolError::InvalidArguments(
                "tool value does not match its declared JSON Schema".into(),
            ))
        }
    }
}

fn schema_error(message: &str) -> ToolError {
    ToolError::Failed(format!("tool schema refused: {message}"))
}

struct Offline;
impl jsonschema::Retrieve for Offline {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("external schema retrieval is disabled".into())
    }
}

fn bounded(value: &Value, max_bytes: usize) -> Result<(), &'static str> {
    fn walk(value: &Value, depth: usize, nodes: &mut usize, bytes: &mut usize) -> bool {
        if depth > MAX_DEPTH || *nodes == 0 {
            return false;
        }
        *nodes -= 1;
        let cost = match value {
            Value::String(s) => s.len(),
            Value::Object(object) => object.keys().map(String::len).sum(),
            _ => 8,
        };
        let Some(left) = bytes.checked_sub(cost) else {
            return false;
        };
        *bytes = left;
        match value {
            Value::Object(object) => object.values().all(|v| walk(v, depth + 1, nodes, bytes)),
            Value::Array(array) => array.iter().all(|v| walk(v, depth + 1, nodes, bytes)),
            _ => true,
        }
    }
    let mut nodes = MAX_NODES;
    let mut bytes = max_bytes;
    if walk(value, 0, &mut nodes, &mut bytes)
        && serde_json::to_vec(value).is_ok_and(|encoded| encoded.len() <= max_bytes)
    {
        Ok(())
    } else {
        Err("schema/value size, depth or node limit exceeded")
    }
}

/// Traverse schema locations only: a `$ref` inside a default/example is data.
/// Expanding local references here bounds graph complexity before compilation,
/// and rejects cycles without risking recursive validator stack exhaustion.
fn inspect(
    schema: &Value,
    root: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    if depth > MAX_DEPTH || *remaining == 0 {
        return Err("recursive or overly complex schemas are unsupported");
    }
    *remaining -= 1;
    let Some(object) = schema.as_object() else {
        return Ok(());
    };
    if let Some(vocabulary) = object.get("$vocabulary").and_then(Value::as_object) {
        for (uri, required) in vocabulary {
            if required == &Value::Bool(true)
                && !matches!(
                    uri.as_str(),
                    "https://json-schema.org/draft/2019-09/vocab/core"
                        | "https://json-schema.org/draft/2019-09/vocab/applicator"
                        | "https://json-schema.org/draft/2019-09/vocab/validation"
                        | "https://json-schema.org/draft/2019-09/vocab/meta-data"
                        | "https://json-schema.org/draft/2019-09/vocab/content"
                        | "https://json-schema.org/draft/2020-12/vocab/core"
                        | "https://json-schema.org/draft/2020-12/vocab/applicator"
                        | "https://json-schema.org/draft/2020-12/vocab/unevaluated"
                        | "https://json-schema.org/draft/2020-12/vocab/validation"
                        | "https://json-schema.org/draft/2020-12/vocab/meta-data"
                        | "https://json-schema.org/draft/2020-12/vocab/content"
                        | "https://json-schema.org/draft/2020-12/vocab/format-annotation"
                )
            {
                return Err("required schema vocabulary is unsupported");
            }
        }
    }
    if object.contains_key("$dynamicRef") || object.contains_key("$recursiveRef") {
        return Err("dynamic and recursive schema references are unsupported");
    }
    if let Some(reference) = object.get("$ref") {
        let reference = reference.as_str().ok_or("invalid schema reference")?;
        let pointer = reference
            .strip_prefix('#')
            .ok_or("only local JSON pointer references are supported")?;
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err("only local JSON pointer references are supported");
        }
        let target = root
            .pointer(pointer)
            .ok_or("unresolved local schema reference")?;
        inspect(target, root, depth + 1, remaining)?;
    }
    // Nested identifiers change reference resolution; do not accidentally check
    // one graph here and compile a different graph in the validation library.
    if depth != 0 && (object.contains_key("$id") || object.contains_key("id")) {
        return Err("nested schema identifiers are unsupported");
    }
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(map) = object.get(key).and_then(Value::as_object) {
            for child in map.values() {
                inspect(child, root, depth + 1, remaining)?;
            }
        }
    }
    if let Some(map) = object.get("dependencies").and_then(Value::as_object) {
        for child in map.values().filter(|value| !value.is_array()) {
            inspect(child, root, depth + 1, remaining)?;
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(array) = object.get(key).and_then(Value::as_array) {
            for child in array {
                inspect(child, root, depth + 1, remaining)?;
            }
        }
    }
    for key in [
        "items",
        "additionalItems",
        "additionalProperties",
        "unevaluatedItems",
        "unevaluatedProperties",
        "contains",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
        "contentSchema",
    ] {
        if let Some(child) = object.get(key) {
            if let Some(array) = child.as_array() {
                for child in array {
                    inspect(child, root, depth + 1, remaining)?;
                }
            } else {
                inspect(child, root, depth + 1, remaining)?;
            }
        }
    }
    Ok(())
}
