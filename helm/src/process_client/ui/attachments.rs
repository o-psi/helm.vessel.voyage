//! Private Helm image drafts; canonical commands contain metadata references only.
#[cfg(test)]
mod tests;

use super::*;
use anyhow::{Context, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::{
    content::{ContentPart, ImageAttachment, ImageMediaType},
    vessel::VoyageCommand,
};

pub(super) const MAX_IMAGES: usize = 4;
pub(super) const MAX_BYTES: usize = 2 * 1024 * 1024;
// Reserve ample space for the Vessel envelope, session identity and incarnation.
const MAX_COMMAND_BYTES: usize = voyage_protocol::vessel::MAX_VESSEL_BODY - 64 * 1024;
const MAX_BASE64_BYTES: usize = 4 * MAX_BYTES.div_ceil(3);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    upload_id: Uuid,
    name: String,
    media_type: ImageMediaType,
    byte_size: u64,
    width: u32,
    height: u32,
    sha256: String,
    data_base64: String,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("attachment", &self.metadata())
            .finish_non_exhaustive()
    }
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 255
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c, '/' | '\\' | '\u{061c}' | '\u{200e}'..='\u{200f}'
                    | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{206f}')
        })
}

impl Image {
    pub(super) fn metadata(&self) -> ImageAttachment {
        ImageAttachment {
            id: self.upload_id,
            name: self.name.clone(),
            media_type: self.media_type,
            byte_size: self.byte_size,
            width: self.width,
            height: self.height,
            sha256: self.sha256.clone(),
        }
    }

    pub(super) fn from_bytes(name: String, bytes: &[u8]) -> Result<Self> {
        ensure!(
            valid_name(&name),
            "Image filename is not a safe display label"
        );
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_BYTES,
            "An image must contain 1 byte to 2 MiB"
        );
        let (media_type, width, height) = voyage_runtime::images::validate(bytes)?;
        ensure!(width > 0 && height > 0, "Image dimensions are empty");
        Ok(Self {
            upload_id: Uuid::new_v4(),
            name,
            media_type,
            byte_size: bytes.len() as u64,
            width,
            height,
            sha256: hex::encode(Sha256::digest(bytes)),
            data_base64: STANDARD.encode(bytes),
        })
    }

    pub(super) fn from_path(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("Image path needs a UTF-8 filename")?;
        ensure!(
            valid_name(name),
            "Image filename is not a safe display label"
        );
        // Runtime helper checks regular-file identity and bounds the read itself.
        let bytes = voyage_runtime::images::read_file(path)?;
        Self::from_bytes(name.to_owned(), &bytes)
    }

    fn validate_saved(&self) -> Result<()> {
        ensure!(!self.upload_id.is_nil(), "Saved image identity is empty");
        ensure!(valid_name(&self.name), "Invalid saved image display label");
        ensure!(
            self.byte_size > 0
                && self.byte_size <= MAX_BYTES as u64
                && self.data_base64.len() <= MAX_BASE64_BYTES,
            "Saved image exceeds its byte limit"
        );
        let bytes = STANDARD
            .decode(&self.data_base64)
            .context("Invalid saved image encoding")?;
        ensure!(
            bytes.len() as u64 == self.byte_size,
            "Saved image size mismatch"
        );
        ensure!(
            self.sha256 == hex::encode(Sha256::digest(&bytes)),
            "Saved image digest mismatch"
        );
        ensure!(
            voyage_runtime::images::validate(&bytes)? == (self.media_type, self.width, self.height),
            "Saved image metadata mismatch"
        );
        Ok(())
    }

    fn upload_command(&self) -> VoyageCommand {
        VoyageCommand::UploadImage {
            upload_id: self.upload_id,
            name: self.name.clone(),
            data_base64: self.data_base64.clone(),
        }
    }

    pub(super) fn label(&self, index: usize) -> String {
        let media = match self.media_type {
            ImageMediaType::Png => "image/png",
            ImageMediaType::Jpeg => "image/jpeg",
            ImageMediaType::WebP => "image/webp",
        };
        format!(
            "[Image {}] {} · {} bytes · {}×{} · {}",
            index + 1,
            media,
            self.byte_size,
            self.width,
            self.height,
            safe(&self.name)
        )
    }
}

fn validate_limits(images: &[Image]) -> Result<()> {
    ensure!(
        images.len() <= MAX_IMAGES,
        "At most four images may be attached"
    );
    let total = images
        .iter()
        .try_fold(0u64, |n, image| n.checked_add(image.byte_size))
        .context("Image size overflow")?;
    ensure!(
        total <= MAX_BYTES as u64,
        "Images together must not exceed 2 MiB"
    );
    Ok(())
}

pub(super) fn validate_set(images: &[Image]) -> Result<()> {
    validate_limits(images)?;
    let mut ids = std::collections::BTreeSet::new();
    for image in images {
        ensure!(
            ids.insert(image.upload_id),
            "Duplicate saved image identity"
        );
        image.validate_saved()?;
    }
    Ok(())
}

fn wire_bound(command: &VoyageCommand) -> Result<()> {
    ensure!(
        serde_json::to_vec(command)?.len() < MAX_COMMAND_BYTES,
        "Image command exceeds the bounded transport limit"
    );
    Ok(())
}

/// Resolve only owned inline markers; literal lookalikes remain authored text.
pub(super) fn content(draft: &composer::Composer, images: &[Image]) -> Result<Vec<ContentPart>> {
    composer::validate_markers(&draft.text, &draft.markers).map_err(anyhow::Error::msg)?;
    ensure!(
        draft.markers.len() == images.len(),
        "Image draft marker count mismatch"
    );
    ensure!(
        draft
            .markers
            .iter()
            .zip(images)
            .all(|(m, i)| m.id == i.upload_id),
        "Image draft identity/order mismatch"
    );
    let mut parts = Vec::new();
    for part in draft.ordered_parts() {
        parts.push(match part {
            composer::ComposerPart::Text(text) => ContentPart::Text { text },
            composer::ComposerPart::Image(id) => ContentPart::Image {
                attachment: images
                    .iter()
                    .find(|image| image.upload_id == id)
                    .context("Image marker has no private attachment")?
                    .metadata(),
            },
        });
    }
    Ok(parts)
}

/// Text-only serialization remains byte-for-byte unchanged.
pub(super) fn prepare(
    command: VoyageCommand,
    draft: &composer::Composer,
    images: &[Image],
) -> Result<VoyageCommand> {
    if images.is_empty() {
        return Ok(command);
    }
    match command {
        VoyageCommand::Submit {
            command_id,
            expected_revision,
            expires_at_ms,
            ..
        } => {
            validate_set(images)?;
            let content = content(draft, images)?;
            voyage_runtime::images::validate_parts(&content)?;
            for image in images {
                wire_bound(&image.upload_command())?;
            }
            let command = VoyageCommand::SubmitContent {
                command_id,
                expected_revision,
                expires_at_ms,
                content,
            };
            wire_bound(&command)?;
            Ok(command)
        }
        VoyageCommand::Steer { .. } => anyhow::bail!(
            "Image steering is unsupported. Wait for the active run to finish; full draft preserved"
        ),
        other => Ok(other),
    }
}

/// Migrate old modal drafts without rewriting any immutable pending command.
pub(super) fn restore_draft(
    text: String,
    markers: Option<Vec<composer::ImageMarker>>,
    images: &[Image],
) -> Result<composer::Composer> {
    validate_set(images)?;
    let mut draft = composer::Composer::default();
    draft.set_text(text);
    match markers {
        Some(markers) => draft.restore_markers(markers).map_err(anyhow::Error::msg)?,
        None => {
            for image in images {
                draft.insert_image(image.upload_id);
            }
        }
    }
    content(&draft, images)?;
    Ok(draft)
}

/// Keyboard deletion removes the owned blob from the private draft, and moving
/// insertion points changes image order without changing immutable upload IDs.
pub(super) fn sync_images(draft: &composer::Composer, images: &mut Vec<Image>) -> Result<()> {
    composer::validate_markers(&draft.text, &draft.markers).map_err(anyhow::Error::msg)?;
    ensure!(
        draft
            .markers
            .iter()
            .all(|marker| images.iter().any(|image| image.upload_id == marker.id)),
        "Image marker has no private attachment"
    );
    images.retain(|image| draft.markers.iter().any(|m| m.id == image.upload_id));
    images.sort_by_key(|image| {
        draft
            .markers
            .iter()
            .position(|m| m.id == image.upload_id)
            .unwrap_or(usize::MAX)
    });
    Ok(())
}

pub(super) fn insert_images(
    draft: &mut composer::Composer,
    images: &mut Vec<Image>,
    added: Vec<Image>,
    mut at: usize,
) -> Result<()> {
    let mut all = images.clone();
    all.extend(added.iter().cloned());
    validate_set(&all)?;
    for image in added {
        let id = image.upload_id;
        draft.insert_image_at(at, id);
        at = draft
            .markers
            .iter()
            .find(|m| m.id == id)
            .context("Image insertion failed")?
            .end;
        images.push(image);
    }
    sync_images(draft, images)?;
    ensure!(
        draft.text.len() <= 65536,
        "Composer limit is 64 KiB; image paste not applied"
    );
    Ok(())
}

pub(super) fn is_image_submission(command: &VoyageCommand) -> bool {
    matches!(command, VoyageCommand::SubmitContent { .. })
}

/// Consume attachments only if the entire frozen draft still matches. Keeping
/// unmatched recovery data is preferable to clearing a newer local draft.
pub(super) fn pending_matches(pending: &state::Pending, view: &View) -> bool {
    let Some(VoyageCommand::SubmitContent {
        content: original, ..
    }) = pending.original.as_deref()
    else {
        return false;
    };
    content(&view.draft, &view.images).is_ok_and(|current| current == *original)
}

fn verify_upload(image: &Image, response: serde_json::Value) -> Result<()> {
    let attachment: ImageAttachment =
        serde_json::from_value(response).context("Upload omitted image metadata")?;
    ensure!(
        attachment == image.metadata(),
        "Upload returned different image identity or metadata"
    );
    Ok(())
}

/// Only the original dispatch calls this after durably saving its complete draft.
/// Resolve/Receipt requests bypass uploads, even if they contain original refs.
pub(super) async fn upload_then_submit(
    client: &Client,
    session: Uuid,
    incarnation: Uuid,
    command_id: Uuid,
    command: VoyageCommand,
    images: &[Image],
) -> Result<serde_json::Value> {
    dispatch_with(command_id, command, images, |command| {
        client.voyage(session, incarnation, command)
    })
    .await
}

async fn dispatch_with<F, Fut>(
    command_id: Uuid,
    command: VoyageCommand,
    images: &[Image],
    mut send: F,
) -> Result<serde_json::Value>
where
    F: FnMut(VoyageCommand) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value>>,
{
    if is_image_submission(&command) {
        let uploads: Result<()> = async {
            validate_set(images)?;
            ensure!(
                !images.is_empty(),
                "Image submission has no local image data"
            );
            let VoyageCommand::SubmitContent { content, .. } = &command else {
                unreachable!()
            };
            let refs: Vec<_> = content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Image { attachment } => Some(attachment.clone()),
                    _ => None,
                })
                .collect();
            ensure!(
                refs == images.iter().map(Image::metadata).collect::<Vec<_>>(),
                "Frozen image references do not match the saved draft"
            );
            wire_bound(&command)?;
            for image in images {
                let upload = image.upload_command();
                wire_bound(&upload)?;
                verify_upload(image, send(upload).await?)?;
            }
            Ok(())
        }
        .await;
        if uploads.is_err() {
            // Even an uncertain upload cannot mean the not-yet-sent submission
            // was admitted. Orphan immutable uploads are harmless. Do not echo
            // transport diagnostics that might contain encoded image bytes.
            return Ok(serde_json::json!({
                "status": "rejected", "command_id": command_id,
                "detail": "Image upload failed or returned invalid metadata. Message not submitted; full draft retained."
            }));
        }
    }
    send(command).await
}

impl App {
    pub(super) fn attachment_details(&self) -> Vec<String> {
        let images = if let Some(id) = self.active_draft {
            self.new_draft_images(id).unwrap_or(&[])
        } else {
            self.selected
                .and_then(|t| self.views.get(&t))
                .map_or(&[][..], |v| v.images.as_slice())
        };
        images
            .iter()
            .enumerate()
            .map(|(n, image)| image.label(n))
            .collect()
    }
    pub(super) fn attachment_summary(&self) -> String {
        "Paste: Ctrl+V / Alt+V · image paths also work".into()
    }
}

/// Owned images are styled editor elements, never parsed from lookalike text.
pub(super) fn styled_draft(draft: &composer::Composer) -> ratatui::text::Text<'static> {
    use ratatui::text::{Line, Span, Text};
    let selection = draft.selection();
    let mut boundaries = vec![0, draft.text.len()];
    for marker in &draft.markers {
        boundaries.extend([marker.start, marker.end]);
    }
    if let Some((start, end)) = selection {
        boundaries.extend([start, end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut lines = vec![Line::default()];
    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let mut style = if draft
            .markers
            .iter()
            .any(|m| m.start <= start && end <= m.end)
        {
            crate::theme::Role::Focus.style()
        } else {
            ratatui::style::Style::default()
        };
        if selection.is_some_and(|(a, b)| a <= start && end <= b) {
            style = style.patch(crate::theme::Role::Selection.style());
        }
        let text = safe(&draft.text[start..end]);
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Line::default());
            }
            lines
                .last_mut()
                .expect("line")
                .spans
                .push(Span::styled(line.to_owned(), style));
        }
    }
    Text::from(lines)
}
