//! Preview compatibility: omission keeps old behavior; explicit selection must not
//! be silently downgraded by a peer that predates the name-selection contract.
use crate::{process::RuntimeCommand, vessel::VoyageCommand};
use serde::Deserialize;
use serde_json::{Value, json};

fn legacy_preview() -> Value {
    json!({"op":"workflow_preview", "id":"example", "scope":null,
        "user_directory":null, "inputs":[], "trust_digest":null})
}

#[test]
fn preview_optional_selection_wire_roundtrip_and_legacy_omission() {
    for names in [None, Some(json!([])), Some(json!(["optional"]))] {
        let mut wire = legacy_preview();
        if let Some(names) = names {
            wire["optional_secret_names"] = names;
        }
        let public: VoyageCommand = serde_json::from_value(wire.clone()).unwrap();
        let runtime: RuntimeCommand = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(public).unwrap(), wire);
        assert_eq!(serde_json::to_value(runtime).unwrap(), wire);
    }
}

// Exact field set and deny_unknown_fields policy of pre-selection previews.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[allow(dead_code)]
enum LegacyCommand {
    WorkflowPreview {
        id: String,
        scope: Option<String>,
        user_directory: Option<std::path::PathBuf>,
        inputs: Vec<(String, String)>,
        trust_digest: Option<String>,
    },
}

#[test]
fn old_peers_refuse_explicit_selection_and_values_are_not_a_selection() {
    let mut wire = legacy_preview();
    assert!(serde_json::from_value::<LegacyCommand>(wire.clone()).is_ok());
    wire["optional_secret_names"] = json!(["optional"]);
    assert!(serde_json::from_value::<LegacyCommand>(wire.clone()).is_err());
    wire["optional_secret_names"] = json!({"optional":"private-value"});
    assert!(serde_json::from_value::<VoyageCommand>(wire.clone()).is_err());
    assert!(serde_json::from_value::<RuntimeCommand>(wire).is_err());
}
