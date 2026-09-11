//! Format 2 is executable packaging, never a reinterpretation of format 1 grants.
use super::{digest, hash, identifier, portable};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_ARCHIVE: usize = 8 * 1024 * 1024;
pub const MAX_EXECUTABLE: usize = 6 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub version: String,
    pub voyage: String,
    pub protocol: u32,
    pub platform: String,
    pub runtime: String,
    pub entrypoint: String,
    pub contents: Vec<Content>,
    pub capabilities: Vec<String>,
    #[serde(deserialize_with = "crate::extension_sdk::deserialize_json")]
    pub definitions: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub manifest: Manifest,
    #[serde(deserialize_with = "super::unique_map")]
    pub files: BTreeMap<String, String>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == 2 && identifier(&self.id),
            "unsupported executable package format or identity"
        );
        super::version(&self.version)?;
        ensure!(
            self.voyage == env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0,
            "executable package requires another Voyage compatibility line"
        );
        ensure!(
            self.protocol == 1,
            "unsupported executable extension protocol"
        );
        ensure!(
            self.platform == "linux-x86_64" && self.runtime == "static-elf",
            "unsupported executable platform or runtime"
        );
        // V1 has no external application dependencies or writable resource tree.
        // Additional artifact kinds require a separately reviewed launch profile.
        ensure!(
            self.contents.len() == 1 && portable(&self.entrypoint),
            "static executable package requires exactly one artifact"
        );
        ensure!(
            self.contents[0].path == self.entrypoint && hash(&self.contents[0].sha256),
            "invalid executable entrypoint or digest"
        );
        ensure!(
            self.capabilities == ["execute"] || self.capabilities == ["execute", "host.file.read"],
            "unsupported, duplicate or unordered executable capabilities"
        );
        crate::extension_sdk::validate_definitions(&self.definitions, &self.capabilities)?;
        let definitions =
            crate::extension_sdk::Definitions::parse(&self.definitions, &self.capabilities)?;
        ensure!(
            definitions
                .tools
                .iter()
                .chain(&definitions.commands)
                .chain(&definitions.lifecycle)
                .all(|definition| self.id.len() + definition.name.len() + 9 <= 64),
            "namespaced executable definition exceeds 64 bytes"
        );
        Ok(())
    }
}
impl Archive {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_ARCHIVE,
            "executable archive exceeds 8 MiB"
        );
        let archive: Self = serde_json::from_slice(bytes)
            .map_err(|_| anyhow::anyhow!("invalid executable archive JSON"))?;
        archive.validate()?;
        Ok(archive)
    }
    pub fn validate(&self) -> Result<()> {
        self.manifest.validate()?;
        ensure!(
            self.files.len() == 1,
            "unexpected executable package contents"
        );
        let _ = self.executable()?;
        Ok(())
    }
    /// Return owned, verified bytes. Launch must seal these bytes, never reopen a
    /// command path supplied by the package or rely on a previous path hash.
    pub fn executable(&self) -> Result<Vec<u8>> {
        let encoded = self
            .files
            .get(&self.manifest.entrypoint)
            .ok_or_else(|| anyhow::anyhow!("executable artifact missing"))?;
        ensure!(
            encoded.len() <= MAX_EXECUTABLE.div_ceil(3) * 4,
            "executable artifact exceeds limit"
        );
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| anyhow::anyhow!("invalid executable base64"))?;
        ensure!(
            STANDARD.encode(&bytes) == *encoded && bytes.len() <= MAX_EXECUTABLE,
            "noncanonical or oversized executable base64"
        );
        ensure!(
            self.manifest
                .contents
                .first()
                .is_some_and(|content| digest(&bytes) == content.sha256),
            "executable artifact integrity mismatch"
        );
        validate_elf(&bytes)?;
        Ok(bytes)
    }
}

/// Restrict the initial artifact contract to dependency-free little-endian
/// x86_64 ET_EXEC. The kernel still validates executable semantics at launch;
/// successful structural validation is not execution or publisher trust.
pub(crate) fn validate_elf(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() >= 64 && bytes.len() <= MAX_EXECUTABLE,
        "invalid executable size"
    );
    ensure!(
        &bytes[..7] == b"\x7fELF\x02\x01\x01",
        "executable must be ELF64 little-endian version 1"
    );
    let u16_at = |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
    let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let u64_at = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    ensure!(
        u16_at(16) == 2 && u16_at(18) == 62 && u32_at(20) == 1 && u16_at(52) == 64,
        "unsupported executable type, machine or header"
    );
    let phoff = usize::try_from(u64_at(32)).map_err(|_| anyhow::anyhow!("invalid ELF table"))?;
    let count = usize::from(u16_at(56));
    ensure!(
        u16_at(54) == 56 && (1..=128).contains(&count) && phoff >= 64,
        "invalid ELF program table"
    );
    let table_end = phoff
        .checked_add(count * 56)
        .ok_or_else(|| anyhow::anyhow!("invalid ELF table"))?;
    ensure!(table_end <= bytes.len(), "truncated ELF program table");
    let mut loads = Vec::new();
    let entry = u64_at(24);
    let mut executable_entry = false;
    for index in 0..count {
        let p = phoff + index * 56;
        let kind = u32_at(p);
        ensure!(
            !matches!(kind, 2 | 3),
            "dynamic executable dependencies and interpreters are unsupported"
        );
        let offset = u64_at(p + 8);
        let address = u64_at(p + 16);
        let size = u64_at(p + 32);
        let memory = u64_at(p + 40);
        ensure!(
            offset
                .checked_add(size)
                .is_some_and(|end| end <= bytes.len() as u64),
            "ELF segment outside artifact"
        );
        if kind == 1 {
            let end = address
                .checked_add(memory)
                .ok_or_else(|| anyhow::anyhow!("invalid ELF address range"))?;
            ensure!(
                size <= memory && memory <= 1024 * 1024 * 1024,
                "invalid ELF load size"
            );
            ensure!(
                loads
                    .iter()
                    .all(|&(start, stop)| end <= start || address >= stop),
                "overlapping ELF load segments"
            );
            loads.push((address, end));
            let flags = u32_at(p + 4);
            ensure!(
                flags & !7 == 0 && flags & 3 != 3,
                "unsupported executable segment permissions"
            );
            executable_entry |=
                flags & 1 != 0 && (address..address.saturating_add(size)).contains(&entry);
            let alignment = u64_at(p + 48);
            ensure!(
                alignment <= 1
                    || (alignment.is_power_of_two() && address % alignment == offset % alignment),
                "invalid ELF segment alignment"
            );
        }
    }
    ensure!(
        executable_entry,
        "ELF entrypoint is not in executable artifact bytes"
    );
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(crate) fn elf() -> Vec<u8> {
        let mut b = vec![0; 128];
        b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        b[16..18].copy_from_slice(&2u16.to_le_bytes());
        b[18..20].copy_from_slice(&62u16.to_le_bytes());
        b[20..24].copy_from_slice(&1u32.to_le_bytes());
        b[24..32].copy_from_slice(&0x400078u64.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[52..54].copy_from_slice(&64u16.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&1u32.to_le_bytes());
        b[68..72].copy_from_slice(&5u32.to_le_bytes());
        b[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        b[96..104].copy_from_slice(&128u64.to_le_bytes());
        b[104..112].copy_from_slice(&128u64.to_le_bytes());
        b
    }
    #[test]
    fn static_elf_rejects_interpreter_and_mutated_ranges() {
        let valid = elf();
        assert!(validate_elf(&valid).is_ok());
        for kind in [2u32, 3] {
            let mut b = valid.clone();
            b[64..68].copy_from_slice(&kind.to_le_bytes());
            assert!(validate_elf(&b).is_err());
        }
        for offset in [32, 72, 80, 96, 104] {
            let mut b = valid.clone();
            b[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            assert!(validate_elf(&b).is_err());
        }
        assert!(validate_elf(&valid[..119]).is_err());
        let mut b = valid;
        b[68..72].copy_from_slice(&7u32.to_le_bytes());
        assert!(validate_elf(&b).is_err());
    }
}
