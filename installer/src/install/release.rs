use super::files;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assets: BTreeMap<String, Binary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_compatibility: Option<UpdateCompatibility>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCompatibility {
    pub schema_version: u32,
    pub formats: BTreeMap<String, Vec<u32>>,
    pub implementation_sha256: String,
    pub build_inputs_sha256: String,
}
impl UpdateCompatibility {
    pub fn validate(&self) -> Result<()> {
        let expected = [
            "catalogue_read",
            "catalogue_write",
            "journal_read",
            "journal_write",
            "process_protocol",
            "vessel_protocol",
            "execution_identity",
        ];
        ensure!(
            self.schema_version == 1
                && self.formats.len() == expected.len()
                && expected
                    .iter()
                    .all(|name| self
                        .formats
                        .get(*name)
                        .is_some_and(|values| !values.is_empty()
                            && values.len() <= 64
                            && values.windows(2).all(|w| w[0] < w[1])
                            && values.iter().all(|value| *value > 0 && *value <= 10000)))
                && [&self.implementation_sha256, &self.build_inputs_sha256]
                    .iter()
                    .all(|digest| digest.len() == 64
                        && digest
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
            "invalid source format compatibility contract"
        );
        for (read, write) in [
            ("catalogue_read", "catalogue_write"),
            ("journal_read", "journal_write"),
        ] {
            ensure!(
                self.formats[write]
                    .iter()
                    .all(|value| self.formats[read].contains(value)),
                "writer format is not readable by its source contract"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Binary {
    pub sha256: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Private,
    System,
}

fn mode(path: &Path, expected: u32, directory: bool) -> Result<()> {
    files::safe(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() }
            && metadata.is_dir() == directory
            && !metadata.file_type().is_symlink()
            && metadata.mode() & 0o7777 == expected,
        "installed release permissions changed: {}",
        path.display()
    );
    Ok(())
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
            let m: Self = serde_json::from_slice(&files::read(&manifest, 1024 * 1024)?)?;
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
                assets: BTreeMap::new(),
                update_compatibility: None,
            })
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(contract) = &self.update_compatibility {
            contract.validate()?;
        }
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
        ensure!(self.assets.len() <= 1800, "Too many browser assets");
        for (name, asset) in &self.assets {
            ensure!(
                name.starts_with("share/voyage/browser/")
                    && name.len() <= 512
                    && !name.contains('\\')
                    && name
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
                    && !name.chars().any(char::is_control)
                    && asset.sha256.len() == 64
                    && asset.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid browser asset"
            );
        }
        if !self.assets.is_empty() {
            for name in [
                "worker.mjs",
                "guardian.py",
                "package.json",
                "package-lock.json",
                "node_modules/playwright-core/package.json",
            ] {
                ensure!(
                    self.assets
                        .contains_key(&format!("share/voyage/browser/{name}")),
                    "Incomplete browser assets"
                );
            }
        }
        let arch = std::env::consts::ARCH;
        #[cfg(target_os = "linux")]
        let accepted = vec![
            format!("linux-{arch}"),
            format!("{arch}-unknown-linux-gnu"),
            format!("{arch}-unknown-linux-musl"),
        ];
        #[cfg(target_os = "macos")]
        let accepted = vec![format!("macos-{arch}"), format!("{arch}-apple-darwin")];
        ensure!(
            accepted.contains(&self.target),
            "Release target does not match this host"
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
        // Preserve release identities understood by already-published installers.
        // System reviews pin the complete manifest separately, including its
        // strict compatibility declaration, before any privileged effects.
        let mut identity = self.clone();
        identity.update_compatibility = None;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&identity)?)
        ))
    }
    pub fn verify(&self, root: &Path) -> Result<()> {
        self.verify_access(root, Access::Private)
    }

    /// The system gateway and bound Voyage must read and execute this release;
    /// only root may replace it. This does not admit an existing installation.
    #[allow(dead_code)]
    pub fn verify_system(&self, root: &Path) -> Result<()> {
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "system release requires root"
        );
        mode(Path::new("/opt/voyage"), 0o755, true)?;
        mode(Path::new("/opt/voyage/releases"), 0o755, true)?;
        self.verify_access(root, Access::System)
    }

    fn verify_access(&self, root: &Path, access: Access) -> Result<()> {
        self.validate()?;
        files::safe(root)?;
        let owned = fs::symlink_metadata(root)?;
        ensure!(
            owned.uid() == unsafe { libc::geteuid() }
                && owned.is_dir()
                && (if access == Access::Private {
                    owned.mode() & 0o077 == 0
                } else {
                    owned.mode() & 0o7777 == 0o755
                }),
            "Installed release directory must be private and owned"
        );
        if access == Access::System {
            mode(&root.join("bin"), 0o755, true)?;
        }
        for (name, b) in &self.binaries {
            let path = root.join("bin").join(name);
            files::safe(&path)?;
            let m = fs::symlink_metadata(&path)?;
            ensure!(
                m.is_file()
                    && m.uid() == unsafe { libc::geteuid() }
                    && (if access == Access::Private {
                        m.mode() & 0o700 == 0o700 && m.mode() & 0o6077 == 0
                    } else {
                        m.mode() & 0o7777 == 0o755
                    }),
                "Installed executable permissions changed"
            );
            ensure!(
                files::hash(&root.join("bin").join(name))? == b.sha256,
                "Installed release changed: {name}"
            );
        }
        let mut total = 0u64;
        for (name, asset) in &self.assets {
            let path = root.join(name);
            files::safe(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            total = total.saturating_add(metadata.len());
            ensure!(
                metadata.is_file()
                    && metadata.uid() == unsafe { libc::geteuid() }
                    && (if access == Access::Private {
                        metadata.mode() & 0o6077 == 0
                    } else {
                        metadata.mode() & 0o7777 == 0o644
                    })
                    && total <= 64 * 1024 * 1024,
                "Unsafe installed browser asset"
            );
            ensure!(
                files::hash(&path)? == asset.sha256,
                "Installed browser asset changed"
            );
        }
        if access == Access::System {
            for directory in self.asset_directories(root) {
                mode(&directory, 0o755, true)?;
            }
            mode(&root.join("release.json"), 0o644, false)?;
        }
        Ok(())
    }

    fn asset_directories(&self, root: &Path) -> BTreeSet<std::path::PathBuf> {
        let mut directories = BTreeSet::new();
        for name in self.assets.keys() {
            let mut parent = root.join(name);
            while parent.pop() && parent != root {
                directories.insert(parent.clone());
            }
        }
        directories
    }
    pub fn stage(&self, source: &Path, root: &Path) -> Result<()> {
        self.stage_access(source, root, Access::Private)
    }

    #[allow(dead_code)]
    pub fn stage_system(&self, source: &Path, root: &Path) -> Result<()> {
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "system release requires root"
        );
        self.validate()?;
        let id = self.id()?;
        ensure!(
            root == Path::new("/opt/voyage/releases").join(id),
            "system release must use the protected versioned layout"
        );
        mode(Path::new("/opt/voyage"), 0o755, true)?;
        mode(Path::new("/opt/voyage/releases"), 0o755, true)?;
        self.stage_access(source, root, Access::System)
    }

    fn stage_access(&self, source: &Path, root: &Path, access: Access) -> Result<()> {
        if root.exists() {
            let old: Self =
                serde_json::from_slice(&files::read(&root.join("release.json"), 1024 * 1024)?)?;
            ensure!(old.id()? == self.id()?, "Release identity conflict");
            return self.verify_access(root, access);
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
        let mut total = 0usize;
        for (name, asset) in &self.assets {
            let src = source.parent().unwrap_or(source).join(name);
            files::safe(&src)?;
            let bytes = files::read(&src, 64 * 1024 * 1024)?;
            total = total.saturating_add(bytes.len());
            ensure!(total <= 64 * 1024 * 1024, "Browser assets exceed limit");
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == asset.sha256,
                "Browser source changed"
            );
            let dest = staging.join(name);
            files::directory(dest.parent().unwrap())?;
            if dest.exists() {
                files::safe(&dest)?;
                ensure!(
                    files::hash(&dest)? == asset.sha256,
                    "Incomplete browser asset changed"
                );
            } else {
                files::write_new(&dest, &bytes)?;
                fs::set_permissions(&dest, fs::Permissions::from_mode(0o600))?;
                files::sync(&dest)?;
            }
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
        let staged: Self = serde_json::from_slice(&files::read(&metadata, 1024 * 1024)?)?;
        ensure!(
            staged.id()? == self.id()?,
            "Staged release metadata changed"
        );
        if access == Access::System {
            for directory in self.asset_directories(&staging) {
                fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
            }
            fs::set_permissions(staging.join("bin"), fs::Permissions::from_mode(0o755))?;
            for name in self.binaries.keys() {
                fs::set_permissions(
                    staging.join("bin").join(name),
                    fs::Permissions::from_mode(0o755),
                )?;
            }
            for name in self.assets.keys() {
                fs::set_permissions(staging.join(name), fs::Permissions::from_mode(0o644))?;
            }
            fs::set_permissions(&metadata, fs::Permissions::from_mode(0o644))?;
            fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))?;
            self.verify_system(&staging)?;
            for name in self.binaries.keys() {
                fs::File::open(staging.join("bin").join(name))?.sync_all()?;
            }
            for name in self.assets.keys() {
                fs::File::open(staging.join(name))?.sync_all()?;
            }
            fs::File::open(&metadata)?.sync_all()?;
            fs::File::open(&staging)?.sync_all()?;
        } else {
            self.verify(&staging)?;
        }
        fs::rename(&staging, root)?;
        files::sync(root)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod system_tests {
    use super::*;
    use std::{os::unix::process::CommandExt, process::Command};

    #[test]
    #[ignore = "requires an explicitly disposable native-root Linux fixture"]
    fn root_staged_release_is_readable_by_an_ordinary_runtime_and_retains_browser_assets() {
        assert_eq!(
            std::env::var("VOYAGE_DISPOSABLE_ROOT_FIXTURE").as_deref(),
            Ok("1")
        );
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let root = Path::new("/opt/voyage");
        assert!(
            !root.exists(),
            "fixture must begin without a system installation"
        );
        let source = Path::new("/opt/voyage-system-source-fixture");
        assert!(!source.exists(), "fixture source already exists");
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all("/opt/voyage");
                let _ = fs::remove_dir_all("/opt/voyage-system-source-fixture");
            }
        }
        let _cleanup = Cleanup;
        fs::create_dir_all(root.join("releases")).unwrap();
        fs::set_permissions(root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(root.join("releases"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(source.join("bin")).unwrap();
        let mut binaries = BTreeMap::new();
        for name in BINARIES {
            let path = source.join("bin").join(name);
            fs::copy("/usr/bin/true", &path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            binaries.insert(
                name.into(),
                Binary {
                    sha256: files::hash(&path).unwrap(),
                },
            );
        }
        let mut assets = BTreeMap::new();
        for name in [
            "worker.mjs",
            "guardian.py",
            "package.json",
            "package-lock.json",
            "node_modules/playwright-core/package.json",
        ] {
            let relative = format!("share/voyage/browser/{name}");
            let path = source.join(&relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"fixture browser asset\n").unwrap();
            assets.insert(
                relative,
                Binary {
                    sha256: files::hash(&path).unwrap(),
                },
            );
        }
        let manifest = Manifest {
            schema_version: 1,
            version: "1.0.3-fixture".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            binaries,
            assets,
            update_compatibility: None,
        };
        let release = root.join("releases").join(manifest.id().unwrap());
        manifest
            .stage_system(&source.join("bin"), &release)
            .unwrap();
        manifest.verify_system(&release).unwrap();
        manifest
            .stage_system(&source.join("bin"), &release)
            .unwrap();
        assert!(
            Command::new(release.join("bin/vessel"))
                .uid(1000)
                .gid(1000)
                .status()
                .unwrap()
                .success()
        );
        let read = Command::new("/usr/bin/cat")
            .uid(1000)
            .gid(1000)
            .arg(release.join("share/voyage/browser/worker.mjs"))
            .output()
            .unwrap();
        assert!(read.status.success());
        assert_eq!(read.stdout, b"fixture browser asset\n");
        fs::set_permissions(
            release.join("bin/vessel"),
            fs::Permissions::from_mode(0o775),
        )
        .unwrap();
        assert!(manifest.verify_system(&release).is_err());
        assert!(
            manifest
                .stage_system(&source.join("bin"), &release)
                .is_err()
        );
    }
}
