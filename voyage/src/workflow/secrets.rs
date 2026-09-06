//! Volatile operator inputs. Public references carry names, never secret values.
use super::{Document, ParameterType, Rendered};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct SecretInputs {
    values: BTreeMap<String, Zeroizing<String>>,
}
impl std::fmt::Debug for SecretInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretInputs([hidden])")
    }
}

pub fn environment_name(name: &str) -> String {
    format!(
        "HELM_WORKFLOW_{}",
        name.replace('-', "_").to_ascii_uppercase()
    )
}

fn validate_names(document: &Document) -> Result<()> {
    document.validate()?;
    let mut names = BTreeSet::new();
    for (name, p) in &document.parameters {
        if p.secret {
            ensure!(
                names.insert(environment_name(name)),
                "workflow secret environment names collide"
            );
        }
    }
    Ok(())
}

impl SecretInputs {
    pub fn collect(document: &Document, values: Vec<(String, String)>) -> Result<Self> {
        // Wrap every input before any fallible validation, including unconsumed
        // trailing values. This does not promise to erase copies held by the OS.
        let values = values
            .into_iter()
            .map(|(name, value)| (name, Zeroizing::new(value)))
            .collect::<Vec<_>>();
        validate_names(document)?;
        ensure!(
            values.len() <= super::MAX_PARAMETERS,
            "too many workflow secret inputs"
        );
        let mut result = Self::default();
        for (name, text) in values {
            let p = document
                .parameters
                .get(&name)
                .filter(|p| p.secret)
                .ok_or_else(|| anyhow::anyhow!("unknown workflow secret input"))?;
            ensure!(
                text.len() <= 8192 && !text.contains('\0'),
                "workflow secret input exceeds bounds or contains NUL"
            );
            let valid = match p.kind {
                ParameterType::String => p.max_length.is_none_or(|max| text.len() <= max),
                ParameterType::Integer => text
                    .parse::<i64>()
                    .ok()
                    .is_some_and(|v| p.accepts(&Value::from(v))),
                ParameterType::Boolean => text
                    .parse::<bool>()
                    .ok()
                    .is_some_and(|v| p.accepts(&Value::Bool(v))),
            };
            ensure!(valid, "workflow secret input fails type or bounds");
            ensure!(
                result.values.insert(name, text).is_none(),
                "duplicate workflow secret input"
            );
        }
        Ok(result)
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.values.keys().cloned().collect()
    }

    pub fn bind(self, run_id: Uuid) -> Result<RunBindings> {
        ensure!(
            !run_id.is_nil(),
            "workflow secret binding requires an accepted run identity"
        );
        Ok(RunBindings {
            run_id,
            values: self.values,
        })
    }
}

/// No Serialize/Clone and no public access to values. Authority is transient and
/// belongs to an actual accepted run, never recovered from invocation metadata.
pub struct RunBindings {
    run_id: Uuid,
    values: BTreeMap<String, Zeroizing<String>>,
}
impl RunBindings {
    pub fn matches_run(&self, run: Uuid) -> bool {
        run == self.run_id
    }
    pub fn resolve(&self, run: Uuid, references: &[String]) -> Result<BoundEnvironment> {
        ensure!(
            self.matches_run(run),
            "workflow secret reference belongs to another run"
        );
        ensure!(
            references.len() <= super::MAX_PARAMETERS,
            "too many workflow secret references"
        );
        let mut environment = BTreeMap::new();
        for name in references {
            let value = self
                .values
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("workflow secret reference is unavailable"))?;
            ensure!(
                environment
                    .insert(environment_name(name), value.clone())
                    .is_none(),
                "duplicate workflow secret reference"
            );
        }
        Ok(BoundEnvironment(environment))
    }
}

/// Only trusted one-shot shell implementations receive this type. Do not add
/// Serialize or expose it through Config, ToolContext, PTY or provider records.
pub struct BoundEnvironment(BTreeMap<String, Zeroizing<String>>);
impl std::fmt::Debug for BoundEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoundEnvironment([hidden])")
    }
}
impl BoundEnvironment {
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

pub fn render_public(
    document: &Document,
    supplied: &[(String, String)],
    secrets: &BTreeSet<String>,
) -> Result<Rendered> {
    validate_names(document)?;
    ensure!(
        supplied.len() <= super::MAX_PARAMETERS,
        "too many workflow inputs"
    );
    for name in secrets {
        ensure!(
            document.parameters.get(name).is_some_and(|p| p.secret),
            "unknown workflow secret input"
        );
    }
    // Reuse the exact public parameter decoder, removing only the separately
    // validated secret declarations from this temporary rendering document.
    let mut public = document.clone();
    public.parameters.retain(|_, p| !p.secret);
    public.prompt = "Validate public inputs ".to_owned()
        + &public
            .parameters
            .keys()
            .map(|name| format!("{{{{{name}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
    let rendered = public.render(supplied)?;
    let mut values = rendered.inputs.clone();
    for (name, p) in &document.parameters {
        if p.secret {
            ensure!(
                !p.required || secrets.contains(name),
                "required workflow secret input is missing"
            );
            values.insert(
                name.clone(),
                if secrets.contains(name) {
                    json!({"workflow_secret":name,"environment":environment_name(name)})
                } else {
                    Value::Null
                },
            );
        }
    }
    let prompt = document.template(|name| Ok(serde_json::to_string(&values[name])?))?;
    Ok(Rendered {
        prompt,
        inputs: rendered.inputs,
    })
}
