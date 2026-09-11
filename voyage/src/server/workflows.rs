//! Trusted workflow rendering and bounded, one-use volatile private bindings.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use uuid::Uuid;
use voyage_protocol::process::RuntimeCommand;
use zeroize::Zeroizing;
type PrivateValues = Vec<(String, Zeroizing<String>)>;
struct Private {
    actor: Uuid,
    deadline: Instant,
    values: PrivateValues,
}
pub(super) struct Prepared {
    pub prompt: String,
    pub invocation: crate::workflow::Invocation,
    pub secrets: crate::workflow::secrets::SecretInputs,
}
type PreparedEntries = BTreeMap<Uuid, (Uuid, Instant, Prepared)>;
#[derive(Default)]
pub(super) struct Workflows {
    inputs: Arc<Mutex<BTreeMap<Uuid, Private>>>,
    prepared: Arc<Mutex<PreparedEntries>>,
}
impl Workflows {
    pub async fn pending(&self) -> bool {
        let mut inputs = self.inputs.lock().await;
        inputs.retain(|_, entry| entry.deadline > Instant::now());
        let mut prepared = self.prepared.lock().await;
        prepared.retain(|_, (_, deadline, _)| *deadline > Instant::now());
        !inputs.is_empty() || !prepared.is_empty()
    }
    pub async fn discard(&self, actor: Uuid, command: Uuid, input: Option<Uuid>) {
        let mut prepared = self.prepared.lock().await;
        if prepared
            .get(&command)
            .is_some_and(|(owner, _, _)| *owner == actor)
        {
            prepared.remove(&command);
        }
        drop(prepared);
        if let Some(input) = input {
            let mut inputs = self.inputs.lock().await;
            if inputs.get(&input).is_some_and(|entry| entry.actor == actor) {
                inputs.remove(&input);
            }
        }
    }
    pub async fn clear(&self) {
        self.inputs.lock().await.clear();
        self.prepared.lock().await.clear();
    }
    pub async fn store(
        &self,
        actor: Uuid,
        id: Uuid,
        values: Vec<(String, String)>,
    ) -> Result<Value> {
        let values = values
            .into_iter()
            .map(|(name, value)| (name, Zeroizing::new(value)))
            .collect::<Vec<_>>();
        ensure!(
            !id.is_nil()
                && values.len() <= 64
                && values
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>()
                    <= 65536,
            "private workflow input exceeds bounds"
        );
        let mut inputs = self.inputs.lock().await;
        inputs.retain(|_, value| value.deadline > Instant::now());
        ensure!(
            inputs.len() < 16 && !inputs.contains_key(&id),
            "private input handle busy or already used"
        );
        inputs.insert(
            id,
            Private {
                actor,
                deadline: Instant::now() + Duration::from_secs(60),
                values,
            },
        );
        let retained = self.inputs.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            retained
                .lock()
                .await
                .retain(|_, entry| entry.deadline > Instant::now());
        });
        Ok(json!({"input_id":id,"expires_in_seconds":60,"storage":"volatile","replay":"never"}))
    }
    pub async fn take(&self, id: Uuid, actor: Uuid) -> Result<Option<Prepared>> {
        let mut prepared = self.prepared.lock().await;
        prepared.retain(|_, (_, deadline, _)| *deadline > Instant::now());
        if let Some((owner, _, _)) = prepared.get(&id) {
            ensure!(*owner == actor, "workflow actor mismatch");
        }
        Ok(prepared.remove(&id).map(|(_, _, prepared)| prepared))
    }
    pub async fn prepare(
        &self,
        workspace: &std::path::Path,
        actor: Uuid,
        command: &RuntimeCommand,
    ) -> Result<RuntimeCommand> {
        let RuntimeCommand::WorkflowSubmit {
            command_id,
            expected_revision,
            expires_at_ms,
            id,
            scope,
            user_directory,
            inputs,
            trust_digest,
            private_inputs_id,
        } = command
        else {
            anyhow::bail!("not a workflow submission")
        };
        let definition = definition(
            workspace,
            id,
            scope.as_deref(),
            user_directory.as_deref(),
            trust_digest.as_deref(),
        )?;
        let values = if let Some(id) = private_inputs_id {
            let mut entries = self.inputs.lock().await;
            entries.retain(|_, value| value.deadline > Instant::now());
            let entry = entries
                .get(id)
                .context("private workflow input unavailable or expired")?;
            ensure!(
                entry.actor == actor,
                "private workflow input actor mismatch"
            );
            entries
                .remove(id)
                .expect("inspected entry")
                .values
                .into_iter()
                .map(|(name, value)| (name, value.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        let secrets =
            crate::workflow::secrets::SecretInputs::collect(&definition.document, values)?;
        let rendered = crate::workflow::secrets::render_public(
            &definition.document,
            inputs,
            &secrets.names(),
        )?;
        let invocation = definition.invocation(rendered.inputs);
        let prompt = rendered.prompt;
        let mut prepared = self.prepared.lock().await;
        prepared.retain(|_, (_, deadline, _)| *deadline > Instant::now());
        ensure!(
            prepared.len() < 16 && !prepared.contains_key(command_id),
            "workflow preparation already pending"
        );
        prepared.insert(
            *command_id,
            (
                actor,
                Instant::now() + Duration::from_secs(60),
                Prepared {
                    prompt: prompt.clone(),
                    invocation,
                    secrets,
                },
            ),
        );
        let retained = self.prepared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            retained
                .lock()
                .await
                .retain(|_, (_, deadline, _)| *deadline > Instant::now());
        });
        Ok(RuntimeCommand::Submit {
            coordination: None,
            command_id: *command_id,
            expected_revision: *expected_revision,
            expires_at_ms: *expires_at_ms,
            prompt,
        })
    }
}
fn definition(
    workspace: &std::path::Path,
    id: &str,
    scope: Option<&str>,
    user_directory: Option<&std::path::Path>,
    trust: Option<&str>,
) -> Result<crate::workflow::Definition> {
    let scope = match scope {
        Some("user") => Some(crate::workflow::Scope::User),
        Some("repository") => Some(crate::workflow::Scope::Repository),
        None => None,
        _ => anyhow::bail!("invalid workflow scope"),
    };
    let definition = crate::workflow::select(
        crate::workflow::discover(workspace, user_directory)?,
        id,
        scope,
    )?;
    definition.authorize(trust)?;
    Ok(definition)
}
pub(super) fn preview(
    workspace: &std::path::Path,
    id: &str,
    scope: Option<&str>,
    user_directory: Option<&std::path::Path>,
    inputs: &[(String, String)],
    trust: Option<&str>,
    optional_secret_names: Option<&[String]>,
) -> Result<Value> {
    let definition = definition(workspace, id, scope, user_directory, trust)?;
    let names = crate::workflow::secrets::preview_names(
        &definition.document,
        optional_secret_names.unwrap_or_default(),
    )?;
    let rendered = crate::workflow::secrets::render_public(&definition.document, inputs, &names)?;
    Ok(
        json!({"definition":definition,"prompt":rendered.prompt,"inputs":rendered.inputs,"secrets":"private_inputs_required_for_execution","secret_names":names}),
    )
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("preview-test.toml"),
            r#"schema_version = 1
id = "preview-test"
version = "1"
description = "Name-only preview regression"
prompt = "Required {{required}} optional {{optional}} public {{public}}"
[parameters.required]
type = "string"
secret = true
required = true
[parameters.optional]
type = "string"
secret = true
[parameters.public]
type = "string"
default = "visible"
"#,
        )
        .unwrap();
        root
    }

    fn render(root: &std::path::Path, names: Option<&[String]>) -> Result<Value> {
        preview(
            root,
            "preview-test",
            Some("user"),
            Some(root),
            &[],
            None,
            names,
        )
    }

    #[test]
    fn preview_legacy_empty_and_selected_names() {
        let root = fixture();
        let legacy = render(root.path(), None).unwrap();
        assert_eq!(legacy, render(root.path(), Some(&[])).unwrap());
        assert_eq!(legacy["secret_names"], json!(["required"]));
        assert!(legacy["prompt"].as_str().unwrap().contains("optional null"));
        let selected = render(root.path(), Some(&["optional".into()])).unwrap();
        assert_eq!(selected["secret_names"], json!(["optional", "required"]));
        assert!(
            selected["prompt"]
                .as_str()
                .unwrap()
                .contains("HELM_WORKFLOW_OPTIONAL")
        );
        assert_eq!(selected["inputs"], json!({"public":"visible"}));
    }

    #[test]
    fn preview_rejects_invalid_duplicate_public_required_and_unknown_names() {
        let root = fixture();
        for names in [
            vec!["unknown".into()],
            vec!["public".into()],
            vec!["required".into()],
            vec!["optional".into(), "optional".into()],
            vec!["".into()],
            vec!["a".repeat(65)],
            vec!["optional=value".into()],
            vec!["HELM_WORKFLOW_OPTIONAL".into()],
            vec!["optional\0".into()],
            vec!["optional".into(); 33],
        ] {
            assert!(render(root.path(), Some(&names)).is_err());
        }
        // A secret cannot be smuggled through public inputs, either.
        assert!(
            preview(
                root.path(),
                "preview-test",
                Some("user"),
                Some(root.path()),
                &[("optional".into(), "private-sentinel".into())],
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn preview_accepts_maximum_declared_name_count_and_identifier_length() {
        let root = fixture();
        let mut document = definition(
            root.path(),
            "preview-test",
            Some("user"),
            Some(root.path()),
            None,
        )
        .unwrap()
        .document;
        let parameter = document.parameters["optional"].clone();
        document.parameters.clear();
        document.prompt = "Preview maximum selection".into();
        let names: Vec<String> = (0..32).map(|index| format!("s{index:063}")).collect();
        for name in &names {
            assert_eq!(name.len(), 64);
            document.parameters.insert(name.clone(), parameter.clone());
        }
        let resolved = crate::workflow::secrets::preview_names(&document, &names).unwrap();
        assert_eq!(resolved.len(), 32);
        assert!(crate::workflow::secrets::render_public(&document, &[], &resolved).is_ok());
    }

    #[tokio::test]
    async fn preview_matches_private_submission_without_changing_mutation_identity() {
        let root = fixture();
        let workflows = Workflows::default();
        let actor = Uuid::new_v4();
        let input_id = Uuid::new_v4();
        workflows
            .store(
                actor,
                input_id,
                vec![
                    ("required".into(), "private-required-sentinel".into()),
                    ("optional".into(), "private-optional-sentinel".into()),
                ],
            )
            .await
            .unwrap();
        let command_id = Uuid::new_v4();
        let command = RuntimeCommand::WorkflowSubmit {
            command_id,
            expected_revision: 7,
            expires_at_ms: 123456,
            id: "preview-test".into(),
            scope: Some("user".into()),
            user_directory: Some(root.path().into()),
            inputs: vec![],
            trust_digest: None,
            private_inputs_id: Some(input_id),
        };
        let translated = workflows
            .prepare(root.path(), actor, &command)
            .await
            .unwrap();
        let response = render(root.path(), Some(&["optional".into()])).unwrap();
        let RuntimeCommand::Submit {
            command_id: actual_id,
            expected_revision,
            expires_at_ms,
            prompt,
            ..
        } = &translated
        else {
            panic!("expected translated submit")
        };
        assert_eq!(*actual_id, command_id);
        assert_eq!(*expected_revision, 7);
        assert_eq!(*expires_at_ms, 123456);
        assert_eq!(response["prompt"], *prompt);
        let prepared = workflows.take(command_id, actor).await.unwrap().unwrap();
        assert_eq!(response["secret_names"], json!(prepared.secrets.names()));
        for public in [
            response.to_string(),
            serde_json::to_string(&command).unwrap(),
            serde_json::to_string(&translated).unwrap(),
            serde_json::to_string(&prepared.invocation).unwrap(),
        ] {
            assert!(!public.contains("private-required-sentinel"));
            assert!(!public.contains("private-optional-sentinel"));
        }
    }
}
