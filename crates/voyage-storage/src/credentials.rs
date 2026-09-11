//! Encrypted connection records. Key custody is external, never a sibling file.
//! The caller retains responsibility for private, bounded, atomic filesystem I/O.
use anyhow::{Result, ensure};
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"VOYAGE-CREDENTIAL-1\0";
pub const OVERHEAD: usize = MAGIC.len() + 12 + 16;

pub fn encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Load a host-local runtime key, not a password and not an environment secret.
/// No cache: removing runtime provisioning locks subsequent reads/writes.
fn key() -> Result<Zeroizing<[u8; 32]>> {
    #[cfg(target_os = "linux")]
    {
        use std::{
            io::Read,
            os::{
                fd::AsRawFd,
                unix::fs::{MetadataExt, OpenOptionsExt},
            },
        };
        let path = std::env::var_os("VOYAGE_CREDENTIAL_KEY_FILE").ok_or_else(|| {
            anyhow::anyhow!(
                "connection storage locked: provision VOYAGE_CREDENTIAL_KEY_FILE on private tmpfs"
            )
        })?;
        let path = std::path::Path::new(&path);
        ensure!(path.is_absolute(), "connection key path must be absolute");
        // Resolve ancestors without accepting symlinks (including the final file).
        let mut current = std::path::PathBuf::new();
        for component in path.components() {
            ensure!(
                !matches!(component, std::path::Component::ParentDir),
                "unsafe connection key path"
            );
            current.push(component);
            ensure!(
                !std::fs::symlink_metadata(&current)
                    .map_err(|_| anyhow::anyhow!("connection key unavailable"))?
                    .file_type()
                    .is_symlink(),
                "unsafe connection key path"
            );
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| anyhow::anyhow!("connection key unavailable"))?;
        let m = file.metadata()?;
        ensure!(
            m.is_file()
                && m.nlink() == 1
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o077 == 0
                && m.len() == 32,
            "invalid private connection key"
        );
        let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        ensure!(
            unsafe { libc::fstatfs(file.as_raw_fd(), fs.as_mut_ptr()) } == 0,
            "connection key filesystem unavailable"
        );
        ensure!(
            unsafe { fs.assume_init() }.f_type == libc::TMPFS_MAGIC,
            "connection key must reside on tmpfs, not beside persistent ciphertext"
        );
        let mut key = Zeroizing::new([0; 32]);
        file.read_exact(&mut *key)
            .map_err(|_| anyhow::anyhow!("connection key unavailable"))?;
        Ok(key)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("encrypted connection storage requires Linux runtime key provisioning")
}

pub fn check_key() -> Result<()> {
    key().map(|_| ())
}

pub fn seal(purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let key = key()?;
    seal_with(&key, purpose, plaintext)
}
fn seal_with(key: &[u8; 32], purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| anyhow::anyhow!("connection encryption unavailable"))?,
    );
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("connection encryption entropy unavailable"))?;
    let mut output = plaintext.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(purpose),
        &mut output,
    )
    .map_err(|_| anyhow::anyhow!("connection encryption failed"))?;
    let mut envelope = Vec::with_capacity(output.len() + MAGIC.len() + nonce.len());
    envelope.extend_from_slice(MAGIC);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&output);
    Ok(envelope)
}

/// Legacy plaintext is readable, but never emitted by seal. Migration is explicit.
pub fn open(purpose: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    if !encrypted(bytes) {
        return Ok(bytes.to_vec());
    }
    let key = key()?;
    open_with(&key, purpose, bytes)
}
fn open_with(key: &[u8; 32], purpose: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        encrypted(bytes) && bytes.len() >= OVERHEAD,
        "invalid encrypted connection record"
    );
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| anyhow::anyhow!("connection encryption unavailable"))?,
    );
    let nonce: [u8; 12] = bytes[MAGIC.len()..MAGIC.len() + 12]
        .try_into()
        .expect("checked header");
    let mut plaintext = bytes[MAGIC.len() + 12..].to_vec();
    let length = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(purpose),
            &mut plaintext,
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "connection storage locked or encrypted record damaged; original retained"
            )
        })?
        .len();
    plaintext.truncate(length);
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_envelope_binds_purpose_key_and_bytes() {
        let key = [7; 32];
        let first = seal_with(&key, b"helm:a.credential", b"synthetic secret").unwrap();
        let second = seal_with(&key, b"helm:a.credential", b"synthetic secret").unwrap();
        assert_ne!(first, second);
        assert!(!first.windows(16).any(|w| w == b"synthetic secret"));
        assert_eq!(
            open_with(&key, b"helm:a.credential", &first).unwrap(),
            b"synthetic secret"
        );
        assert!(open_with(&[8; 32], b"helm:a.credential", &first).is_err());
        assert!(open_with(&key, b"helm:b.credential", &first).is_err());
        for n in 0..first.len() {
            let mut corrupt = first.clone();
            corrupt[n] ^= 1;
            assert!(open_with(&key, b"helm:a.credential", &corrupt).is_err());
        }
        for n in 0..first.len() {
            assert!(open_with(&key, b"helm:a.credential", &first[..n]).is_err());
        }
        assert_eq!(open(b"legacy", b"unchanged").unwrap(), b"unchanged");
    }
}
