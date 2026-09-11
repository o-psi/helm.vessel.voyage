//! Provider-only projection: never mutate canonical messages or persist raster bytes.
use super::{Message, Role};
use crate::{artifacts, provider::ProviderError};
use voyage_protocol::{content::ContentPart, tool_result::ToolContent};

fn invalid(message: &'static str) -> ProviderError {
    ProviderError::Request(message.into())
}

/// Keep a bounded newest suffix of image occurrences. Older images become explicit
/// omissions in the request only, retaining their identity and tool-call provenance.
/// The newest image-bearing message must fit in full; never hide an oversized result.
pub(crate) fn project(
    messages: &mut [Message],
    scope: Option<&artifacts::Scope>,
) -> Result<(), ProviderError> {
    let newest = messages
        .iter()
        .rposition(|m| image_sizes(m).next().is_some());
    let mut count = 0usize;
    let mut bytes = 0u64;
    let mut store = None;
    let mut cutoff = false;
    for (index, message) in messages.iter_mut().enumerate().rev() {
        let sizes: Vec<_> = image_sizes(message).collect();
        if sizes.is_empty() {
            continue;
        }
        let size = sizes
            .iter()
            .try_fold(0u64, |n, size| n.checked_add(*size))
            .ok_or_else(|| invalid("image projection byte count overflow"))?;
        let fits = !cutoff
            && count.saturating_add(sizes.len()) <= 4
            && bytes
                .checked_add(size)
                .is_some_and(|n| n <= crate::images::MAX_BYTES as u64);
        if !fits {
            if Some(index) == newest {
                return Err(invalid(
                    "newest image result exceeds four images or aggregate 2 MiB limit",
                ));
            }
            cutoff = true;
            for part in &mut message.parts {
                if let ContentPart::Image { attachment } = part {
                    *part = ContentPart::Text {
                        text: format!(
                            "[historical image omitted from provider projection: {}]",
                            attachment.id
                        ),
                    };
                }
            }
            if let Some(output) = &mut message.tool_output {
                for part in &mut output.content {
                    if let ToolContent::Image { artifact } = part {
                        *part = ToolContent::Text {
                            text: format!(
                                "[historical tool image omitted from provider projection: {} sha256:{}]",
                                artifact.id, artifact.sha256
                            ),
                        };
                    }
                }
                message.content = output.text_fallback();
            }
            message.image_data.clear();
            continue;
        }
        count += sizes.len();
        bytes += size;
        if let Some(output) = &message.tool_output {
            if message.role != Role::Tool || !message.parts.is_empty() {
                return Err(invalid(
                    "visual tool output requires an unambiguous tool message",
                ));
            }
            for part in &output.content {
                if let ToolContent::Image { artifact } = part {
                    if store.is_none() {
                        let scope = scope.ok_or_else(|| {
                            invalid("visual tool output has no authorized session artifact scope")
                        })?;
                        store = Some(
                            artifacts::Store::open(&scope.directory, scope.session)
                                .map_err(|_| invalid("session artifact storage unavailable"))?,
                        );
                    }
                    let data = store.as_ref().unwrap().resolve(artifact)
                        .map_err(|_| invalid("visual tool artifact failed session authorization or integrity validation"))?;
                    // Decode before retaining bytes, even if a provider is never called.
                    crate::provider::multimodal::tool_image(artifact, &data)?;
                    message.image_data.insert(artifact.id, data);
                }
            }
        }
    }
    Ok(())
}
fn image_sizes(message: &Message) -> impl Iterator<Item = u64> + '_ {
    message
        .parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Image { attachment } => Some(attachment.byte_size),
            _ => None,
        })
        .chain(
            message
                .tool_output
                .iter()
                .flat_map(|o| &o.content)
                .filter_map(|p| match p {
                    ToolContent::Image { artifact } => Some(artifact.byte_size),
                    _ => None,
                }),
        )
}

#[cfg(test)]
mod checks {
    use super::*;
    use voyage_protocol::tool_result::ToolOutput;
    #[test]
    fn authorized_hydration_projection_and_canonical_references() {
        let directory = tempfile::tempdir().unwrap();
        let scope = artifacts::Scope {
            directory: directory.path().to_path_buf(),
            session: uuid::Uuid::new_v4(),
        };
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        let mut store = artifacts::Store::open(&scope.directory, scope.session).unwrap();
        let reference = store
            .put(
                uuid::Uuid::new_v4(),
                "screenshot.png",
                "image/png",
                buffer.get_ref(),
            )
            .unwrap();
        let output = ToolOutput {
            content: vec![ToolContent::Image {
                artifact: reference.clone(),
            }],
            ..Default::default()
        };
        let mut message = Message::tool("call-original", output.text_fallback());
        message.tool_output = Some(Box::new(output));
        let canonical = vec![message; 5];
        let stored = serde_json::to_string(&canonical).unwrap();
        let mut projected = canonical.clone();
        project(&mut projected, Some(&scope)).unwrap();
        assert!(projected[0].image_data.is_empty());
        assert!(
            projected[0]
                .content
                .contains("historical tool image omitted")
        );
        for message in &projected[1..] {
            assert_eq!(message.role, Role::Tool);
            assert_eq!(message.tool_call_id.as_deref(), Some("call-original"));
            assert_eq!(message.image_data[&reference.id], *buffer.get_ref());
        }
        assert_eq!(serde_json::to_string(&canonical).unwrap(), stored);
        let mut resumed: Vec<Message> = serde_json::from_str(&stored).unwrap();
        project(&mut resumed, Some(&scope)).unwrap();
        assert_eq!(resumed[4].image_data[&reference.id], *buffer.get_ref());
        assert!(project(&mut canonical.clone(), None).is_err());
        let other = tempfile::tempdir().unwrap();
        let wrong = artifacts::Scope {
            directory: other.path().to_path_buf(),
            session: uuid::Uuid::new_v4(),
        };
        assert!(project(&mut canonical.clone(), Some(&wrong)).is_err());
        let mut corrupt = canonical.clone();
        if let ToolContent::Image { artifact } =
            &mut corrupt[4].tool_output.as_mut().unwrap().content[0]
        {
            artifact.sha256 = "0".repeat(64);
        }
        assert!(project(&mut corrupt, Some(&scope)).is_err());
        // Branch copy retains exact reference identity but authorizes the destination session.
        let branch = tempfile::tempdir().unwrap();
        let branch_scope = artifacts::Scope {
            directory: branch.path().to_path_buf(),
            session: uuid::Uuid::new_v4(),
        };
        let mut destination =
            artifacts::Store::open(&branch_scope.directory, branch_scope.session).unwrap();
        assert_eq!(
            store.copy_to(&mut destination, &reference).unwrap(),
            reference
        );
        project(&mut canonical.clone(), Some(&branch_scope)).unwrap();
    }
}
