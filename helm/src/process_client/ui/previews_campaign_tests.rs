//! Owned-byte decoding and asynchronous preview lifecycle, without terminal IO.
use super::*;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};

fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba(color)))
        .write_to(&mut output, ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
fn source(bytes: Vec<u8>) -> Source {
    Source {
        id: Uuid::new_v4(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
    }
}
fn batch(worker: &Worker, generation: u64) -> Batch {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(batch) = worker.poll() {
            assert_eq!(batch.generation, generation);
            return batch;
        }
        assert!(
            Instant::now() < deadline,
            "bounded preview worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn transparent_owned_pixels_are_flattened_and_large_images_are_bounded() {
    for (color, expected) in [
        ([255, 0, 200, 0], [32, 32, 32]),
        ([100, 150, 200, 255], [100, 150, 200]),
        ([200, 100, 0, 128], [116, 66, 16]),
    ] {
        let decoded = decode::thumbnail(&png(3, 2, color), || false).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        assert!(decoded.to_rgb8().pixels().all(|pixel| pixel.0 == expected));
    }
    let decoded = decode::thumbnail(&png(1024, 256, [1, 2, 3, 255]), || false).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (512, 128));
}

#[test]
fn decoding_cancellation_is_observed_at_each_stage_and_bad_formats_are_explicit() {
    let bytes = png(10, 10, [1, 2, 3, 255]);
    for cancel_at in 1..=4 {
        let calls = std::cell::Cell::new(0);
        let result = decode::thumbnail(&bytes, || {
            calls.set(calls.get() + 1);
            calls.get() == cancel_at
        });
        assert_eq!(result.unwrap_err(), "Preview cancelled");
        assert_eq!(calls.get(), cancel_at);
    }
    for invalid in [
        Vec::new(),
        vec![0; decode::MAX_BYTES + 1],
        b"not an image".to_vec(),
        b"GIF89a\x01\x00\x01\x00".to_vec(),
        b"\x89PNG\r\n\x1a\n".to_vec(),
    ] {
        assert!(decode::thumbnail(&invalid, || false).is_err());
    }
    assert!(
        decode::thumbnail(&png(2049, 2049, [0, 0, 0, 255]), || false)
            .unwrap_err()
            .contains("4 megapixels")
    );
}

#[test]
fn preview_worker_checks_identity_hash_bounds_and_keeps_failure_per_attachment() {
    let config = render::Config::from_env(false, false).unwrap();
    let mut worker = Worker::new(config).unwrap();
    let valid = png(3, 2, [1, 2, 3, 255]);
    let first = source(valid.clone());
    let first_id = first.id;
    let mut wrong_hash = source(valid.clone());
    wrong_hash.sha256 = "0".repeat(64);
    let bad_id = wrong_hash.id;
    let generation = worker
        .request(vec![first, wrong_hash, source(b"not image data".to_vec())])
        .unwrap();
    let result = batch(&worker, generation);
    assert_eq!(result.images.len(), 3);
    assert_eq!(result.images[0].0, first_id);
    assert!(result.images[0].1.is_ok());
    assert_eq!(result.images[1].0, bad_id);
    assert!(
        result.images[1]
            .1
            .as_ref()
            .err()
            .unwrap()
            .contains("integrity")
    );
    assert!(result.images[2].1.is_err());
    assert!(worker.poll().is_none());
    for bad in [
        Source {
            id: Uuid::nil(),
            sha256: "a".repeat(64),
            bytes: valid.clone(),
        },
        Source {
            id: Uuid::new_v4(),
            sha256: "a".repeat(63),
            bytes: valid.clone(),
        },
        Source {
            id: Uuid::new_v4(),
            sha256: "z".repeat(64),
            bytes: valid.clone(),
        },
        source(Vec::new()),
        source(vec![0; decode::MAX_BYTES + 1]),
    ] {
        assert!(
            worker
                .request(vec![bad])
                .unwrap_err()
                .contains("bounds or identity")
        );
    }
    let duplicate = Uuid::new_v4();
    let mut first = source(valid.clone());
    first.id = duplicate;
    let mut second = source(valid.clone());
    second.id = duplicate;
    assert!(worker.request(vec![first, second]).is_err());
    assert!(
        worker
            .request((0..5).map(|_| source(valid.clone())).collect())
            .is_err()
    );
    assert!(
        worker
            .request(vec![source(vec![0; decode::MAX_BYTES]), source(vec![1])])
            .is_err()
    );
    worker.shutdown().unwrap();
    worker.shutdown().unwrap();
    assert_eq!(
        worker.request(vec![source(valid)]).unwrap_err(),
        "Preview worker stopped"
    );
}

#[test]
fn preview_cache_reuses_exact_owned_content_and_clear_reconfigure_invalidate_it() {
    let config = render::Config::from_env(false, false).unwrap();
    let mut worker = Worker::new(config).unwrap();
    let bytes = png(4, 4, [1, 2, 3, 255]);
    let identity = Uuid::new_v4();
    let input = || {
        let mut value = source(bytes.clone());
        value.id = identity;
        value
    };
    let generation = worker.request(vec![input()]).unwrap();
    let first = batch(&worker, generation).images.remove(0).1.unwrap();
    let generation = worker.request(vec![input()]).unwrap();
    let second = batch(&worker, generation).images.remove(0).1.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    worker.clear();
    assert!(worker.poll().is_none());
    let generation = worker.request(vec![input()]).unwrap();
    let third = batch(&worker, generation).images.remove(0).1.unwrap();
    assert!(!Arc::ptr_eq(&first, &third));
    worker.reconfigure(config);
    let generation = worker.request(vec![input()]).unwrap();
    let fourth = batch(&worker, generation).images.remove(0).1.unwrap();
    assert!(!Arc::ptr_eq(&third, &fourth));
    worker.shutdown().unwrap();
}

#[test]
fn newest_preview_generation_wins_and_text_mode_never_claims_display_space() {
    let config = render::Config::from_env(false, false).unwrap();
    let mut worker = Worker::new(config).unwrap();
    let mut latest = 0;
    let mut identity = Uuid::nil();
    for value in 0..24 {
        let input = source(png(32, 16, [value, 2, 3, 255]));
        identity = input.id;
        latest = worker.request(vec![input]).unwrap();
    }
    let result = batch(&worker, latest);
    assert_eq!(result.images[0].0, identity);
    let preview = result.images[0].1.as_ref().unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| {
            assert_eq!(preview.height(80, 24), 0);
            let area = frame.area();
            assert!(!preview.draw(frame, area));
        })
        .unwrap();
    assert!(render::PreparedPreview::new(DynamicImage::new_rgb8(513, 1), &config, 0).is_err());
    assert!(render::PreparedPreview::new(DynamicImage::new_rgb8(1, 1), &config, 4).is_err());
    worker.shutdown().unwrap();
}
