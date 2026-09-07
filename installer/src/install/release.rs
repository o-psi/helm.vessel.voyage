use super::files;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
pub const BINARIES: [&str; 4] = ["helm", "vessel", "voyage", "voyage-installer"];
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub target: String,
    pub binaries: BTreeMap<String, Binary>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Binary {
    pub sha256: String,
}
impl Manifest {
    pub fn inspect(bin: &Path) -> Result<Self> {
        files::safe(bin)?;
        let mut binaries = BTreeMap::new();
        for name in BINARIES {
            let p = bin.join(name);
            files::safe(&p)?;
            let m = fs::symlink_metadata(&p)?;
            ensure!(
                m.is_file() && m.mode() & 0o111 != 0 && m.mode() & 0o6022 == 0,
                "Invalid release executable {name}"
            );
            binaries.insert(
                name.into(),
                Binary {
                    sha256: files::hash(&p)?,
                },
            );
        }
        let manifest = bin.parent().unwrap_or(bin).join("release.json");
        if fs::symlink_metadata(&manifest).is_ok() {
            let m: Self = serde_json::from_slice(&files::read(&manifest, 65536)?)?;
            m.validate()?;
            for (name, b) in &binaries {
                ensure!(
                    m.binaries.get(name).is_some_and(|v| v.sha256 == b.sha256),
                    "Release checksum mismatch for {name}"
                );
            }
            Ok(m)
        } else {
            Ok(Self {
                schema_version: 1,
                version: super::version::inspect(bin)?,
                target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                binaries,
            })
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "Unsupported release manifest schema"
        );
        ensure!(
            !self.version.is_empty()
                && self.version.len() <= 128
                && !self.version.chars().any(char::is_control),
            "Invalid release version"
        );
        ensure!(
            self.binaries.len() == 4 && BINARIES.iter().all(|n| self.binaries.contains_key(*n)),
            "Release must contain all four binaries"
        );
        let arch = std::env::consts::ARCH;
        ensure!(
            [
                format!("linux-{arch}"),
                format!("{arch}-unknown-linux-gnu"),
                format!("{arch}-unknown-linux-musl")
            ]
            .contains(&self.target),
            "Release target does not match this Linux host"
        );
        ensure!(
            self.binaries
                .values()
                .all(|b| b.sha256.len() == 64 && b.sha256.bytes().all(|c| c.is_ascii_hexdigit())),
            "Invalid release hash"
        );
        Ok(())
    }
    pub fn id(&self) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)))
    }
    pub fn verify(&self, root: &Path) -> Result<()> {
        self.validate()?;
        files::safe(root)?;
        let owned = fs::symlink_metadata(root)?;
        ensure!(
            owned.uid() == unsafe { libc::geteuid() } && owned.mode() & 0o077 == 0,
            "Installed release directory must be private and owned"
        );
        for (name, b) in &self.binaries {
            let path = root.join("bin").join(name);
            files::safe(&path)?;
            let m = fs::symlink_metadata(&path)?;
            ensure!(
                m.is_file()
                    && m.uid() == unsafe { libc::geteuid() }
                    && m.mode() & 0o700 == 0o700
                    && m.mode() & 0o6077 == 0,
                "Installed executable permissions changed"
            );
            ensure!(
                files::hash(&root.join("bin").join(name))? == b.sha256,
                "Installed release changed: {name}"
            );
        }
        Ok(())
    }
    pub fn stage(&self, source: &Path, root: &Path) -> Result<()> {
        if root.exists() {
            let old: Self =
                serde_json::from_slice(&files::read(&root.join("release.json"), 65536)?)?;
            ensure!(old.id()? == self.id()?, "Release identity conflict");
            return self.verify(root);
        }
        let staging = root.with_extension("staging");
        files::directory(&staging)?;
        files::directory(&staging.join("bin"))?;
        for (name, b) in &self.binaries {
            let dest = staging.join("bin").join(name);
            if dest.exists() {
                ensure!(
                    files::hash(&dest)? == b.sha256,
                    "Incomplete stage contains changed binary"
                );
                continue;
            }
            let bytes = files::read(&source.join(name), 1024 * 1024 * 1024)?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == b.sha256,
                "Source binary changed during installation"
            );
            let temporary = dest.with_extension(format!(
                "copy-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            files::write_new(&temporary, &bytes)?;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
            fs::hard_link(&temporary, &dest)?;
            fs::remove_file(&temporary)?;
            files::sync(&dest)?;
        }
        let metadata = staging.join("release.json");
        if !metadata.exists() {
            let temporary = metadata.with_extension(format!(
                "json-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            files::write_new(&temporary, &serde_json::to_vec_pretty(self)?)?;
            fs::hard_link(&temporary, &metadata)?;
            fs::remove_file(&temporary)?;
            files::sync(&metadata)?;
        }
        let staged: Self = serde_json::from_slice(&files::read(&metadata, 65536)?)?;
        ensure!(
            staged.id()? == self.id()?,
            "Staged release metadata changed"
        );
        self.verify(&staging)?;
        fs::rename(&staging, root)?;
        files::sync(root)
    }
}
