//! Ordered multimodal turn contract. References contain no paths or image bytes.
//!
//! Validation here checks metadata only. The executing runtime must separately
//! authorize and resolve references, decode images with resource limits, verify
//! actual type/dimensions/size, and enforce the selected provider's request limit.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPart {
    Text { text: String },
    Image { attachment: ImageAttachment },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageAttachment {
    /// Opaque identifier scoped to its owning session; never a filesystem path.
    pub id: Uuid,
    /// Immutable lowercase SHA-256 of the verified encoded raster bytes.
    pub sha256: String,
    /// Display label only. Must never be used to resolve a file.
    pub name: String,
    pub media_type: ImageMediaType,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageMediaType {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    WebP,
}

/// Local bounds, narrowed by the executing provider's supported limits.
/// Encoded transport overhead needs a separate check on the actual request.
#[derive(Clone, Copy, Debug)]
pub struct ContentLimits {
    pub max_parts: usize,
    pub max_text_bytes: usize,
    pub max_images: usize,
    pub max_image_bytes: u64,
    pub max_total_image_bytes: u64,
    pub max_dimension: u32,
    pub max_pixels: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentError {
    Empty,
    TooManyParts,
    TextLimit,
    ImagesUnsupported,
    TooManyImages,
    InvalidImageMetadata,
    ImageByteLimit,
    TotalImageByteLimit,
    ImageDimensionLimit,
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "add text or an image before sending",
            Self::TooManyParts => "turn has too many content parts",
            Self::TextLimit => "turn text exceeds the byte limit",
            Self::ImagesUnsupported => {
                "select a model with image input support or remove attachments"
            }
            Self::TooManyImages => "remove images to meet the attachment count limit",
            Self::InvalidImageMetadata => "attachment metadata is invalid; attach the image again",
            Self::ImageByteLimit => "image exceeds the byte limit; attach a smaller image",
            Self::TotalImageByteLimit => {
                "attachments exceed the total byte limit; remove or reduce images"
            }
            Self::ImageDimensionLimit => "image exceeds the dimension or pixel limit; resize it",
        })
    }
}

impl std::error::Error for ContentError {}

/// Preserves authored text and order. Unknown capability must be passed as false.
/// This is not proof that an attachment exists, is authorized, or is decodable.
pub fn validate_content(
    parts: &[ContentPart],
    limits: ContentLimits,
    supports_images: bool,
) -> Result<(), ContentError> {
    if parts.len() > limits.max_parts {
        return Err(ContentError::TooManyParts);
    }
    let mut text_bytes = 0usize;
    let mut image_bytes = 0u64;
    let mut images = 0usize;
    let mut has_content = false;
    for part in parts {
        match part {
            ContentPart::Text { text } => {
                text_bytes = text_bytes
                    .checked_add(text.len())
                    .ok_or(ContentError::TextLimit)?;
                if text_bytes > limits.max_text_bytes {
                    return Err(ContentError::TextLimit);
                }
                has_content |= !text.trim().is_empty();
            }
            ContentPart::Image { attachment: image } => {
                if !supports_images {
                    return Err(ContentError::ImagesUnsupported);
                }
                images += 1;
                if images > limits.max_images {
                    return Err(ContentError::TooManyImages);
                }
                if image.id.is_nil()
                    || image.sha256.len() != 64
                    || !image
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || image.name.trim().is_empty()
                    || image.name.len() > 255
                    || image.name.chars().any(|c| {
                        c.is_control()
                            || matches!(c,
                        '/' | '\\' | '\u{061c}' | '\u{200e}'..='\u{200f}' |
                        '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}')
                    })
                    || image.byte_size == 0
                    || image.width == 0
                    || image.height == 0
                {
                    return Err(ContentError::InvalidImageMetadata);
                }
                if image.byte_size > limits.max_image_bytes {
                    return Err(ContentError::ImageByteLimit);
                }
                image_bytes = image_bytes
                    .checked_add(image.byte_size)
                    .ok_or(ContentError::TotalImageByteLimit)?;
                if image_bytes > limits.max_total_image_bytes {
                    return Err(ContentError::TotalImageByteLimit);
                }
                if image.width > limits.max_dimension
                    || image.height > limits.max_dimension
                    || u64::from(image.width) * u64::from(image.height) > limits.max_pixels
                {
                    return Err(ContentError::ImageDimensionLimit);
                }
                has_content = true;
            }
        }
    }
    if !has_content {
        return Err(ContentError::Empty);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ContentLimits {
        ContentLimits {
            max_parts: 8,
            max_text_bytes: 10,
            max_images: 2,
            max_image_bytes: 100,
            max_total_image_bytes: 150,
            max_dimension: 100,
            max_pixels: 1000,
        }
    }

    fn image() -> ContentPart {
        ContentPart::Image {
            attachment: ImageAttachment {
                id: Uuid::from_u128(1),
                sha256: "0".repeat(64),
                name: "screen.png".into(),
                media_type: ImageMediaType::Png,
                byte_size: 100,
                width: 20,
                height: 30,
            },
        }
    }

    #[test]
    fn ordered_roundtrip_preserves_canonical_text() {
        let parts = vec![
            ContentPart::Text {
                text: " a\n".into(),
            },
            image(),
            ContentPart::Text {
                text: "after".into(),
            },
        ];
        let bytes = serde_json::to_vec(&parts).unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<ContentPart>>(&bytes).unwrap(),
            parts
        );
        assert_eq!(validate_content(&parts, limits(), true), Ok(()));
        assert_eq!(validate_content(&[image()], limits(), true), Ok(()));
    }

    #[test]
    fn unsupported_images_do_not_reject_text() {
        assert_eq!(
            validate_content(&[image()], limits(), false),
            Err(ContentError::ImagesUnsupported)
        );
        assert_eq!(
            validate_content(
                &[ContentPart::Text {
                    text: "hello".into()
                }],
                limits(),
                false
            ),
            Ok(())
        );
        assert_eq!(
            validate_content(&[], limits(), true),
            Err(ContentError::Empty)
        );
        assert_eq!(
            validate_content(&[ContentPart::Text { text: " \n".into() }], limits(), true),
            Err(ContentError::Empty)
        );
    }

    #[test]
    fn aggregate_limits_and_overflow_are_rejected() {
        assert_eq!(
            validate_content(&[image(), image()], limits(), true),
            Err(ContentError::TotalImageByteLimit)
        );
        let text = ContentPart::Text {
            text: "ééé".into()
        };
        assert_eq!(
            validate_content(&[text.clone(), text], limits(), true),
            Err(ContentError::TextLimit)
        );
        let mut huge = image();
        let ContentPart::Image { attachment } = &mut huge else {
            unreachable!()
        };
        attachment.byte_size = u64::MAX;
        let limits = ContentLimits {
            max_image_bytes: u64::MAX,
            max_total_image_bytes: u64::MAX,
            ..limits()
        };
        assert_eq!(
            validate_content(&[huge, image()], limits, true),
            Err(ContentError::TotalImageByteLimit)
        );
    }

    #[test]
    fn individual_limits_are_enforced_at_the_boundary() {
        for (limits, expected) in [
            (
                ContentLimits {
                    max_parts: 0,
                    ..limits()
                },
                ContentError::TooManyParts,
            ),
            (
                ContentLimits {
                    max_images: 0,
                    ..limits()
                },
                ContentError::TooManyImages,
            ),
            (
                ContentLimits {
                    max_image_bytes: 99,
                    ..limits()
                },
                ContentError::ImageByteLimit,
            ),
            (
                ContentLimits {
                    max_dimension: 29,
                    ..limits()
                },
                ContentError::ImageDimensionLimit,
            ),
            (
                ContentLimits {
                    max_pixels: 599,
                    ..limits()
                },
                ContentError::ImageDimensionLimit,
            ),
        ] {
            assert_eq!(validate_content(&[image()], limits, true), Err(expected));
        }
        assert_eq!(
            validate_content(
                &[image()],
                ContentLimits {
                    max_parts: 1,
                    max_images: 1,
                    max_image_bytes: 100,
                    max_total_image_bytes: 100,
                    max_dimension: 30,
                    max_pixels: 600,
                    ..limits()
                },
                true
            ),
            Ok(())
        );
    }

    #[test]
    fn untrusted_metadata_and_dimensions_are_rejected() {
        for name in ["", "../secret", "a\\b", "bad\u{1b}[31m", "bad\u{202e}png"] {
            let mut part = image();
            let ContentPart::Image { attachment } = &mut part else {
                unreachable!()
            };
            attachment.name = name.into();
            assert_eq!(
                validate_content(&[part], limits(), true),
                Err(ContentError::InvalidImageMetadata)
            );
        }
        let mut part = image();
        let ContentPart::Image { attachment } = &mut part else {
            unreachable!()
        };
        attachment.width = 100;
        assert_eq!(
            validate_content(&[part], limits(), true),
            Err(ContentError::ImageDimensionLimit)
        );
        assert!(serde_json::from_str::<ImageMediaType>("\"image/svg+xml\"").is_err());
        let mut value = serde_json::to_value(image()).unwrap();
        value["attachment"]["path"] = "/etc/passwd".into();
        assert!(serde_json::from_value::<ContentPart>(value).is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::{
        process::RuntimeCommand,
        vessel::{VoyageCommand, VoyageRequest},
    };

    #[test]
    fn legacy_submit_encoding_is_unchanged() {
        let id = Uuid::from_u128(1);
        let command = RuntimeCommand::Submit {
            coordination: None,
            command_id: id,
            expected_revision: 2,
            expires_at_ms: 3,
            prompt: " a\n".into(),
        };
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            "{\"op\":\"submit\",\"command_id\":\"00000000-0000-0000-0000-000000000001\",\"expected_revision\":2,\"expires_at_ms\":3,\"prompt\":\" a\\n\"}"
        );
    }

    #[test]
    fn upload_bytes_never_appear_in_debug_and_do_not_reserve_turn_commands() {
        let command = VoyageCommand::UploadImage {
            upload_id: Uuid::from_u128(1),
            name: "test.png".into(),
            data_base64: "PRIVATE_BASE64_PIXELS".into(),
        };
        let request = VoyageRequest {
            session_id: Uuid::from_u128(2),
            incarnation: None,
            command,
        };
        assert!(!format!("{request:?}").contains("PRIVATE_BASE64_PIXELS"));
        assert_eq!(request.command.mutation_id(), None);
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["data_base64"], "PRIVATE_BASE64_PIXELS");
        assert!(serde_json::from_value::<VoyageRequest>(encoded).is_ok());
    }

    #[test]
    fn ordered_references_are_strict_and_authorized_as_execution() {
        use crate::process::{ProcessRight, required_process_right};
        let command = RuntimeCommand::SubmitContent {
            command_id: Uuid::from_u128(1),
            expected_revision: 2,
            expires_at_ms: 3,
            content: vec![
                ContentPart::Text {
                    text: "before".into(),
                },
                ContentPart::Text {
                    text: "after".into(),
                },
            ],
        };
        assert_eq!(
            required_process_right(&command),
            Some(ProcessRight::Execute)
        );
        assert!(!command.observes_suspended());
        assert!(command.mutation_id().is_some());
        let mut value = serde_json::to_value(command).unwrap();
        value["prompt"] = "ambiguous".into();
        assert!(serde_json::from_value::<RuntimeCommand>(value).is_err());
    }
}
