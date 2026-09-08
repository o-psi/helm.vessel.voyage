//! Decode only already-owned attachment bytes; never open a path or URL.
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::Cursor;

pub(super) const MAX_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_THUMBNAIL: u32 = 512;

/// Cancellation is cooperative between stages. An individual decoder call cannot
/// be interrupted; its input, dimensions and output allocation are admitted below.
pub(super) fn thumbnail(
    bytes: &[u8],
    cancelled: impl Fn() -> bool,
) -> Result<DynamicImage, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Preview input exceeds 2 MiB".into());
    }
    if cancelled() {
        return Err("Preview cancelled".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "Preview format unavailable")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("Preview format unavailable".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    // This library limit is best-effort decoder accounting, not an OS memory cap.
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|_| "Preview header or decoder limit rejected")?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODED_BYTES
    {
        return Err(
            "Preview exceeds 4 megapixels or 32 MiB decoded output; attachment retained".into(),
        );
    }
    if cancelled() {
        return Err("Preview cancelled".into());
    }
    let decoded = DynamicImage::from_decoder(decoder)
        .map_err(|_| "Preview raster unavailable; attachment retained")?;
    if cancelled() {
        return Err("Preview cancelled".into());
    }
    let thumbnail = decoded.thumbnail(MAX_THUMBNAIL.min(width), MAX_THUMBNAIL.min(height));
    drop(decoded);
    if cancelled() {
        return Err("Preview cancelled".into());
    }
    // Flatten only the bounded thumbnail, preserving transparent image appearance
    // without exposing arbitrary RGB values hidden underneath alpha.
    let rgba = thumbnail.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (from, to) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = u16::from(from[3]);
        for channel in 0..3 {
            to[channel] =
                ((u16::from(from[channel]) * alpha + 32 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    Ok(DynamicImage::ImageRgb8(rgb))
}
