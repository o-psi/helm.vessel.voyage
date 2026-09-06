use anyhow::{Result, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{
    io::{Read, Write},
    path::{Component, Path},
};
pub(super) const MAX_CATALOG: usize = 1_048_576;
pub(super) fn directory(base: &Path, parts: &[&str], create: bool) -> Result<Option<Dir>> {
    let mut dir = Dir::open_ambient_dir(base, cap_std::ambient_authority())?;
    for part in parts {
        if create {
            match dir.create_dir(part) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
        }
        dir = match dir.open_dir_nofollow(part) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
    }
    Ok(Some(dir))
}
pub(super) fn read(dir: &Dir, name: &str, max: usize) -> Result<Option<Vec<u8>>> {
    let mut opts = OpenOptions::new();
    opts.read(true).follow(FollowSymlinks::No).nonblock(true);
    let f = match dir.open_with(name, &opts) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        f.metadata()?.is_file() && f.metadata()?.len() <= max as u64,
        "input must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    f.take(max as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "input exceeds limit");
    Ok(Some(bytes))
}
pub(super) fn publish(dir: &Dir, name: &str, bytes: &[u8]) -> Result<()> {
    publish_observed(dir, name, bytes, |_| Ok(()))
}
pub(super) fn publish_observed(
    dir: &Dir,
    name: &str,
    bytes: &[u8],
    mut observe: impl FnMut(u8) -> Result<()>,
) -> Result<()> {
    read(dir, name, MAX_CATALOG)?;
    let tmp = format!(".new-{}", uuid::Uuid::new_v4());
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true).follow(FollowSymlinks::No);
    let mut f = dir.open_with(&tmp, &opts)?;
    let result = (|| {
        observe(0)?;
        f.write_all(bytes)?;
        observe(1)?;
        f.sync_all()?;
        observe(2)?;
        dir.rename(&tmp, dir, name)?;
        #[cfg(unix)]
        dir.open(".")?.sync_all()?;
        observe(3)?;
        Ok(())
    })();
    let _ = dir.remove_file(&tmp);
    result
}
pub(super) fn local(path: &Path) -> Result<Vec<u8>> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let dir = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    read(
        &dir,
        path.file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid source path"))?,
        super::MAX_ARCHIVE,
    )?
    .ok_or_else(|| anyhow::anyhow!("source missing"))
}
pub(super) fn pack(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_dir(),
        "package source must be a real directory"
    );
    let root = if let Some(name) = path.file_name() {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Dir::open_ambient_dir(parent, cap_std::ambient_authority())?.open_dir_nofollow(name)?
    } else {
        Dir::open_ambient_dir(path, cap_std::ambient_authority())?
    };
    let raw = read(&root, "manifest.json", super::MAX_ARCHIVE)?
        .ok_or_else(|| anyhow::anyhow!("manifest.json missing"))?;
    let manifest: super::Manifest =
        serde_json::from_slice(&raw).map_err(|_| anyhow::anyhow!("invalid manifest JSON"))?;
    ensure!(
        manifest.contents.len() <= super::MAX_FILES,
        "too many contents"
    );
    let mut files = std::collections::BTreeMap::new();
    for item in &manifest.contents {
        ensure!(super::portable(&item.path), "invalid package path");
        let mut dir = root.try_clone()?;
        let path = Path::new(&item.path);
        for part in path.parent().unwrap().components() {
            let Component::Normal(part) = part else {
                anyhow::bail!("invalid content component")
            };
            dir = dir.open_dir_nofollow(part)?;
        }
        let data = read(
            &dir,
            path.file_name().unwrap().to_str().unwrap(),
            super::MAX_ARCHIVE,
        )?
        .ok_or_else(|| anyhow::anyhow!("content missing"))?;
        files.insert(
            item.path.clone(),
            String::from_utf8(data).map_err(|_| anyhow::anyhow!("content must be UTF-8"))?,
        );
    }
    let mut found = std::collections::BTreeSet::new();
    enumerate(&root, "", &mut found, &mut 0)?;
    let expected = files
        .keys()
        .cloned()
        .chain(std::iter::once("manifest.json".into()))
        .collect();
    ensure!(
        found == expected,
        "package directory contains undeclared or missing files"
    );
    let archive = super::Archive { manifest, files };
    archive.validate()?;
    let bytes = serde_json::to_vec(&archive)?;
    super::Archive::parse(&bytes)?;
    Ok(bytes)
}

fn enumerate(
    dir: &Dir,
    prefix: &str,
    found: &mut std::collections::BTreeSet<String>,
    count: &mut usize,
) -> Result<()> {
    for entry in dir.entries()? {
        let entry = entry?;
        *count += 1;
        ensure!(*count <= 288, "package directory entry limit exceeded");
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 package path"))?;
        let path = format!("{prefix}{name}");
        ensure!(super::portable(&path), "nonportable package path");
        let ty = entry.file_type()?;
        ensure!(
            ty.is_file() || ty.is_dir(),
            "package contains symlink or nonregular input"
        );
        if ty.is_dir() {
            ensure!(
                path.matches('/').count() < 8,
                "package directory nesting exceeds limit"
            );
            enumerate(
                &dir.open_dir_nofollow(&name)?,
                &format!("{path}/"),
                found,
                count,
            )?;
        } else {
            found.insert(path);
        }
        ensure!(
            found.len() <= super::MAX_FILES + 1,
            "package directory contains too many files"
        );
    }
    Ok(())
}
