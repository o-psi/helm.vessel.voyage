//! Explicit filesystem-skill snapshots adapted to existing declarative packages.
//! Discovery/import never activates instructions or executes supporting files.
use super::{
    Archive, Content, Kind, MAX_ARCHIVE, MAX_FILES, Manifest, digest, identifier, portable, store,
};
use anyhow::{Context, Result, bail, ensure};
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

const MAX_ENTRIES: usize = 128;
const MAX_DEPTH: usize = 8;
const PROVENANCE: &str = "voyage-import.json";

#[derive(Serialize)]
pub(super) struct Inspection {
    pub source: PathBuf,
    pub name: String,
    pub description: String,
    pub digest: String,
    pub warnings: Vec<String>,
    pub archive: Archive,
}
impl Inspection {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&self.archive)?)
    }
}

// Open every component without following links, including the explicitly selected
// source itself. Do not canonicalize through a symlink before checking it.
fn open(path: &Path) -> Result<Dir> {
    ensure!(path.is_absolute(), "skill path must be absolute");
    let mut components = path.components();
    let first = components.next().context("missing root")?;
    let mut anchor = PathBuf::new();
    match first {
        Component::RootDir => anchor.push(first.as_os_str()),
        Component::Prefix(_) => {
            anchor.push(first.as_os_str());
            ensure!(
                matches!(components.next(), Some(Component::RootDir)),
                "absolute root required"
            );
            anchor.push(std::path::MAIN_SEPARATOR.to_string());
        }
        _ => bail!("absolute root required"),
    }
    let mut dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?;
    for component in components {
        let Component::Normal(name) = component else {
            bail!("skill paths cannot contain parent traversal")
        };
        dir = dir.open_dir_nofollow(name)?;
    }
    Ok(dir)
}
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    ensure!(
        !path.components().any(|c| matches!(c, Component::ParentDir)),
        "skill paths cannot contain parent traversal"
    );
    Ok(path)
}

/// A deliberately strict YAML mapping subset: string scalars and literal/folded
/// multiline strings. Reject unsupported YAML rather than misinterpret authority.
pub(crate) fn metadata(text: &str) -> Result<(String, String, Vec<String>)> {
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("---"),
        "SKILL.md must start with YAML frontmatter (---)"
    );
    let mut fields = BTreeMap::<String, String>::new();
    let mut current: Option<(String, bool)> = None;
    let mut closed = false;
    for line in lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with(' ') {
            let (key, fold) = current
                .as_ref()
                .context("unsupported nested YAML; use string metadata")?;
            let value = fields.get_mut(key).unwrap();
            if !value.is_empty() {
                value.push(if *fold { ' ' } else { '\n' });
            }
            value.push_str(line.trim());
            continue;
        }
        current = None;
        let (key, raw) = line
            .split_once(':')
            .context("invalid frontmatter mapping")?;
        ensure!(
            !key.is_empty()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid metadata key"
        );
        ensure!(!fields.contains_key(key), "duplicate metadata key");
        let raw = raw.trim();
        if matches!(raw, "|" | "|-" | "|+" | ">" | ">-" | ">+") {
            fields.insert(key.into(), String::new());
            current = Some((key.into(), raw.starts_with('>')));
            continue;
        }
        let value = if raw.starts_with('"') {
            serde_json::from_str::<String>(raw)
                .context("double-quoted metadata must use JSON-compatible escapes")?
        } else if raw.starts_with('\'') {
            ensure!(
                raw.len() >= 2 && raw.ends_with('\''),
                "unterminated quoted metadata"
            );
            let inner = &raw[1..raw.len() - 1];
            ensure!(
                !inner.replace("''", "").contains('\''),
                "invalid single-quoted metadata"
            );
            inner.replace("''", "'")
        } else {
            ensure!(
                !raw.is_empty()
                    && !raw.contains(": ")
                    && !raw.contains(" #")
                    && !raw.starts_with(['[', '{', '&', '*', '!', '|', '>']),
                "unsupported YAML scalar; quote the value"
            );
            raw.to_owned()
        };
        fields.insert(key.into(), value);
    }
    ensure!(closed, "unterminated SKILL.md frontmatter");
    let name = fields.remove("name").context("missing skill name")?;
    ensure!(
        identifier(&name),
        "skill name must be a portable package identifier"
    );
    let description = fields
        .remove("description")
        .context("missing skill description")?;
    ensure!(
        !description.trim().is_empty() && description.len() <= 2048,
        "description must contain 1..2048 bytes"
    );
    ensure!(
        lines.any(|line| !line.trim().is_empty()),
        "skill instruction body is empty"
    );
    let warnings = fields
        .keys()
        .map(|key| format!("Metadata {key} is preserved as text, not interpreted or authorized"))
        .collect();
    Ok((name, description, warnings))
}

fn collect(
    dir: &Dir,
    prefix: &str,
    depth: usize,
    entries: &mut usize,
    bytes: &mut usize,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    ensure!(depth <= MAX_DEPTH, "skill directory depth exceeds eight");
    let mut names = Vec::new();
    for entry in dir.entries()? {
        *entries += 1;
        ensure!(*entries <= MAX_ENTRIES, "skill tree exceeds 128 entries");
        names.push(entry?.file_name());
    }
    names.sort();
    for name in names {
        let name = name.to_str().context("non-UTF-8 resource name")?;
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        ensure!(
            (prefix.is_empty() && name == "SKILL.md") || portable(&path),
            "unsupported resource path {path}; lowercase portable resource paths are required"
        );
        ensure!(
            path != PROVENANCE && path != "skill.md",
            "reserved import path {path}"
        );
        let metadata = dir.symlink_metadata(name)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "symlink skill content refused"
        );
        if metadata.is_dir() {
            collect(
                &dir.open_dir_nofollow(name)?,
                &path,
                depth + 1,
                entries,
                bytes,
                files,
            )?;
        } else {
            ensure!(metadata.is_file(), "skill content must be a regular file");
            ensure!(
                files.len() < MAX_FILES - 1,
                "skill has too many files (31 source files maximum)"
            );
            let raw = store::read(dir, name, MAX_ARCHIVE)?
                .context("skill file disappeared during read")?;
            *bytes += raw.len();
            ensure!(*bytes <= MAX_ARCHIVE, "skill source exceeds 64 KiB");
            files.insert(
                path,
                String::from_utf8(raw).context("binary skill resources are unsupported")?,
            );
        }
    }
    Ok(())
}
pub(super) fn inspect(path: &Path) -> Result<Inspection> {
    let source = absolute(path)?;
    let dir = open(&source).context("cannot open skill directory without symlinks")?;
    let mut files = BTreeMap::new();
    collect(&dir, "", 0, &mut 0, &mut 0, &mut files)?;
    let skill = files
        .remove("SKILL.md")
        .context("skill directory has no SKILL.md")?;
    let (name, description, mut warnings) = metadata(&skill)?;
    if files.contains_key("agents/openai.yaml") {
        warnings.push("agents/openai.yaml is inert resource text; UI, permissions and tool dependencies are not interpreted".into());
    }
    if files.keys().any(|p| p.starts_with("scripts/")) {
        warnings.push(
            "Scripts are inert UTF-8 resources, not installed executables; import never runs hooks"
                .into(),
        );
    }
    warnings.push("SKILL.md becomes skill.md; other relative paths are preserved. No source files are materialized or watched after import".into());
    let mut hashes: BTreeMap<String, String> = files
        .iter()
        .map(|(p, s)| (p.clone(), digest(s.as_bytes())))
        .collect();
    hashes.insert("SKILL.md".into(), digest(skill.as_bytes()));
    files.insert("skill.md".into(), skill);
    files.insert(
        PROVENANCE.into(),
        serde_json::to_string(&serde_json::json!({"adapter":1,"source":source,"files":hashes}))?,
    );
    let archive = Archive {
        manifest: Manifest {
            format: 1,
            id: name.clone(),
            version: "1.0.0".into(),
            helm: env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0.into(),
            capabilities: vec!["model_context".into()],
            contents: files
                .iter()
                .map(|(path, text)| Content {
                    path: path.clone(),
                    kind: if path == "skill.md" {
                        Kind::Skill
                    } else {
                        Kind::Resource
                    },
                    sha256: digest(text.as_bytes()),
                })
                .collect(),
            entrypoints: vec!["skill.md".into()],
        },
        files,
    };
    let bytes = serde_json::to_vec(&archive)?;
    Archive::parse(&bytes)?;
    Ok(Inspection {
        source,
        name,
        description,
        digest: digest(&bytes),
        warnings,
        archive,
    })
}

#[derive(Serialize)]
pub(super) struct Candidate {
    root: PathBuf,
    source: PathBuf,
    name: Option<String>,
    description: Option<String>,
    digest: Option<String>,
    diagnostic: Option<String>,
    duplicate_name: bool,
}
pub(super) fn discover(
    workspace: &Path,
    home: Option<&Path>,
    extra: &[PathBuf],
    legacy: bool,
) -> Result<Vec<Candidate>> {
    ensure!(extra.len() <= 16, "at most sixteen explicit skill roots");
    let mut roots = vec![absolute(workspace)?.join(".agents/skills")];
    if let Some(home) = home {
        roots.push(absolute(home)?.join(".agents/skills"));
    }
    roots.extend(
        extra
            .iter()
            .map(|p| absolute(p))
            .collect::<Result<Vec<_>>>()?,
    );
    if legacy {
        roots.push(absolute(workspace)?.join(".codex/skills"));
        if let Some(home) = home {
            roots.push(absolute(home)?.join(".codex/skills"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    let mut visited = 0;
    for root in roots {
        if !seen.insert(root.clone()) {
            continue;
        }
        let dir = match open(&root) {
            Ok(dir) => dir,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(_) => {
                result.push(Candidate {
                    source: root.clone(),
                    root,
                    name: None,
                    description: None,
                    digest: None,
                    diagnostic: Some("root unavailable or symlink refused".into()),
                    duplicate_name: false,
                });
                continue;
            }
        };
        let mut names = Vec::new();
        for entry in dir.entries()? {
            visited += 1;
            ensure!(
                visited <= 128,
                "discovery exceeds 128 entries; select a narrower root"
            );
            names.push(entry?.file_name());
        }
        names.sort();
        for name in names {
            let source = root.join(name);
            let mut row = Candidate {
                root: root.clone(),
                source: source.clone(),
                name: None,
                description: None,
                digest: None,
                diagnostic: None,
                duplicate_name: false,
            };
            match inspect(&source) {
                Ok(value) => {
                    row.name = Some(value.name);
                    row.description = Some(value.description);
                    row.digest = Some(value.digest);
                }
                Err(error) => row.diagnostic = Some(format!("{error:#}")),
            }
            result.push(row);
        }
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for row in &result {
        if let Some(name) = &row.name {
            *counts.entry(name.clone()).or_default() += 1;
        }
    }
    for row in &mut result {
        row.duplicate_name = row.name.as_ref().is_some_and(|name| counts[name] > 1);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::catalog::{Catalog, Scope};
    fn skill(path: &Path, name: &str) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("SKILL.md"), format!("---\nname: {name}\ndescription: >\n  Useful reusable\n  instructions\n---\nRead references/guide.md.\n")).unwrap();
    }
    #[test]
    fn snapshot_import_review_replacement_and_resources() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        skill(&source, "example");
        std::fs::create_dir(source.join("references"))?;
        std::fs::write(source.join("references/guide.md"), "Preserved resource")?;
        let first = inspect(&source)?;
        assert_eq!(first.description, "Useful reusable instructions");
        assert_eq!(
            first.archive.files["references/guide.md"],
            "Preserved resource"
        );
        assert_eq!(first.digest, inspect(&source)?.digest);
        let workspace = temp.path().join("work");
        std::fs::create_dir(&workspace)?;
        let catalog = Catalog::new(&workspace, &temp.path().join("user"))?;
        catalog.mutate(
            Scope::Project,
            &first.name,
            None,
            Some(&first.bytes()?),
            None,
        )?;
        assert!(!catalog.inspect(Scope::Project, "example")?.active);
        catalog.mutate(
            Scope::Project,
            "example",
            Some(&first.digest),
            None,
            Some(true),
        )?;
        assert!(catalog.inspect(Scope::Project, "example")?.active);
        std::fs::write(source.join("references/guide.md"), "Changed resource")?;
        let next = inspect(&source)?;
        assert_ne!(first.digest, next.digest);
        assert!(
            catalog
                .mutate(
                    Scope::Project,
                    "example",
                    Some("wrong"),
                    Some(&next.bytes()?),
                    None
                )
                .is_err()
        );
        catalog.mutate(
            Scope::Project,
            "example",
            Some(&first.digest),
            Some(&next.bytes()?),
            None,
        )?;
        assert!(!catalog.inspect(Scope::Project, "example")?.active);
        Ok(())
    }
    #[test]
    fn discovery_roots_duplicates_and_legacy_are_explicit() -> Result<()> {
        let t = tempfile::tempdir()?;
        let work = t.path().join("work");
        let home = t.path().join("home");
        skill(&work.join(".agents/skills/a"), "same");
        skill(&home.join(".agents/skills/b"), "same");
        skill(&work.join(".codex/skills/c"), "legacy");
        let rows = discover(&work, Some(&home), &[], false)?;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.duplicate_name));
        assert_eq!(discover(&work, Some(&home), &[], true)?.len(), 3);
        assert_eq!(
            discover(&work, None, &[home.join(".agents/skills")], false)?.len(),
            2
        );
        Ok(())
    }
    #[test]
    fn malformed_metadata_and_unsupported_resources_refuse() -> Result<()> {
        for text in [
            "no metadata",
            "---\nname: a\nname: b\ndescription: x\n---\nx",
            "---\nname: good\ndescription: [bad]\n---\nx",
            "---\nname: good\ndescription: x\n---\n",
            "---\nname: ../bad\ndescription: x\n---\nx",
        ] {
            assert!(metadata(text).is_err());
        }
        assert_eq!(
            metadata("---\nname: good\ndescription: 'It''s useful'\n---\nx")?.1,
            "It's useful"
        );
        let t = tempfile::tempdir()?;
        skill(t.path(), "good");
        std::fs::write(t.path().join("image.png"), [255, 0])?;
        assert!(inspect(t.path()).is_err());
        std::fs::remove_file(t.path().join("image.png"))?;
        std::fs::write(t.path().join("README.md"), "unsupported uppercase path")?;
        assert!(inspect(t.path()).is_err());
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn symlink_sources_resources_and_roots_refuse() -> Result<()> {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir()?;
        let source = t.path().join("source");
        skill(&source, "good");
        symlink(&source, t.path().join("alias"))?;
        assert!(inspect(&t.path().join("alias")).is_err());
        symlink("SKILL.md", source.join("resource.md"))?;
        assert!(inspect(&source).is_err());
        assert!(open(&source.join("../source")).is_err());
        Ok(())
    }
    #[test]
    fn source_move_changes_review_digest_and_limits_are_bounded() -> Result<()> {
        let t = tempfile::tempdir()?;
        let source = t.path().join("source");
        skill(&source, "good");
        let before = inspect(&source)?.digest;
        let moved = t.path().join("moved");
        std::fs::rename(&source, &moved)?;
        assert_ne!(before, inspect(&moved)?.digest);
        for i in 0..32 {
            std::fs::write(moved.join(format!("file{i}.md")), "x")?;
        }
        assert!(inspect(&moved).is_err());
        Ok(())
    }
}
