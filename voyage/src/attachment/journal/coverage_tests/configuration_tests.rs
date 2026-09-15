use super::*;

fn configure(expected_revision: u64) -> RuntimeCommand {
    RuntimeCommand::Configure {
        command_id: Uuid::new_v4(),
        expected_revision,
        expires_at_ms: 61000,
        config_path: "host-private.toml".into(),
    }
}

#[test]
fn initial_settings_are_first_write_wins_and_do_not_change_conversation() {
    let (_root, mut j, s, g) = fixture();
    assert_eq!(j.initial_configuration(s.id).unwrap(), None);
    j.retain_initial_configuration(&g, "first".into()).unwrap();
    j.retain_initial_configuration(&g, "second".into()).unwrap();
    assert_eq!(j.saved_configuration(&g).unwrap().as_deref(), Some("first"));
    assert_eq!(revision(&j, s.id), 0);
    assert!(
        j.retain_initial_configuration(&g, "x".repeat(1024 * 1024 + 1))
            .is_err()
    );
}

#[test]
fn account_materialization_is_compare_and_swap_and_changes_only_account() {
    let (_root, mut j, _s, g) = fixture();
    let before = r#"{"config":{"model":"fixture"},"workspace":"local"}"#;
    let after = r#"{"config":{"model":"fixture","account":{"id":"bound"}},"workspace":"local"}"#;
    j.retain_initial_configuration(&g, before.into()).unwrap();
    assert!(
        j.materialize_account_configuration(&g, before, &after.replace("fixture", "other"))
            .is_err()
    );
    assert_eq!(j.saved_configuration(&g).unwrap().as_deref(), Some(before));
    j.materialize_account_configuration(&g, before, after)
        .unwrap();
    assert!(
        j.materialize_account_configuration(&g, before, after)
            .is_err()
    );
    assert!(
        j.materialize_account_configuration(&g, after, after)
            .is_err()
    );
    assert_eq!(j.saved_configuration(&g).unwrap().as_deref(), Some(after));
}

#[test]
fn configuration_receipt_survives_retry_and_invalidates_old_access_review() {
    let (_root, mut j, s, g) = fixture();
    j.check_access_revision(&g, 0).unwrap();
    assert!(j.check_access_revision(&g, 1).is_err());
    let command = configure(0);
    let receipt = j
        .configure(
            &g,
            command.clone(),
            "private settings".into(),
            "new-model".into(),
            1000,
        )
        .unwrap();
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["apply_at"], "immediate");
    assert_eq!(j.load_session(s.id).unwrap().session.model, "new-model");
    assert!(j.check_access_revision(&g, 0).is_err());
    j.check_access_revision(&g, 1).unwrap();
    assert_eq!(
        j.configure(
            &g,
            command.clone(),
            "ignored on retry".into(),
            "ignored".into(),
            999999
        )
        .unwrap(),
        receipt
    );
    assert_eq!(
        j.saved_configuration(&g).unwrap().as_deref(),
        Some("private settings")
    );
    let mut conflict = command;
    if let RuntimeCommand::Configure { config_path, .. } = &mut conflict {
        *config_path = "different".into();
    }
    assert!(
        j.configure(&g, conflict, "x".into(), "fixture".into(), 1000)
            .is_err()
    );
}

#[test]
fn failed_configuration_is_atomic_for_stale_expired_and_oversized_requests() {
    let (_root, mut j, s, g) = fixture();
    for (command, settings, now) in [
        (configure(1), "x".into(), 1000),
        (configure(0), "x".into(), 61000),
        (configure(0), "x".repeat(1024 * 1024 + 1), 1000),
        (RuntimeCommand::Health, "x".into(), 1000),
    ] {
        assert!(
            j.configure(&g, command, settings, "changed".into(), now)
                .is_err()
        );
        assert_eq!(revision(&j, s.id), 0);
        assert_eq!(j.initial_configuration(s.id).unwrap(), None);
    }
}

#[test]
fn active_run_defers_inference_but_forbids_full_configuration() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let rev = revision(&j, s.id);
    assert!(
        j.configure(&g, configure(rev), "x".into(), "next".into(), 1000)
            .is_err()
    );
    let command = RuntimeCommand::SetInference {
        command_id: Uuid::new_v4(),
        expected_revision: rev,
        expires_at_ms: 61000,
        model: "next".into(),
        reasoning_effort: None,
        service_tier: None,
    };
    let receipt = j
        .configure(&g, command, "next settings".into(), "next".into(), 1000)
        .unwrap();
    assert_eq!(receipt["apply_at"], "next_turn");
    let saved = j.load_session(s.id).unwrap();
    assert_eq!(saved.session.model, "fixture");
    assert_eq!(saved.session.pending_model.as_deref(), Some("next"));
    assert_eq!(j.run(run.id).unwrap().state, RunState::Accepted);
}

#[test]
fn active_access_change_cannot_switch_model() {
    let (_root, mut j, s, g) = fixture();
    admit(&mut j, &g);
    let rev = revision(&j, s.id);
    let command = RuntimeCommand::SetAccess {
        command_id: Uuid::new_v4(),
        expected_revision: rev,
        expires_at_ms: 61000,
        access: "read_only".into(),
    };
    assert!(
        j.configure(&g, command.clone(), "x".into(), "other".into(), 1000)
            .is_err()
    );
    assert_eq!(revision(&j, s.id), rev);
    j.configure(&g, command, "restricted".into(), "fixture".into(), 1000)
        .unwrap();
    assert_eq!(
        j.saved_configuration(&g).unwrap().as_deref(),
        Some("restricted")
    );
}
