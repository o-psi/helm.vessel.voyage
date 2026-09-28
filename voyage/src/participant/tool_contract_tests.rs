use super::*;
#[test]
fn assignment_arguments_are_strict_and_defaults_do_not_invent_disclosure() {
    for value in [
        json!({"action":"submit","participant":"local","task":"offline"}),
        json!({"action":"observe","participant":"local","assignment_id":Uuid::nil()}),
        json!({"action":"cancel","participant":"local","assignment_id":Uuid::nil()}),
        json!({"action":"list"}),
    ] {
        assert!(serde_json::from_value::<Args>(value.clone()).is_ok());
        let mut extra = value;
        extra["unexpected"] = json!(true);
        if extra["action"] != "list" {
            assert!(serde_json::from_value::<Args>(extra).is_err());
        }
    }
    let action: Args =
        serde_json::from_value(json!({"action":"submit","participant":"local","task":"offline"}))
            .unwrap();
    match action {
        Args::Submit {
            context,
            assignment_id,
            ..
        } => {
            assert!(context.is_empty());
            assert!(assignment_id.is_none());
        }
        _ => panic!("wrong action"),
    }
    for value in [
        json!({"action":"submit","task":"offline"}),
        json!({"action":"retry","assignment_id":"wrong"}),
        json!({"action":"observe","assignment_id":Uuid::nil()}),
    ] {
        assert!(serde_json::from_value::<Args>(value).is_err());
    }
}

async fn offline_participant(root: &std::path::Path) -> ParticipantTool {
    let session = crate::session::Session::new(root.into(), "offline".into());
    let path = root.join("journal");
    let mut journal = crate::attachment::journal::Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(path, session.id).await.unwrap();
    ParticipantTool {
        parent: Arc::new(Parent {
            owner,
            run_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            vessel_id: Uuid::new_v4(),
            endpoints: vec![],
        }),
    }
}

#[tokio::test]
async fn offline_configuration_rejects_invalid_endpoints_before_network_or_monitor() {
    let root = tempfile::tempdir().unwrap();
    let tool = offline_participant(root.path()).await;
    let config = crate::Config::default();
    assert!(
        ParticipantTool::configured(
            tool.parent.owner.clone(),
            tool.parent.run_id,
            tool.parent.principal_id,
            &config
        )
        .await
        .unwrap()
        .is_none()
    );
    let endpoint = ParticipantEndpoint {
        name: "offline".into(),
        credential_file: root.path().join("absent"),
        participant_vessel_id: Uuid::new_v4(),
        binding_id: Uuid::new_v4(),
        binding_revision: 1,
    };
    for case in 0..8 {
        let mut invalid = endpoint.clone();
        match case {
            0 => invalid.name.clear(),
            1 => invalid.name = "x".repeat(65),
            2 => invalid.credential_file = "relative".into(),
            3 => invalid.participant_vessel_id = Uuid::nil(),
            4 => invalid.binding_id = Uuid::nil(),
            5 => invalid.binding_revision = 0,
            _ => (),
        }
        let config = crate::Config {
            participants: match case {
                6 => vec![endpoint.clone(), endpoint.clone()],
                7 => vec![endpoint.clone(); 17],
                _ => vec![invalid],
            },
            ..Default::default()
        };
        let result = ParticipantTool::configured(
            tool.parent.owner.clone(),
            tool.parent.run_id,
            tool.parent.principal_id,
            &config,
        )
        .await;
        let error = result
            .err()
            .expect("invalid endpoint was admitted")
            .to_string();
        assert!(
            error.contains(if case == 7 {
                "limit"
            } else {
                "invalid participant"
            }),
            "{error}"
        );
    }
}

#[tokio::test]
async fn offline_dispatch_refuses_unknown_endpoint_and_cancelled_or_read_only_delegation() {
    let root = tempfile::tempdir().unwrap();
    let tool = offline_participant(root.path()).await;
    let mut context = crate::tools::reliability_tests::context(root.path());
    assert_eq!(tool.definition().name, "participant");
    assert!(matches!(
        tool.execute(json!({"action":"invalid"}), &context).await,
        Err(ToolError::InvalidArguments(_))
    ));
    for action in ["submit", "observe", "cancel"] {
        let args = if action == "submit" {
            json!({"action":action,"participant":"missing","task":"offline"})
        } else {
            json!({"action":action,"participant":"missing","assignment_id":Uuid::new_v4()})
        };
        let error = tool.execute(args, &context).await.unwrap_err().to_string();
        assert!(error.contains("not locally configured"), "{error}");
    }
    context.cancellation.cancel();
    let args = json!({"action":"submit","participant":"missing","task":"offline"});
    assert!(
        tool.execute(args.clone(), &context)
            .await
            .unwrap_err()
            .to_string()
            .contains("cancellation")
    );
    context.cancellation = Default::default();
    let config = crate::Config {
        access: Some(AccessMode::ReadOnly),
        ..Default::default()
    };
    context.policy = Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    assert!(
        tool.execute(args, &context)
            .await
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
}

#[tokio::test]
async fn offline_approval_refusal_precedes_credential_access_and_missing_credentials_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let mut tool = offline_participant(root.path()).await;
    Arc::get_mut(&mut tool.parent)
        .unwrap()
        .endpoints
        .push(ParticipantEndpoint {
            name: "offline".into(),
            credential_file: root.path().join("never-created"),
            participant_vessel_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            binding_revision: 1,
        });
    let mut context = crate::tools::reliability_tests::context(root.path());
    let config = crate::Config {
        access: Some(AccessMode::Approval),
        ..Default::default()
    };
    context.policy = Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    let args = json!({"action":"submit","participant":"offline","task":"offline"});
    let error = tool
        .execute(args.clone(), &context)
        .await
        .unwrap_err()
        .to_string();
    // The unattended approver refuses before attempting to load credentials.
    assert!(!error.contains("never-created"), "{error}");
    assert!(
        error.contains("denied") || error.contains("approval"),
        "{error}"
    );
    context = crate::tools::reliability_tests::context(root.path());
    for args in [
        args,
        json!({"action":"observe","participant":"offline","assignment_id":Uuid::new_v4()}),
        json!({"action":"cancel","participant":"offline","assignment_id":Uuid::new_v4()}),
    ] {
        assert!(matches!(
            tool.execute(args, &context).await,
            Err(ToolError::Failed(_))
        ));
    }
    assert!(!root.path().join("never-created").exists());
}
