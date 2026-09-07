use super::{Report, files, release::Manifest};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};
#[derive(Serialize, Deserialize, Default)]
pub(super) struct Journal {
    pub(super) schema_version: u32,
    pub(super) current: Option<String>,
    pub(super) previous: Option<String>,
    pub(super) pending: Option<String>,
}
pub(super) struct Layout {
    pub(super) root: PathBuf,
    pub(super) bin: PathBuf,
}
impl Layout {
    pub(super) fn get() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is required")?);
        files::safe(&home)?;
        Ok(Self {
            root: home.join(".local/share/voyage/install"),
            bin: home.join(".local/bin"),
        })
    }
    pub(super) fn journal(&self) -> Result<Journal> {
        files::safe(&self.root)?;
        if let Ok(meta) = fs::symlink_metadata(&self.root) {
            ensure!(
                meta.is_dir()
                    && meta.uid() == unsafe { libc::geteuid() }
                    && meta.mode() & 0o077 == 0,
                "Installation root must be private and owned"
            );
        }
        let p = self.root.join("transaction.json");
        if !p.exists() {
            return Ok(Journal {
                schema_version: 1,
                ..Default::default()
            });
        }
        let j: Journal = serde_json::from_slice(&files::read(&p, 65536)?)?;
        ensure!(
            j.schema_version == 1,
            "Unsupported installation journal schema"
        );
        for id in [&j.current, &j.previous, &j.pending].into_iter().flatten() {
            ensure!(
                id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid release identity"
            );
        }
        Ok(j)
    }
    pub(super) fn release(&self, id: &str) -> PathBuf {
        self.root.join("releases").join(id)
    }
    pub(super) fn save(&self, j: &Journal) -> Result<()> {
        files::atomic_json(&self.root.join("transaction.json"), j)
    }
    pub(super) fn pointer(&self) -> Result<Option<String>> {
        let p = self.root.join("current");
        match fs::read_link(&p) {
            Ok(target) => {
                let id = target
                    .file_name()
                    .and_then(|v| v.to_str())
                    .context("Invalid current release link")?
                    .to_owned();
                ensure!(
                    target == self.release(&id),
                    "Unrecognized current release link"
                );
                Ok(Some(id))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub(super) fn verify(&self, id: &str) -> Result<Manifest> {
        let root = self.release(id);
        let m: Manifest = serde_json::from_slice(&files::read(&root.join("release.json"), 65536)?)?;
        ensure!(m.id()? == id, "Installed release identity mismatch");
        m.verify(&root)?;
        Ok(m)
    }
    pub(super) fn report(&self, id: &str, version: &str, j: &Journal) -> Result<Report> {
        let current_version = j
            .current
            .as_ref()
            .map(|id| self.verify(id).map(|m| m.version))
            .transpose()?
            .unwrap_or_else(|| "not installed".into());
        Ok(Report {
            release: id.into(),
            release_dir: self.release(id),
            bin_dir: self.bin.clone(),
            changed: j.current.as_deref() != Some(id),
            current_release: j.current.clone(),
            actions: vec![
                format!("Version: {current_version} → {version}"),
                format!("Publish verified release {id}"),
                format!("Expose four commands in {}", self.bin.display()),
                "Preserve configurations, credentials, sessions, backups and previous releases"
                    .into(),
            ],
        })
    }
}
