//! Owner-local registry boundaries; no default registry or environment changes.
use super::*;
fn local() -> (tempfile::TempDir, Registry) {
    let dir = tempfile::tempdir().unwrap();
    let r = Registry::new(dir.path().join("accounts"));
    (dir, r)
}
#[test]
fn connection_validation_is_transactional_and_builtin_ids_are_stable() {
    let (_dir, r) = local();
    for endpoint in [
        "http://example.invalid",
        "https://user@example.invalid",
        "https://example.invalid/?key=x",
        "https://example.invalid/#fragment",
        " https://example.invalid",
        "https://example.invalid/\n",
    ] {
        assert!(
            r.add_connection(
                "fixture".into(),
                endpoint.into(),
                vec![Transport::OpenaiChat]
            )
            .is_err()
        );
    }
    assert!(
        r.add_connection("fixture".into(), "https://example.invalid".into(), vec![])
            .is_err()
    );
    assert!(r.connections().unwrap().is_empty());
    for provider in ["openai", "anthropic"] {
        let first = r.ensure_api_connection(provider).unwrap();
        assert_eq!(r.ensure_api_connection(provider).unwrap().id, first.id);
        assert_eq!(r.connection(first.id).unwrap().revision, 1);
    }
    assert!(r.ensure_api_connection("other").is_err());
    assert!(r.connection(Uuid::new_v4()).is_err());
    assert_eq!(r.connections().unwrap().len(), 2);
}
#[test]
fn api_rotation_recovery_and_removal_preserve_binding_invariants() {
    let (_dir, r) = local();
    let c = r.ensure_api_connection("openai").unwrap();
    let a = r
        .add_api(
            c.id,
            "first".into(),
            "First".into(),
            ApiKeyInput::Stored("fixture-one".into()),
        )
        .unwrap();
    let b = r
        .add_api(
            c.id,
            "second".into(),
            "Second".into(),
            ApiKeyInput::Stored("fixture-two".into()),
        )
        .unwrap();
    let binding = r.freeze(a.id, Transport::OpenaiResponses).unwrap();
    assert!(r.freeze(a.id, Transport::Anthropic).is_err());
    assert!(r.rename(a.id, "second".into(), "Duplicate".into()).is_err());
    r.rename(a.id, "renamed".into(), "Renamed".into()).unwrap();
    assert_eq!(r.validate_binding(&binding).unwrap().alias, "renamed");
    let rotated = r
        .rotate_api(&binding, ApiKeyInput::Stored("fixture-three".into()), true)
        .unwrap();
    assert_eq!(rotated.identity_generation, binding.identity_generation);
    assert_eq!(r.resolve_api_key(&binding).unwrap(), "fixture-three");
    let replaced = r
        .rotate_api(&binding, ApiKeyInput::Stored("fixture-four".into()), false)
        .unwrap();
    assert!(replaced.identity_generation > binding.identity_generation);
    assert!(r.validate_binding(&binding).is_err());
    let new_binding = r.freeze(a.id, Transport::OpenaiResponses).unwrap();
    r.logout(a.id, false).unwrap();
    assert!(r.resolve_api_key(&new_binding).is_err());
    let logged_out = r.list(|d| d.id == a.id).unwrap().1.remove(0);
    assert!(
        r.reauthenticate_api(
            a.id,
            new_binding.identity_generation,
            ApiKeyInput::Stored("fixture-five".into()),
            true
        )
        .is_err()
    );
    let restored = r
        .reauthenticate_api(
            a.id,
            logged_out.identity_generation,
            ApiKeyInput::Stored("fixture-five".into()),
            true,
        )
        .unwrap();
    assert_eq!(restored.state, AccountState::Ready);
    let restored_binding = r.freeze(a.id, Transport::OpenaiResponses).unwrap();
    assert_eq!(
        r.resolve_api_key(&restored_binding).unwrap(),
        "fixture-five"
    );
    r.logout(a.id, true).unwrap();
    r.logout(a.id, false).unwrap();
    let removed = r.list(|d| d.id == a.id).unwrap().1.remove(0);
    assert_eq!(removed.state, AccountState::Removed);
    assert!(
        r.reauthenticate_api(
            a.id,
            removed.identity_generation,
            ApiKeyInput::Stored("fixture-six".into()),
            true
        )
        .is_err()
    );
    assert_eq!(r.list(|d| d.id == b.id).unwrap().1.len(), 1);
    assert!(r.enrollment_actor(b.id).unwrap().is_none());
}
#[test]
fn legacy_api_migration_rejects_invalid_inputs_without_environment_access() {
    let (_dir, r) = local();
    for (endpoint, transport, variable) in [
        ("http://127.0.0.1:9/v1", Transport::ChatgptOauth, "X"),
        (
            "https://example.invalid?secret=x",
            Transport::OpenaiChat,
            "X",
        ),
        ("https://example.invalid", Transport::OpenaiChat, "bad=name"),
        ("https://example.invalid", Transport::OpenaiChat, "1INVALID"),
        ("https://example.invalid", Transport::OpenaiChat, ""),
    ] {
        assert!(
            r.migrate_legacy_api(endpoint.into(), transport, variable.into())
                .is_err()
        );
    }
    assert!(r.connections().unwrap().is_empty());
    assert!(r.list(|_| true).unwrap().1.is_empty());
}
#[test]
fn unknown_account_and_invalid_input_cannot_publish_records() {
    let (_dir, r) = local();
    let c = r.ensure_api_connection("anthropic").unwrap();
    for input in ["", "has\nnewline", "has\rreturn"] {
        assert!(
            r.add_api(
                c.id,
                "fixture".into(),
                "Fixture".into(),
                ApiKeyInput::Stored(input.into())
            )
            .is_err()
        );
    }
    for alias in ["", "has\nnewline"] {
        assert!(
            r.add_api(
                c.id,
                alias.into(),
                "Fixture".into(),
                ApiKeyInput::Stored("fixture-key".into())
            )
            .is_err()
        );
    }
    let unknown = Uuid::new_v4();
    assert!(r.rename(unknown, "alias".into(), "label".into()).is_err());
    assert!(r.logout(unknown, false).is_err());
    assert!(r.freeze(unknown, Transport::Anthropic).is_err());
    assert!(
        r.reauthenticate_api(unknown, 1, ApiKeyInput::Stored("fixture-key".into()), true)
            .is_err()
    );
    assert!(r.list(|_| true).unwrap().1.is_empty());
}
