use crate::tools::schema::CompiledSchema;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Tool,
    Command,
    Lifecycle,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definitions {
    pub tools: Vec<Definition>,
    pub commands: Vec<Definition>,
    pub lifecycle: Vec<Definition>,
}

pub(crate) fn validate_definitions(value: &Value, capabilities: &[String]) -> Result<()> {
    Definitions::parse(value, capabilities).map(|_| ())
}

impl Definitions {
    pub(crate) fn parse(value: &Value, capabilities: &[String]) -> Result<Self> {
        ensure!(
            capabilities == ["execute"] || capabilities == ["execute", "host.file.read"],
            "unsupported extension capabilities"
        );
        let bytes = serde_json::to_vec(value)?;
        ensure!(
            bytes.len() <= 512 * 1024,
            "extension definitions exceed limit"
        );
        super::wire::parse_json(&bytes)?;
        let definitions: Self = serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("invalid extension definitions"))?;
        ensure!(
            definitions.tools.len() <= 32
                && definitions.commands.len() <= 16
                && definitions.lifecycle.len() <= 2,
            "too many extension definitions"
        );
        let mut names = BTreeSet::new();
        for (kind, list) in [
            (Kind::Tool, &definitions.tools),
            (Kind::Command, &definitions.commands),
            (Kind::Lifecycle, &definitions.lifecycle),
        ] {
            for def in list {
                ensure!(valid_name(&def.name), "invalid extension definition name");
                ensure!(
                    names.insert(def.name.clone()),
                    "duplicate extension definition name"
                );
                ensure!(
                    def.description.len() <= 4096 && !def.description.chars().any(char::is_control),
                    "invalid extension description"
                );
                if kind == Kind::Lifecycle {
                    ensure!(
                        matches!(def.name.as_str(), "run_start" | "run_finish"),
                        "unsupported extension lifecycle"
                    );
                }
                CompiledSchema::compile(&def.input_schema)?;
                CompiledSchema::compile(&def.output_schema)?;
            }
        }
        Ok(definitions)
    }

    pub(crate) fn find(&self, kind: Kind, name: &str) -> Result<&Definition> {
        let list = match kind {
            Kind::Tool => &self.tools,
            Kind::Command => &self.commands,
            Kind::Lifecycle => &self.lifecycle,
        };
        list.iter()
            .find(|def| def.name == name)
            .ok_or_else(|| anyhow::anyhow!("extension definition not pinned"))
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn definitions_are_pinned_bounded_and_offline() {
        let mut value = json!({"tools":[{"name":"echo","description":"Echo",
            "input_schema":{"type":"object"},"output_schema":true}],"commands":[],"lifecycle":[]});
        assert!(validate_definitions(&value, &["execute".into()]).is_ok());
        assert!(validate_definitions(&value, &["host.file.read".into()]).is_err());
        value["tools"][0]["input_schema"] = json!({"$ref":"https://invalid/schema"});
        assert!(validate_definitions(&value, &["execute".into()]).is_err());
        value["tools"][0]["input_schema"] = json!(true);
        value["extra"] = json!(true);
        assert!(validate_definitions(&value, &["execute".into()]).is_err());
    }
}
