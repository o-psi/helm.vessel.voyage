//! Private Helm image drafts; canonical commands contain metadata references only.
mod input;
mod render;
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
    fn metadata(&self) -> ImageAttachment {
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

    fn from_bytes(name: String, bytes: &[u8]) -> Result<Self> {
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

    fn from_path(path: &Path) -> Result<Self> {
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

    fn label(&self, index: usize) -> String {
        let media = match self.media_type {
            ImageMediaType::Png => "image/png",
            ImageMediaType::Jpeg => "image/jpeg",
            ImageMediaType::WebP => "image/webp",
        };
        format!(
            "{}. {} · {} · {} bytes · {}×{}",
            index + 1,
            safe(&self.name),
            media,
            self.byte_size,
            self.width,
            self.height
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

/// Called before saving pending identity. Text-only serialization is unchanged.
pub(super) fn prepare(command: VoyageCommand, images: &[Image]) -> Result<VoyageCommand> {
    if images.is_empty() {
        return Ok(command);
    }
    match command {
        VoyageCommand::Submit {
            command_id,
            expected_revision,
            expires_at_ms,
            prompt,
        } => {
            ensure!(prompt.len() <= 64 * 1024, "Draft limit is 64 KiB");
            validate_set(images)?;
            for image in images {
                wire_bound(&image.upload_command())?;
            }
            let mut content = Vec::with_capacity(images.len() + 1);
            if !prompt.is_empty() {
                content.push(ContentPart::Text { text: prompt });
            }
            content.extend(images.iter().map(|image| ContentPart::Image {
                attachment: image.metadata(),
            }));
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
            "Image steering is unsupported. Wait for the active run to finish or remove images; full draft preserved"
        ),
        // Slash controls do not upload or consume attachments.
        other => Ok(other),
    }
}

pub(super) fn is_image_submission(command: &VoyageCommand) -> bool {
    matches!(command, VoyageCommand::SubmitContent { .. })
}

/// Consume attachments only if the entire frozen draft still matches. Keeping
/// unmatched recovery data is preferable to clearing a newer local draft.
pub(super) fn pending_matches(pending: &state::Pending, view: &View) -> bool {
    if pending.draft != view.draft.text {
        return false;
    }
    let Some(VoyageCommand::SubmitContent { content, .. }) = pending.original.as_deref() else {
        return false;
    };
    let refs: Vec<_> = content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Image { attachment } => Some(attachment.clone()),
            _ => None,
        })
        .collect();
    refs == view.images.iter().map(Image::metadata).collect::<Vec<_>>()
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

#[derive(Clone, Copy)]
enum Destination {
    Live(Target),
    Draft(Uuid),
}

pub(super) struct Modal {
    destination: Destination,
    input: composer::Composer,
    confirm_screenshot: bool,
}

fn push_image(images: &mut Vec<Image>, image: Image) -> Result<()> {
    ensure!(
        images.len() < MAX_IMAGES,
        "At most four images may be attached"
    );
    let total = images.iter().map(|i| i.byte_size).sum::<u64>();
    ensure!(
        total + image.byte_size <= MAX_BYTES as u64,
        "Images together must not exceed 2 MiB"
    );
    images.push(image);
    Ok(())
}

fn edit(images: &mut Vec<Image>, input: &str) -> Result<()> {
    let input = input.trim();
    ensure!(
        !input.is_empty(),
        "Enter a local image path or remove INDEX"
    );
    if let Some(index) = input.strip_prefix("remove ") {
        let index: usize = index
            .trim()
            .parse()
            .context("Use remove followed by a one-based image index")?;
        ensure!(
            index > 0 && index <= images.len(),
            "Image index is out of range"
        );
        images.remove(index - 1);
        Ok(())
    } else {
        ensure!(
            images.len() < MAX_IMAGES,
            "At most four images may be attached"
        );
        let path = input.strip_prefix("add ").unwrap_or(input);
        push_image(images, Image::from_path(Path::new(path))?)
    }
}

impl App {
    fn attachment_images(&self, destination: Destination) -> Result<&[Image]> {
        match destination {
            Destination::Live(target) => Ok(&self
                .views
                .get(&target)
                .context("Selected voyage unavailable")?
                .images),
            Destination::Draft(id) => self.new_draft_images(id),
        }
    }

    fn ensure_images_editable(&self, destination: Destination) -> Result<()> {
        match destination {
            Destination::Live(target) => {
                let view = self.views.get(&target).context("Voyage unavailable")?;
                ensure!(
                    view.pending.is_none(),
                    "Delivery pending; attachments are frozen"
                );
                ensure!(
                    !view.deleted() && !view.archived(),
                    "This voyage is not editable"
                );
                Ok(())
            }
            Destination::Draft(id) => self.ensure_draft_images_editable(id),
        }
    }

    fn save_attachment_edit(&mut self, destination: Destination, images: Vec<Image>) -> Result<()> {
        self.ensure_images_editable(destination)?;
        validate_set(&images)?;
        match destination {
            Destination::Live(target) => {
                let view = self.views.get_mut(&target).context("Voyage unavailable")?;
                let old = std::mem::replace(&mut view.images, images);
                if let Err(error) = drafts::save(&self.clients[target.route], view) {
                    view.images = old;
                    return Err(error.context("Attachment edit was not saved"));
                }
            }
            Destination::Draft(id) => self.set_new_draft_images(id, images)?,
        }
        self.status = "Attachments saved locally · composer text preserved · nothing sent".into();
        Ok(())
    }

    fn change_attachment(&mut self, destination: Destination, input: &str) -> Result<()> {
        // Check frozen state before reading a file or changing the draft.
        self.ensure_images_editable(destination)?;
        let mut images = self.attachment_images(destination)?.to_vec();
        edit(&mut images, input)?;
        self.save_attachment_edit(destination, images)
    }

    fn capture_attachment(&mut self, destination: Destination) -> Result<()> {
        self.ensure_images_editable(destination)?;
        let mut images = self.attachment_images(destination)?.to_vec();
        ensure!(
            images.len() < MAX_IMAGES,
            "Remove an image before taking a screenshot"
        );
        ensure!(
            images.iter().map(|i| i.byte_size).sum::<u64>() < MAX_BYTES as u64,
            "Remove an image to make room for a screenshot"
        );
        // Only reachable after a separate displayed warning and typed CAPTURE.
        // Helper rechecks Helm-local Config policy and bounds capture to 5s/2MiB.
        let bytes = crate::screenshot::capture(true)?;
        push_image(
            &mut images,
            Image::from_bytes("screenshot.png".into(), &bytes)?,
        )?;
        self.save_attachment_edit(destination, images)
    }
}
