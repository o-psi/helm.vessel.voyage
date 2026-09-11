//! Structural/package regressions, not proof of executable isolation.
use super::*;
use base64::Engine;
use serde_json::json;

pub(super) fn package() -> executable::Archive {
    let bytes = executable::tests::elf();
    executable::Archive {
        manifest: executable::Manifest {
            format: 2,
            id: "example".into(),
            version: "1.0.0".into(),
            voyage: "0.1".into(),
            protocol: 1,
            platform: "linux-x86_64".into(),
            runtime: "static-elf".into(),
            entrypoint: "tool".into(),
            contents: vec![executable::Content {
                path: "tool".into(),
                sha256: digest(&bytes),
            }],
            capabilities: vec!["execute".into()],
            definitions: json!({"tools":[{"name":"echo","description":"Echo","input_schema":true,"output_schema":true}],"commands":[],"lifecycle":[]}),
        },
        files: BTreeMap::from([(
            "tool".into(),
            base64::engine::general_purpose::STANDARD.encode(bytes),
        )]),
    }
}
#[test]
fn executable_format_is_separate_and_exact() -> Result<()> {
    let mut archive = package();
    let raw = serde_json::to_vec(&archive)?;
    assert!(matches!(Package::parse(&raw)?, Package::Executable(_)));
    assert!(Archive::parse(&raw).is_err());
    archive.manifest.capabilities.push("shell".into());
    assert!(archive.validate().is_err());
    archive = package();
    archive.files.get_mut("tool").unwrap().push('\n');
    assert!(archive.validate().is_err());
    archive = package();
    archive.manifest.contents[0].sha256 = "0".repeat(64);
    assert!(archive.validate().is_err());
    let raw = String::from_utf8(raw)?;
    let duplicate = raw.replace(
        "\"input_schema\":true",
        "\"input_schema\":true,\"input_schema\":false",
    );
    assert_ne!(duplicate, raw);
    assert!(Package::parse(duplicate.as_bytes()).is_err());
    Ok(())
}
#[test]
fn declarative_format_does_not_gain_executable_grants_or_size() -> Result<()> {
    let text = "Guidance";
    let mut archive = Archive {
        manifest: Manifest {
            format: 1,
            id: "guide".into(),
            version: "1.0.0".into(),
            helm: "0.1".into(),
            capabilities: vec!["model_context".into()],
            contents: vec![Content {
                path: "guide.md".into(),
                kind: Kind::Skill,
                sha256: digest(text.as_bytes()),
            }],
            entrypoints: vec!["guide.md".into()],
        },
        files: BTreeMap::from([("guide.md".into(), text.into())]),
    };
    assert!(matches!(
        Package::parse(&serde_json::to_vec(&archive)?)?,
        Package::Declarative(_)
    ));
    archive.manifest.capabilities = vec!["execute".into()];
    assert!(archive.validate().is_err());
    let mut oversized = serde_json::to_vec(&archive)?;
    oversized.resize(MAX_ARCHIVE + 1, b' ');
    assert!(Archive::parse(&oversized).is_err());
    Ok(())
}
#[cfg(target_os = "linux")]
#[test]
fn real_catalog_review_update_and_revocation_are_digest_bound() -> Result<()> {
    use catalog::{Catalog, Scope};
    let temp = tempfile::tempdir()?;
    let workspace = temp.path().join("work");
    let user = temp.path().join("user");
    std::fs::create_dir(&workspace)?;
    let catalog = Catalog::new(&workspace, &user)?;
    let archive = package();
    let bytes = serde_json::to_vec(&archive)?;
    let sha = digest(&bytes);
    catalog.mutate(Scope::User, "example", None, Some(&bytes), None)?;
    assert!(!catalog.inspect(Scope::User, "example")?.execution_reviewed);
    assert!(
        catalog
            .mutate(Scope::User, "example", Some(&sha), None, Some(true))
            .is_err()
    );
    assert!(
        catalog
            .review_executable(Scope::User, "example", &sha, &["model_context".into()])
            .is_err()
    );
    catalog.review_executable(Scope::User, "example", &sha, &["execute".into()])?;
    let inspection = catalog.inspect(Scope::User, "example")?;
    assert!(inspection.execution_reviewed && !inspection.active);
    assert_eq!(catalog.executable_snapshots()?.len(), 1);
    assert!(
        catalog
            .mutate(
                Scope::User,
                "example",
                Some(&"0".repeat(64)),
                None,
                Some(false)
            )
            .is_err()
    );
    assert!(catalog.inspect(Scope::User, "example")?.execution_reviewed);
    let mut replacement = package();
    replacement.manifest.version = "1.0.1".into();
    let changed = serde_json::to_vec(&replacement)?;
    catalog.mutate(Scope::User, "example", Some(&sha), Some(&changed), None)?;
    let next = catalog.inspect(Scope::User, "example")?;
    assert!(!next.execution_reviewed && next.digest == digest(&changed));
    assert!(
        catalog
            .review_executable(Scope::User, "example", &sha, &["execute".into()])
            .is_err()
    );
    catalog.mutate(Scope::User, "example", Some(&next.digest), None, None)?;
    assert!(catalog.inspect(Scope::User, "example").is_err());
    Ok(())
}
#[cfg(target_os = "linux")]
#[test]
fn unobserved_work_revokes_review_but_blocks_replacement() -> Result<()> {
    use catalog::{Catalog, Scope};
    let temp = tempfile::tempdir()?;
    let workspace = temp.path().join("work");
    let user = temp.path().join("user");
    std::fs::create_dir(&workspace)?;
    let catalog = Catalog::new(&workspace, &user)?;
    let raw = serde_json::to_vec(&package())?;
    let sha = digest(&raw);
    catalog.mutate(Scope::User, "example", None, Some(&raw), None)?;
    catalog.review_executable(Scope::User, "example", &sha, &["execute".into()])?;
    let binding = catalog.inspect(Scope::User, "example")?.binding;
    // A durable interrupted invocation without a live file lock must STILL block.
    let db = rusqlite::Connection::open(user.join("host-resources/reservations.sqlite3"))?;
    db.execute(
        "INSERT INTO reservations(id,kind,owner,units) VALUES('interrupted','extensions','run',1)",
        [],
    )?;
    db.execute(
        "INSERT INTO extension_reservations VALUES('interrupted',?1,?2,'invocation','invoke')",
        rusqlite::params![binding, sha],
    )?;
    assert!(
        catalog
            .mutate(Scope::User, "example", Some(&sha), None, None)
            .is_err()
    );
    let inspection = catalog.inspect(Scope::User, "example")?;
    assert_eq!(inspection.digest, sha);
    assert!(!inspection.execution_reviewed);
    assert_eq!(inspection.pending_execution, Some(1));
    db.execute(
        "UPDATE reservations SET observed=2 WHERE id='interrupted'",
        [],
    )?;
    assert!(
        catalog
            .mutate(Scope::User, "example", Some(&sha), None, None)
            .is_err()
    );
    db.execute(
        "UPDATE reservations SET observed=1 WHERE id='interrupted'",
        [],
    )?;
    catalog.mutate(Scope::User, "example", Some(&sha), None, None)?;
    Ok(())
}
#[cfg(unix)]
#[test]
fn pack_refuses_symlinks_and_undeclared_inputs() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let archive = package();
    std::fs::write(
        temp.path().join("manifest.json"),
        serde_json::to_vec(&archive.manifest)?,
    )?;
    let bytes = executable::tests::elf();
    std::fs::write(temp.path().join("tool"), &bytes)?;
    assert!(Package::parse(&store::pack(temp.path())?).is_ok());
    std::fs::write(temp.path().join("extra"), "not declared")?;
    assert!(store::pack(temp.path()).is_err());
    std::fs::remove_file(temp.path().join("extra"))?;
    std::fs::remove_file(temp.path().join("tool"))?;
    let outside = tempfile::NamedTempFile::new()?;
    std::fs::write(outside.path(), bytes)?;
    std::os::unix::fs::symlink(outside.path(), temp.path().join("tool"))?;
    assert!(store::pack(temp.path()).is_err());
    Ok(())
}
