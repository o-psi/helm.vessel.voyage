use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

fn authorization(state: &State) -> authorization::Authorization {
    authorization::Authorization {
        scope_source: None,
        browser_history: false,
        authority: None,
        actor: state.actor,
        grant: None,
        owner_connection: false,
    }
}
fn raster() -> String {
    let image =
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([255, 64, 0])));
    let mut buffer = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, image::ImageFormat::Png)
        .unwrap();
    STANDARD.encode(buffer.into_inner())
}

#[tokio::test]
async fn upload_is_idempotent_and_conflicts_never_change_the_original_attachment() {
    let (_root, state) = tests::fixture().await;
    let id = Uuid::new_v4();
    let data = raster();
    let before = state.owner.snapshot().await.unwrap();
    let attachment = images::upload(
        &state,
        authorization(&state),
        id,
        "pixel.png".into(),
        data.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        images::upload(
            &state,
            authorization(&state),
            id,
            "pixel.png".into(),
            data.clone()
        )
        .await
        .unwrap(),
        attachment
    );
    assert!(
        images::upload(
            &state,
            authorization(&state),
            id,
            "changed.png".into(),
            data.clone()
        )
        .await
        .is_err()
    );
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::new_v4(),
            "../escape.png".into(),
            data.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(
        images::upload(&state, authorization(&state), id, "pixel.png".into(), data)
            .await
            .unwrap(),
        attachment
    );
    let after = state.owner.snapshot().await.unwrap();
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.session.messages.len(), after.session.messages.len());
}

#[tokio::test]
async fn above_old_two_mib_boundary_uploads_without_advancing_history() {
    let (_root, state) = tests::fixture().await;
    // A noisy valid raster stays above the old 2 MiB limit without exceeding
    // dimension, pixel, image-store or frame budgets.
    let mut raster = image::RgbImage::new(1100, 1100);
    let mut seed = 1u32;
    for pixel in raster.pixels_mut() {
        for channel in &mut pixel.0 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *channel = (seed >> 16) as u8;
        }
    }
    let mut data = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(raster)
        .write_to(&mut data, image::ImageFormat::Png)
        .unwrap();
    let bytes = data.into_inner();
    assert!(bytes.len() > 2 * 1024 * 1024 && bytes.len() <= crate::images::MAX_BYTES);
    let before = state.owner.snapshot().await.unwrap();
    let attachment = images::upload(
        &state,
        authorization(&state),
        Uuid::new_v4(),
        "camera.png".into(),
        STANDARD.encode(&bytes),
    )
    .await
    .unwrap();
    assert_eq!(attachment["byte_size"], bytes.len() as u64);
    assert_eq!(
        state.owner.snapshot().await.unwrap().revision,
        before.revision
    );
}

#[tokio::test]
async fn invalid_uploads_fail_before_storing_bytes_or_advancing_history() {
    let (_root, state) = tests::fixture().await;
    let before = state.owner.snapshot().await.unwrap().revision;
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::nil(),
            "pixel.png".into(),
            raster()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("identity")
    );
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::new_v4(),
            "pixel.png".into(),
            "a".repeat((crate::images::MAX_BYTES).div_ceil(3) * 4 + 1)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("exceeds limit")
    );
    for data in ["!!!".to_owned(), "YQ".to_owned(), "YQ==\n".to_owned()] {
        assert!(
            images::upload(
                &state,
                authorization(&state),
                Uuid::new_v4(),
                "pixel.png".into(),
                data
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("base64")
        );
    }
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::new_v4(),
            "pixel.png".into(),
            STANDARD.encode(vec![0; crate::images::MAX_BYTES + 1])
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("bounded canonical")
    );
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::new_v4(),
            "pixel.png".into(),
            STANDARD.encode(b"not an image")
        )
        .await
        .is_err()
    );
    assert_eq!(state.owner.snapshot().await.unwrap().revision, before);
    state.shutdown.cancel();
    assert!(
        images::upload(
            &state,
            authorization(&state),
            Uuid::new_v4(),
            "pixel.png".into(),
            raster()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("runtime stopping")
    );
}

#[derive(Debug)]
struct Withdrawn;
impl crate::policy::ExecutionAuthority for Withdrawn {
    fn check(&self) -> Result<()> {
        anyhow::bail!("synthetic authority withdrawn")
    }
}

#[tokio::test]
async fn current_authority_is_checked_before_decoding_image_data() {
    let (_root, state) = tests::fixture().await;
    let mut auth = authorization(&state);
    auth.authority = Some(Arc::new(Withdrawn));
    assert!(
        images::upload(
            &state,
            auth,
            Uuid::new_v4(),
            "pixel.png".into(),
            "not base64".into()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("authority withdrawn")
    );
}
