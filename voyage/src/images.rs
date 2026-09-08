//! Bounded image decoding and session-scoped, private immutable blob storage.
//! Attachment names are labels, never filesystem paths. Diagnostics omit image bytes.
use anyhow::{Result, bail, ensure};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
};
use uuid::Uuid;
use voyage_protocol::content::{
    ContentLimits, ContentPart, ImageAttachment, ImageMediaType, validate_content,
};

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IMAGES: u64 = 128;
const MAX_DIMENSION: u32 = 8192;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub fn validate_parts(parts: &[ContentPart]) -> Result<()> {
    validate_content(
        parts,
        ContentLimits {
            max_parts: 16,
            max_text_bytes: 64 * 1024,
            max_images: 4,
            max_image_bytes: MAX_BYTES as u64,
            max_total_image_bytes: MAX_BYTES as u64,
            max_dimension: MAX_DIMENSION,
            max_pixels: MAX_PIXELS,
        },
        true,
    )?;
    Ok(())
}

/// Concatenate authored text exactly, without separators or normalization.
pub fn text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::Image { .. } => None,
        })
        .collect()
}

/// Check container completeness and exclude animation before invoking a raster decoder.
fn container(bytes: &[u8], format: ImageFormat) -> Result<()> {
    match format {
        ImageFormat::Png => {
            let mut pos = 8usize;
            let mut end = false;
            while pos < bytes.len() {
                ensure!(bytes.len() - pos >= 12, "incomplete PNG container");
                let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into()?) as usize;
                let next = pos
                    .checked_add(12)
                    .and_then(|p| p.checked_add(len))
                    .filter(|p| *p <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("incomplete PNG chunk"))?;
                let expected = u32::from_be_bytes(bytes[next - 4..next].try_into()?);
                ensure!(
                    crc32fast::hash(&bytes[pos + 4..next - 4]) == expected,
                    "invalid PNG checksum"
                );
                let kind = &bytes[pos + 4..pos + 8];
                ensure!(
                    !matches!(kind, b"acTL" | b"fcTL" | b"fdAT"),
                    "animated images are unsupported"
                );
                if kind == b"IEND" {
                    ensure!(len == 0 && next == bytes.len(), "invalid PNG terminator");
                    end = true;
                }
                pos = next;
            }
            ensure!(end, "incomplete PNG container");
        }
        ImageFormat::Jpeg => {
            ensure!(
                bytes.starts_with(&[0xff, 0xd8]) && bytes.ends_with(&[0xff, 0xd9]),
                "incomplete JPEG container"
            );
        }
        ImageFormat::WebP => {
            ensure!(bytes.len() >= 12, "incomplete WebP container");
            let len = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
            ensure!(
                len.checked_add(8) == Some(bytes.len()),
                "incomplete WebP container"
            );
            let mut pos = 12;
            while pos < bytes.len() {
                ensure!(bytes.len() - pos >= 8, "incomplete WebP chunk");
                let kind = &bytes[pos..pos + 4];
                let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into()?) as usize;
                let next = pos
                    .checked_add(8)
                    .and_then(|p| p.checked_add(len))
                    .and_then(|p| p.checked_add(len % 2))
                    .filter(|p| *p <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("incomplete WebP chunk"))?;
                ensure!(
                    !matches!(kind, b"ANIM" | b"ANMF"),
                    "animated images are unsupported"
                );
                if kind == b"VP8X" {
                    ensure!(
                        len == 10 && bytes[pos + 8] & 2 == 0,
                        "invalid or animated WebP image"
                    );
                }
                pos = next;
            }
        }
        _ => bail!("unsupported image signature"),
    }
    Ok(())
}

pub fn validate(bytes: &[u8]) -> Result<(ImageMediaType, u32, u32)> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_BYTES,
        "image byte limit exceeded or image empty"
    );
    let format =
        image::guess_format(bytes).map_err(|_| anyhow::anyhow!("unsupported image signature"))?;
    let media = match format {
        ImageFormat::Png => ImageMediaType::Png,
        ImageFormat::Jpeg => ImageMediaType::Jpeg,
        ImageFormat::WebP => ImageMediaType::WebP,
        _ => bail!("unsupported image signature"),
    };
    container(bytes, format)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|_| anyhow::anyhow!("invalid image header or resource limit"))?;
    let (width, height) = decoder.dimensions();
    ensure!(
        width > 0
            && height > 0
            && width <= MAX_DIMENSION
            && height <= MAX_DIMENSION
            && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "image dimension or pixel limit exceeded"
    );
    ensure!(
        decoder.total_bytes() <= 64 * 1024 * 1024,
        "decoded image allocation limit exceeded"
    );
    // The image JPEG adapter disables strict mode; EOI alone is insufficient.
    if format == ImageFormat::Jpeg {
        let options = zune_core::options::DecoderOptions::default()
            .set_strict_mode(true)
            .jpeg_set_max_scans(32)
            .set_max_width(MAX_DIMENSION as usize)
            .set_max_height(MAX_DIMENSION as usize);
        let mut strict = zune_jpeg::JpegDecoder::new_with_options(
            zune_core::bytestream::ZCursor::new(bytes),
            options,
        );
        strict
            .decode_headers()
            .map_err(|_| anyhow::anyhow!("invalid JPEG headers"))?;
        ensure!(
            strict
                .output_buffer_size()
                .is_some_and(|n| n <= 64 * 1024 * 1024),
            "decoded image allocation limit exceeded"
        );
        strict
            .decode()
            .map_err(|_| anyhow::anyhow!("invalid or incomplete JPEG raster"))?;
    } else {
        image::DynamicImage::from_decoder(decoder)
            .map_err(|_| anyhow::anyhow!("invalid or incomplete image raster"))?;
    }
    Ok((media, width, height))
}

/// Never follow a symlink or block opening a FIFO/device. Errors do not expose paths.
#[cfg(unix)]
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow::anyhow!("cannot open image file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow::anyhow!("cannot inspect image file"))?;
    ensure!(metadata.is_file(), "image input must be a regular file");
    ensure!(
        metadata.len() > 0 && metadata.len() <= MAX_BYTES as u64,
        "image file byte limit exceeded or file empty"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("cannot read image file"))?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_BYTES,
        "image file byte limit exceeded or file empty"
    );
    Ok(bytes)
}
#[cfg(not(unix))]
pub fn read_file(_path: &Path) -> Result<Vec<u8>> {
    bail!("safe image file reads unsupported on this platform")
}

type StoredUpload = (String, String, String, Vec<u8>, Vec<u8>);

pub struct Store {
    connection: Connection,
    session: Uuid,
    #[cfg(unix)]
    path: std::path::PathBuf,
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(unix)]
    directory: std::fs::File,
}
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageStore")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Store {
    #[cfg(not(unix))]
    pub fn open(_directory: &Path, _session: Uuid) -> Result<Self> {
        bail!("private image storage unsupported on this platform")
    }

    #[cfg(unix)]
    pub fn open(directory: &Path, session: Uuid) -> Result<Self> {
        use crate::attachment::journal::{open_private_file, prepare_directory};
        ensure!(!session.is_nil(), "invalid image store session");
        let journal = prepare_directory(directory.to_path_buf())?;
        let dir = prepare_directory(journal.join("images"))?;
        let directory = std::fs::File::open(&dir)?;
        let path = dir.join("images.sqlite3");
        let file = open_private_file(&path)?;
        ensure!(
            file.metadata()?.len() <= MAX_DATABASE_BYTES,
            "image database size limit exceeded"
        );
        check_sidecars(&path)?;
        let connection = Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        let mut store = Self {
            connection,
            session,
            path,
            file,
            directory,
        };
        store.check_files()?;
        store
            .connection
            .busy_timeout(std::time::Duration::from_secs(5))?;
        store.connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;")?;
        let page_size: u64 = store
            .connection
            .query_row("PRAGMA page_size", [], |r| r.get(0))?;
        ensure!(page_size > 0, "invalid image database page size");
        store
            .connection
            .pragma_update(None, "max_page_count", MAX_DATABASE_BYTES / page_size)?;
        let tx = store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS scope (singleton INTEGER PRIMARY KEY CHECK(singleton=1), session TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS images (id TEXT PRIMARY KEY, session TEXT NOT NULL, principal TEXT NOT NULL,
                metadata TEXT NOT NULL, digest BLOB NOT NULL CHECK(length(digest)=32), bytes BLOB NOT NULL CHECK(length(bytes) BETWEEN 1 AND 2097152));")?;
        tx.execute(
            "INSERT OR IGNORE INTO scope VALUES (1, ?1)",
            [session.to_string()],
        )?;
        let owner: String =
            tx.query_row("SELECT session FROM scope WHERE singleton=1", [], |r| {
                r.get(0)
            })?;
        ensure!(
            owner == session.to_string(),
            "image store belongs to another session"
        );
        let (count, size): (u64, u64) = tx.query_row(
            "SELECT count(*), coalesce(sum(length(bytes)),0) FROM images",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            count <= MAX_IMAGES && size <= MAX_DATABASE_BYTES,
            "image store capacity exceeded"
        );
        tx.commit()?;
        store.check_files()?;
        store.directory.sync_all()?;
        std::fs::File::open(journal)?.sync_all()?;
        Ok(store)
    }

    #[cfg(unix)]
    fn check_files(&self) -> Result<()> {
        use crate::attachment::journal::{open_private_file, prepare_directory};
        use std::os::unix::fs::MetadataExt;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid image storage"))?;
        prepare_directory(parent.to_path_buf())?;
        let now = std::fs::symlink_metadata(parent)?;
        let original = self.directory.metadata()?;
        ensure!(
            now.dev() == original.dev() && now.ino() == original.ino(),
            "image storage directory changed"
        );
        let now = open_private_file(&self.path)?.metadata()?;
        let original = self.file.metadata()?;
        ensure!(
            now.dev() == original.dev() && now.ino() == original.ino(),
            "image database changed"
        );
        ensure!(
            now.len() <= MAX_DATABASE_BYTES,
            "image database size limit exceeded"
        );
        check_sidecars(&self.path)
    }
    #[cfg(not(unix))]
    fn check_files(&self) -> Result<()> {
        bail!("private image storage unsupported on this platform")
    }

    pub fn put(
        &mut self,
        principal: Uuid,
        id: Uuid,
        name: &str,
        bytes: &[u8],
    ) -> Result<ImageAttachment> {
        self.check_files()?;
        ensure!(!principal.is_nil(), "invalid image principal");
        let (media_type, width, height) = validate(bytes)?;
        let metadata = ImageAttachment {
            id,
            sha256: hex::encode(Sha256::digest(bytes)),
            name: name.to_owned(),
            media_type,
            byte_size: bytes.len() as u64,
            width,
            height,
        };
        validate_parts(&[ContentPart::Image {
            attachment: metadata.clone(),
        }])?;
        let encoded = serde_json::to_string(&metadata)?;
        let digest = Sha256::digest(bytes).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<StoredUpload> = tx
            .query_row(
                "SELECT session, principal, metadata, digest, bytes FROM images WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((session, owner, meta, hash, data)) = existing {
            ensure!(
                session == self.session.to_string()
                    && owner == principal.to_string()
                    && meta == encoded
                    && hash == digest
                    && data == bytes,
                "image identifier conflicts with immutable attachment"
            );
        } else {
            let (count, size): (u64, u64) = tx.query_row(
                "SELECT count(*), coalesce(sum(length(bytes)),0) FROM images",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(
                count < MAX_IMAGES
                    && size
                        .checked_add(bytes.len() as u64)
                        .is_some_and(|n| n <= MAX_DATABASE_BYTES),
                "image store capacity exceeded"
            );
            tx.execute("INSERT INTO images (id, session, principal, metadata, digest, bytes) VALUES (?1,?2,?3,?4,?5,?6)",
                params![id.to_string(), self.session.to_string(), principal.to_string(), encoded, digest, bytes])?;
        }
        tx.commit()?;
        self.check_files()?;
        Ok(metadata)
    }

    fn load(&self, attachment: &ImageAttachment) -> Result<(Uuid, Vec<u8>)> {
        self.check_files()?;
        validate_parts(&[ContentPart::Image {
            attachment: attachment.clone(),
        }])?;
        let row: Option<(String, String, Vec<u8>, Vec<u8>)> = self.connection.query_row(
            "SELECT principal, metadata, digest, bytes FROM images WHERE id=?1 AND session=?2 AND length(bytes) BETWEEN 1 AND 2097152",
            params![attachment.id.to_string(), self.session.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        let (principal, encoded, digest, bytes) =
            row.ok_or_else(|| anyhow::anyhow!("image attachment unavailable in this session"))?;
        let stored: ImageAttachment = serde_json::from_str(&encoded)
            .map_err(|_| anyhow::anyhow!("invalid stored image metadata"))?;
        ensure!(
            &stored == attachment && attachment.byte_size == bytes.len() as u64,
            "image metadata integrity failure"
        );
        ensure!(
            Sha256::digest(&bytes).as_slice() == digest
                && hex::encode(&digest) == attachment.sha256,
            "image content integrity failure"
        );
        let (media, width, height) = validate(&bytes)?;
        ensure!(
            media == attachment.media_type
                && width == attachment.width
                && height == attachment.height,
            "image raster metadata mismatch"
        );
        let principal =
            Uuid::parse_str(&principal).map_err(|_| anyhow::anyhow!("invalid image ownership"))?;
        ensure!(!principal.is_nil(), "invalid image ownership");
        Ok((principal, bytes))
    }

    pub fn resolve(&self, attachment: &ImageAttachment) -> Result<Vec<u8>> {
        self.load(attachment).map(|(_, bytes)| bytes)
    }

    /// Branch copying preserves the original principal, identifier and display name.
    pub fn copy_to(&self, dest: &mut Store, metadata: &ImageAttachment) -> Result<ImageAttachment> {
        let (principal, bytes) = self.load(metadata)?;
        dest.put(principal, metadata.id, &metadata.name, &bytes)
    }
}

#[cfg(unix)]
fn check_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = std::path::PathBuf::from(name);
        match std::fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                crate::attachment::journal::open_private_file(&sidecar)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => bail!("cannot inspect image database sidecar"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raster(format: ImageFormat) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([255, 64, 0]),
        ));
        let mut buffer = Cursor::new(Vec::new());
        image.write_to(&mut buffer, format).unwrap();
        buffer.into_inner()
    }
    #[test]
    fn signatures_full_rasters_and_truncation() {
        for (format, media) in [
            (ImageFormat::Png, ImageMediaType::Png),
            (ImageFormat::Jpeg, ImageMediaType::Jpeg),
            (ImageFormat::WebP, ImageMediaType::WebP),
        ] {
            let bytes = raster(format);
            assert_eq!(validate(&bytes).unwrap(), (media, 2, 2));
            for len in [0, 1, bytes.len() / 2, bytes.len() - 1] {
                assert!(validate(&bytes[..len]).is_err());
            }
        }
        assert!(validate(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
        assert!(validate(&vec![0; MAX_BYTES + 1]).is_err());
        let mut jpeg = raster(ImageFormat::Jpeg);
        jpeg.truncate(jpeg.len() / 2);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        assert!(
            validate(&jpeg).is_err(),
            "fake EOI must not make truncation valid"
        );
    }
    #[test]
    fn reject_dimensions_animation_and_corrupt_checksums() {
        let mut bytes = raster(ImageFormat::Png);
        bytes[16..20].copy_from_slice(&9000u32.to_be_bytes());
        let checksum = crc32fast::hash(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&checksum.to_be_bytes());
        assert!(validate(&bytes).is_err());
        let mut bytes = raster(ImageFormat::Png);
        bytes[29] ^= 1;
        assert!(validate(&bytes).is_err());
        let mut bytes = raster(ImageFormat::Png);
        let mut chunk = 8u32.to_be_bytes().to_vec();
        chunk.extend_from_slice(b"acTL");
        chunk.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 0]);
        chunk.extend_from_slice(&crc32fast::hash(&chunk[4..]).to_be_bytes());
        bytes.splice(33..33, chunk);
        assert!(
            validate(&bytes)
                .unwrap_err()
                .to_string()
                .contains("animated")
        );
    }
    #[test]
    fn text_order_limits_and_debug_privacy() {
        let mut message = crate::model::Message::new(crate::model::Role::User, " a\n b ");
        message.parts = vec![
            ContentPart::Text {
                text: " a\n".into(),
            },
            ContentPart::Text { text: " b ".into() },
        ];
        assert_eq!(text(&message.parts), message.content);
        assert!(validate_parts(&message.parts).is_ok());
        message
            .image_data
            .insert(Uuid::new_v4(), b"PRIVATE-IMAGE-BYTES".to_vec());
        assert!(!format!("{message:?}").contains("PRIVATE-IMAGE-BYTES"));
        assert!(
            !serde_json::to_string(&message)
                .unwrap()
                .contains("image_data")
        );
    }
    #[cfg(unix)]
    #[test]
    fn store_reopen_conflicts_scope_corruption_and_branch_copy() {
        let root = tempfile::tempdir().unwrap();
        let session = Uuid::new_v4();
        let principal = Uuid::new_v4();
        let id = Uuid::new_v4();
        let bytes = raster(ImageFormat::Png);
        let mut store = Store::open(root.path(), session).unwrap();
        let meta = store.put(principal, id, "pixels.jpg", &bytes).unwrap();
        assert_eq!(
            meta.media_type,
            ImageMediaType::Png,
            "extension is not type authority"
        );
        assert_eq!(
            store.put(principal, id, "pixels.jpg", &bytes).unwrap(),
            meta
        );
        assert!(store.put(principal, id, "changed.png", &bytes).is_err());
        assert!(store.put(Uuid::new_v4(), id, "pixels.jpg", &bytes).is_err());
        assert!(
            store
                .put(principal, Uuid::new_v4(), "../escape.png", &bytes)
                .is_err()
        );
        drop(store);
        assert!(Store::open(root.path(), Uuid::new_v4()).is_err());
        let store = Store::open(root.path(), session).unwrap();
        assert_eq!(store.resolve(&meta).unwrap(), bytes);
        let branch_root = tempfile::tempdir().unwrap();
        let mut branch = Store::open(branch_root.path(), Uuid::new_v4()).unwrap();
        assert!(branch.resolve(&meta).is_err());
        assert_eq!(store.copy_to(&mut branch, &meta).unwrap(), meta);
        assert_eq!(branch.resolve(&meta).unwrap(), bytes);
        store
            .connection
            .execute(
                "UPDATE images SET bytes=zeroblob(length(bytes)) WHERE id=?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(store.resolve(&meta).is_err());
        assert_eq!(branch.resolve(&meta).unwrap(), bytes);
    }
    #[cfg(unix)]
    #[test]
    fn file_reads_refuse_symlinks_fifos_and_byte_overruns() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("image");
        let bytes = raster(ImageFormat::Png);
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(read_file(&file).unwrap(), bytes);
        let link = root.path().join("link");
        symlink(&file, &link).unwrap();
        assert!(read_file(&link).is_err());
        let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_file(&fifo).is_err());
        std::fs::File::create(&file)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(read_file(&file).is_err());
    }
}
