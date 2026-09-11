use super::*;
use std::future::ready;

fn image() -> Image {
    let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    Image::from_bytes("pixel.png".into(), &bytes).unwrap()
}
fn submit(text: &str) -> VoyageCommand {
    VoyageCommand::Submit {
        coordination: None,
        command_id: Uuid::from_u128(74),
        expected_revision: 9,
        expires_at_ms: 100,
        prompt: text.into(),
    }
}
fn editor(text: &str) -> composer::Composer {
    let mut draft = composer::Composer::default();
    draft.set_text(text.into());
    draft
}
fn view() -> View {
    View::new(voyage_protocol::vessel::ProcessInfo {
        archive: None,
        deletion: None,
        session_id: Uuid::from_u128(1),
        incarnation: Uuid::from_u128(2),
        workspace: "/tmp".into(),
        state: voyage_protocol::process::ProcessState::Live,
        name: None,
    })
}

#[test]
fn private_roundtrip_and_public_payload_never_expose_bytes() {
    let image = image();
    let restored: Image = serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
    validate_set(std::slice::from_ref(&restored)).unwrap();
    assert_eq!(image, restored);
    assert!(!format!("{image:?}").contains(&image.data_base64));
    let mut draft = editor("describe ");
    draft.insert_image(image.upload_id);
    let command = prepare(
        submit("ignored projection"),
        &draft,
        std::slice::from_ref(&image),
    )
    .unwrap();
    let public = serde_json::to_string(&command).unwrap();
    assert!(!public.contains("data_base64") && !public.contains(&image.data_base64));
    assert_eq!(
        serde_json::to_value(image.metadata()).unwrap()["sha256"],
        image.sha256
    );
}

#[test]
fn sends_real_ordered_parts_and_preserves_literal_marker_text() {
    let first = image();
    let second = image();
    let mut draft = editor("before after [Image 1]");
    draft.cursor = 7;
    let mut images = Vec::new();
    insert_images(&mut draft, &mut images, vec![first.clone()], 7).unwrap();
    insert_images(&mut draft, &mut images, vec![second.clone()], 0).unwrap();
    assert_eq!(images, [second.clone(), first.clone()]);
    assert_eq!(draft.authored_text(), "before after [Image 1]");
    let parts = content(&draft, &images).unwrap();
    assert_eq!(
        parts,
        vec![
            ContentPart::Image {
                attachment: second.metadata()
            },
            ContentPart::Text {
                text: "before ".into()
            },
            ContentPart::Image {
                attachment: first.metadata()
            },
            ContentPart::Text {
                text: "after [Image 1]".into()
            }
        ]
    );
    let command = prepare(submit("not authoritative"), &draft, &images).unwrap();
    let VoyageCommand::SubmitContent { content: sent, .. } = command else {
        panic!("content");
    };
    assert_eq!(parts, sent);
}

#[test]
fn keyboard_delete_removes_private_image_without_changing_authored_text() {
    let mut draft = editor("é before after");
    let mut images = Vec::new();
    insert_images(&mut draft, &mut images, vec![image()], 10).unwrap();
    draft.cursor = draft.markers[0].end;
    draft.backspace();
    sync_images(&draft, &mut images).unwrap();
    assert!(images.is_empty() && draft.markers.is_empty());
    assert_eq!(draft.text, "é before after");
    let legacy = submit(&draft.text);
    assert_eq!(
        serde_json::to_vec(&legacy).unwrap(),
        serde_json::to_vec(&prepare(legacy.clone(), &draft, &images).unwrap()).unwrap()
    );
}

#[test]
fn legacy_private_drafts_migrate_without_changing_pending_command_identity() {
    let images = vec![image(), image()];
    let text = " keep\n";
    let original = VoyageCommand::SubmitContent {
        command_id: Uuid::from_u128(74),
        expected_revision: 9,
        expires_at_ms: 100,
        content: vec![
            ContentPart::Text { text: text.into() },
            ContentPart::Image {
                attachment: images[0].metadata(),
            },
            ContentPart::Image {
                attachment: images[1].metadata(),
            },
        ],
    };
    let before = serde_json::to_vec(&original).unwrap();
    let mut view = view();
    view.draft = restore_draft(text.into(), None, &images).unwrap();
    view.images = images;
    let pending = state::Pending {
        account_host: None,
        command_id: Uuid::from_u128(74),
        incarnation: Uuid::from_u128(2),
        draft: text.into(),
        preserve_draft: false,
        receipt_only: false,
        original: Some(Box::new(original)),
    };
    assert_ne!(view.draft.text, pending.draft);
    assert!(pending_matches(&pending, &view));
    assert_eq!(
        before,
        serde_json::to_vec(pending.original.as_ref().unwrap()).unwrap()
    );
    let restored = restore_draft(
        view.draft.text.clone(),
        Some(view.draft.markers.clone()),
        &view.images,
    )
    .unwrap();
    assert_eq!(restored.markers, view.draft.markers);
    assert_eq!(restored.authored_text(), text);
    view.draft.insert('!');
    assert!(!pending_matches(&pending, &view));
}

#[test]
fn corrupt_ownership_fails_closed_and_image_only_stays_image_only() {
    let images = vec![image()];
    assert!(restore_draft("[Image 1]".into(), Some(vec![]), &images).is_err());
    let draft = restore_draft(String::new(), None, &images).unwrap();
    assert!(draft.authored_text().is_empty());
    assert_eq!(
        content(&draft, &images).unwrap(),
        [ContentPart::Image {
            attachment: images[0].metadata()
        }]
    );
    let mut markers = draft.markers.clone();
    markers[0].id = Uuid::new_v4();
    assert!(restore_draft(draft.text, Some(markers), &images).is_err());
}

#[test]
fn paste_limits_and_active_image_steering_preserve_staged_data() {
    let mut draft = editor("kept");
    let mut images = vec![];
    assert!(
        insert_images(
            &mut draft,
            &mut images,
            (0..5).map(|_| image()).collect(),
            0
        )
        .is_err()
    );
    assert_eq!(draft.text, "kept");
    assert!(images.is_empty());
    insert_images(&mut draft, &mut images, vec![image()], 0).unwrap();
    let before = draft.text.clone();
    let command = VoyageCommand::Steer {
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: 1,
        expires_at_ms: 100,
        run_id: Uuid::new_v4(),
        prompt: "text".into(),
    };
    assert!(prepare(command, &draft, &images).is_err());
    assert_eq!(before, draft.text);
}

#[tokio::test]
async fn uploads_precede_ordered_submission_and_resolution_never_uploads() {
    let images = vec![image(), image()];
    let draft = restore_draft("keep".into(), None, &images).unwrap();
    let command = prepare(submit("keep"), &draft, &images).unwrap();
    let mut seen = Vec::new();
    let result = dispatch_with(Uuid::from_u128(74), command.clone(), &images, |request| {
        let result = match &request {
            VoyageCommand::UploadImage { upload_id, .. } => serde_json::to_value(
                images
                    .iter()
                    .find(|i| i.upload_id == *upload_id)
                    .unwrap()
                    .metadata(),
            )
            .unwrap(),
            VoyageCommand::SubmitContent { .. } => serde_json::json!({"status":"accepted"}),
            _ => panic!("unexpected"),
        };
        seen.push(request);
        ready(Ok(result))
    })
    .await
    .unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(result["status"], "accepted");
    let mut resolve_calls = 0;
    dispatch_with(
        Uuid::from_u128(74),
        VoyageCommand::Resolve {
            command_id: Uuid::from_u128(74),
            original: Some(Box::new(command)),
        },
        &images,
        |request| {
            assert!(matches!(request, VoyageCommand::Resolve { .. }));
            resolve_calls += 1;
            ready(Ok(serde_json::json!({"status":"accepted"})))
        },
    )
    .await
    .unwrap();
    assert_eq!(resolve_calls, 1);
}

#[tokio::test]
async fn upload_failure_never_sends_and_unknown_submit_is_not_retried() {
    let images = vec![image()];
    let draft = restore_draft(String::new(), None, &images).unwrap();
    let command = prepare(submit(""), &draft, &images).unwrap();
    let mut calls = 0;
    let result = dispatch_with(Uuid::from_u128(74), command.clone(), &images, |request| {
        assert!(matches!(request, VoyageCommand::UploadImage { .. }));
        calls += 1;
        ready(Err(anyhow::anyhow!("unsafe raw transport diagnostic")))
    })
    .await
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(result["status"], "rejected");
    assert!(!result.to_string().contains("unsafe raw"));
    let mut calls = 0;
    let result = dispatch_with(Uuid::from_u128(74), command, &images, |request| {
        calls += 1;
        ready(match request {
            VoyageCommand::UploadImage { .. } => {
                Ok(serde_json::to_value(images[0].metadata()).unwrap())
            }
            _ => Err(anyhow::anyhow!("lost reply")),
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(calls, 2);
}
