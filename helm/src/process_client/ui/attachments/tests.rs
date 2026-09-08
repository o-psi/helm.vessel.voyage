use super::*;
use crossterm::event::{KeyEvent, KeyEventKind};
use std::future::ready;

fn image() -> Image {
    let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    Image::from_bytes("pixel.png".into(), &bytes).unwrap()
}
fn submit(text: &str) -> VoyageCommand {
    VoyageCommand::Submit {
        command_id: Uuid::from_u128(74),
        expected_revision: 9,
        expires_at_ms: 100,
        prompt: text.into(),
    }
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
fn pending(images: &[Image]) -> state::Pending {
    state::Pending {
        command_id: Uuid::from_u128(74),
        incarnation: Uuid::from_u128(2),
        draft: " keep\n".into(),
        preserve_draft: false,
        receipt_only: false,
        original: Some(Box::new(prepare(submit(" keep\n"), images).unwrap())),
    }
}
fn app() -> App {
    let mut view = view();
    view.images = vec![image()];
    view.draft.text = " keep\n".into();
    view.draft.cursor = view.draft.text.len();
    let target = Target {
        route: 0,
        session: view.process.session_id,
    };
    App {
        attachment_modal: None,
        clients: Vec::new(),
        new_chat_config: None,
        views: BTreeMap::from([(target, view)]),
        selected: Some(target),
        new_drafts: BTreeMap::new(),
        active_draft: None,
        draft_hits: Default::default(),
        sender: mpsc::channel(4).0,
        command_checks: BTreeMap::new(),
        first_send_checks: BTreeMap::new(),
        status: String::new(),
        quit: false,
        help: false,
        archives: false,
        help_scroll: 0,
        explore: None,
        interactions: Default::default(),
        completion: Default::default(),
        inference: Default::default(),
        sidebar: Default::default(),
        terminal_request: None,
    }
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn attachment_private_roundtrip_redacts_debug_and_public_references() {
    let image = image();
    let recovered: Image = serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
    validate_set(&[recovered.clone()]).unwrap();
    assert_eq!(recovered, image);
    let debug = format!("{image:?}");
    assert!(!debug.contains(&image.data_base64));
    assert!(!debug.contains("data_base64"));
    assert!(debug.contains("pixel.png"));
    let private = serde_json::to_value(&image).unwrap();
    assert_eq!(private["data_base64"], image.data_base64);
    assert_eq!(private["upload_id"], image.upload_id.to_string());
    assert_eq!(private["sha256"], image.sha256);
    assert_eq!(image.sha256.len(), 64);
    let public = serde_json::to_value(image.metadata()).unwrap();
    assert_eq!(public["id"], image.upload_id.to_string());
    assert!(public.get("data_base64").is_none());
}

#[test]
fn attachment_text_only_and_image_only_serialization() {
    let before = serde_json::to_vec(&submit(" a\n")).unwrap();
    assert_eq!(
        before,
        serde_json::to_vec(&prepare(submit(" a\n"), &[]).unwrap()).unwrap()
    );
    let image = image();
    for text in ["", " a\n"] {
        let command = prepare(submit(text), &[image.clone()]).unwrap();
        assert!(
            !serde_json::to_string(&command)
                .unwrap()
                .contains("data_base64")
        );
        let VoyageCommand::SubmitContent {
            command_id,
            expected_revision,
            expires_at_ms,
            content,
        } = command
        else {
            panic!("expected content")
        };
        assert_eq!(command_id, Uuid::from_u128(74));
        assert_eq!(expected_revision, 9);
        assert_eq!(expires_at_ms, 100);
        assert_eq!(content.len(), if text.is_empty() { 1 } else { 2 });
        assert_eq!(
            content.last(),
            Some(&ContentPart::Image {
                attachment: image.metadata()
            })
        );
        if !text.is_empty() {
            assert_eq!(content[0], ContentPart::Text { text: text.into() });
        }
    }
}

#[test]
fn attachment_remove_preserves_remaining_uuid_and_composer_text() {
    let mut images = vec![image(), image()];
    let survivor = images[1].clone();
    assert!(edit(&mut images, "remove 0").is_err());
    assert!(edit(&mut images, "remove 3").is_err());
    edit(&mut images, "remove 1").unwrap();
    assert_eq!(images, vec![survivor]);
    edit(&mut images, "remove 1").unwrap();
    assert!(images.is_empty());
    let mut app = app();
    let target = app.selected.unwrap();
    let original = app.views[&target].images.clone();
    assert!(
        app.attachment_input(&Event::Key(KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::CONTROL
        )))
        .unwrap()
    );
    app.attachment_input(&Event::Paste("unused path.png".into()))
        .unwrap();
    app.attachment_input(&key(KeyCode::Esc)).unwrap();
    assert!(app.attachment_modal.is_none());
    assert_eq!(app.views[&target].draft.text, " keep\n");
    assert_eq!(app.views[&target].images, original);
}

#[test]
fn attachment_pending_equality_includes_text_and_all_metadata() {
    let mut view = view();
    view.draft.text = " keep\n".into();
    view.images = vec![image()];
    let p = pending(&view.images);
    assert!(pending_matches(&p, &view));
    view.draft.text.push('x');
    assert!(!pending_matches(&p, &view));
    view.draft.text.pop();
    let id = view.images[0].upload_id;
    view.images[0].upload_id = Uuid::new_v4();
    assert!(!pending_matches(&p, &view));
    view.images[0].upload_id = id;
    view.images[0].name = "another.png".into();
    assert!(!pending_matches(&p, &view));
    view.images.clear();
    assert!(!pending_matches(&p, &view));
}

#[test]
fn attachment_frozen_modal_refuses_file_reads_or_capture_confirmation() {
    let mut app = app();
    let target = app.selected.unwrap();
    let p = pending(&app.views[&target].images);
    app.views.get_mut(&target).unwrap().pending = Some(p);
    app.attachment_input(&key(KeyCode::F(6))).unwrap();
    app.attachment_input(&Event::Paste("/does/not/exist.png".into()))
        .unwrap();
    let error = app.attachment_input(&key(KeyCode::Enter)).unwrap_err();
    assert!(error.to_string().contains("frozen"));
    app.attachment_modal.as_mut().unwrap().input.text = "screenshot".into();
    let error = app.attachment_input(&key(KeyCode::Enter)).unwrap_err();
    assert!(error.to_string().contains("frozen"));
    assert!(!app.attachment_modal.as_ref().unwrap().confirm_screenshot);
    assert_eq!(app.views[&target].draft.text, " keep\n");
}

#[test]
fn attachment_screenshot_requires_separate_typed_confirmation_and_can_cancel() {
    let mut app = app();
    let target = app.selected.unwrap();
    app.attachment_input(&key(KeyCode::F(6))).unwrap();
    app.attachment_input(&Event::Paste("screenshot".into()))
        .unwrap();
    app.attachment_input(&key(KeyCode::Enter)).unwrap();
    assert!(app.attachment_modal.as_ref().unwrap().confirm_screenshot);
    assert!(app.attachment_modal.as_ref().unwrap().input.text.is_empty());
    assert!(app.attachment_input(&key(KeyCode::Enter)).is_err());
    assert!(
        app.attachment_input(&Event::Paste("CAPTURE".into()))
            .is_err()
    );
    let mut repeated = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    repeated.kind = KeyEventKind::Repeat;
    app.attachment_input(&Event::Key(repeated)).unwrap();
    assert!(app.attachment_modal.as_ref().unwrap().confirm_screenshot);
    app.attachment_input(&key(KeyCode::Esc)).unwrap();
    assert!(!app.attachment_modal.as_ref().unwrap().confirm_screenshot);
    assert_eq!(app.views[&target].images.len(), 1);
    assert_eq!(app.views[&target].draft.text, " keep\n");
}

#[test]
fn attachment_limits_and_invalid_metadata_are_rejected() {
    let image = image();
    for name in ["", "../x", "a\\b", "\u{202e}png", "a\nb"] {
        assert!(!valid_name(name));
    }
    assert!(validate_set(&vec![image.clone(); MAX_IMAGES + 1]).is_err());
    assert!(validate_set(&[image.clone(), image.clone()]).is_err());
    let mut wrong = image.clone();
    wrong.width += 1;
    assert!(wrong.validate_saved().is_err());
    wrong = image.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(wrong.validate_saved().is_err());
    wrong = image.clone();
    wrong.data_base64 = "A".repeat(MAX_BASE64_BYTES + 1);
    assert!(wrong.validate_saved().is_err());
    wrong = image.clone();
    wrong.byte_size = MAX_BYTES as u64;
    assert!(validate_limits(&[wrong, image.clone()]).is_err());
    let mut metadata = serde_json::to_value(image.metadata()).unwrap();
    verify_upload(&image, metadata.clone()).unwrap();
    metadata["id"] = Uuid::new_v4().to_string().into();
    assert!(verify_upload(&image, metadata).is_err());
    // Full-sized payload still leaves substantial room in the 4 MiB envelope.
    let command = VoyageCommand::UploadImage {
        upload_id: image.upload_id,
        name: "x".repeat(255),
        data_base64: STANDARD.encode(vec![0; MAX_BYTES]),
    };
    wire_bound(&command).unwrap();
}

#[tokio::test]
async fn attachment_uploads_precede_submission_and_stable_refs_match() {
    let images = vec![image(), image()];
    let command = prepare(submit("hello"), &images).unwrap();
    let mut calls = Vec::new();
    let value = dispatch_with(Uuid::from_u128(74), command, &images, |command| {
        let response = match &command {
            VoyageCommand::UploadImage { upload_id, .. } => serde_json::to_value(
                images
                    .iter()
                    .find(|i| i.upload_id == *upload_id)
                    .unwrap()
                    .metadata(),
            )
            .unwrap(),
            VoyageCommand::SubmitContent { .. } => serde_json::json!({"status":"accepted"}),
            _ => panic!("unexpected command"),
        };
        calls.push(command);
        ready(Ok(response))
    })
    .await
    .unwrap();
    assert_eq!(value["status"], "accepted");
    assert_eq!(calls.len(), 3);
    assert!(matches!(calls[0], VoyageCommand::UploadImage { .. }));
    assert!(matches!(calls[1], VoyageCommand::UploadImage { .. }));
    assert!(matches!(calls[2], VoyageCommand::SubmitContent { .. }));
}

#[tokio::test]
async fn attachment_failed_upload_does_not_submit_and_resolution_never_replays() {
    let images = vec![image(), image()];
    let command = prepare(submit("hello"), &images).unwrap();
    let mut calls = 0;
    let rejected = dispatch_with(Uuid::from_u128(74), command.clone(), &images, |cmd| {
        assert!(matches!(cmd, VoyageCommand::UploadImage { .. }));
        calls += 1;
        ready(if calls == 1 {
            Ok(serde_json::to_value(images[0].metadata()).unwrap())
        } else {
            Err(anyhow::anyhow!("uncertain upload"))
        })
    })
    .await
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["command_id"], Uuid::from_u128(74).to_string());
    assert_eq!(images.len(), 2);
    let resolve = VoyageCommand::Resolve {
        command_id: Uuid::from_u128(74),
        original: Some(Box::new(command)),
    };
    assert!(
        !serde_json::to_string(&resolve)
            .unwrap()
            .contains("data_base64")
    );
    let mut calls = 0;
    dispatch_with(Uuid::from_u128(74), resolve, &images, |cmd| {
        assert!(matches!(cmd, VoyageCommand::Resolve { .. }));
        calls += 1;
        ready(Ok(serde_json::json!({"status":"unknown"})))
    })
    .await
    .unwrap();
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn attachment_uncertain_submit_is_not_replayed() {
    let images = vec![image()];
    let command = prepare(submit("hello"), &images).unwrap();
    let mut calls = 0;
    let result = dispatch_with(Uuid::from_u128(74), command, &images, |cmd| {
        calls += 1;
        ready(match cmd {
            VoyageCommand::UploadImage { .. } => {
                Ok(serde_json::to_value(images[0].metadata()).unwrap())
            }
            VoyageCommand::SubmitContent { .. } => {
                Err(anyhow::anyhow!("connection lost after admission"))
            }
            _ => panic!("unexpected replay"),
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(calls, 2);
}

#[test]
fn attachment_steering_refuses_without_consuming_draft() {
    let images = vec![image()];
    let command = VoyageCommand::Steer {
        command_id: Uuid::from_u128(74),
        expected_revision: 9,
        expires_at_ms: 100,
        run_id: Uuid::from_u128(3),
        prompt: "keep this".into(),
    };
    assert!(
        prepare(command, &images)
            .unwrap_err()
            .to_string()
            .contains("Image steering is unsupported")
    );
    assert_eq!(images.len(), 1);
}
