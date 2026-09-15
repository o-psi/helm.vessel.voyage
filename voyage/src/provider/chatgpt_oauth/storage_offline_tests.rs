use super::*;
use base64::Engine;

fn tokens(subject: &str) -> crate::provider::chatgpt_oauth::OAuthTokens {
    let claims = serde_json::json!({"sub":subject,"account_id":"offline"});
    crate::provider::chatgpt_oauth::OAuthTokens {
        access_token: format!(
            "offline.{}.unsigned",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&claims).unwrap())
        ),
        refresh_token: "offline-refresh".into(),
        id_token: None,
        expires_at: 4102444800,
        account_id: "offline".into(),
    }
}

#[test]
fn rotation_is_fenced_compare_and_swap_and_owner_save_recovers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache/record.json");
    let old = tokens("subject");
    let mut new = old.clone();
    new.refresh_token = "offline-rotated".into();
    write(&path, Some(&serde_json::to_vec(&old).unwrap())).unwrap();
    assert!(begin_refresh(&path, &new).is_err());
    let fence = begin_refresh(&path, &old).unwrap();
    assert!(
        read_tokens(&path)
            .unwrap_err()
            .downcast_ref::<crate::provider::chatgpt_oauth::RefreshPending>()
            .is_some()
    );
    assert!(begin_refresh(&path, &old).is_err());
    assert!(commit_refresh(&path, &old, &new, uuid::Uuid::new_v4()).is_err());
    assert!(commit_refresh(&path, &old, &tokens("another-subject"), fence).is_err());
    commit_refresh(&path, &old, &new, fence).unwrap();
    let loaded: crate::provider::chatgpt_oauth::OAuthTokens =
        serde_json::from_slice(&read_tokens(&path).unwrap().unwrap()).unwrap();
    assert!(loaded == new);
    assert!(commit_refresh(&path, &old, &new, fence).is_err());
    let stale = begin_refresh(&path, &new).unwrap();
    write(&path, None).unwrap();
    assert_eq!(read_tokens(&path).unwrap().unwrap(), b"null");
    assert!(commit_refresh(&path, &new, &old, stale).is_err());
    assert!(begin_refresh(&path, &old).is_err());
    write(&path, Some(&serde_json::to_vec(&old).unwrap())).unwrap();
    assert!(begin_refresh(&path, &old).is_ok());
}

#[test]
fn opaque_identity_cannot_be_rotated_even_with_matching_fence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private/tokens");
    let mut opaque = tokens("subject");
    opaque.access_token = "opaque-offline".into();
    write(&path, Some(&serde_json::to_vec(&opaque).unwrap())).unwrap();
    let id = begin_refresh(&path, &opaque).unwrap();
    assert!(commit_refresh(&path, &opaque, &opaque, id).is_err());
    assert!(read_tokens(&path).is_err());
}

#[test]
fn missing_paths_reserved_names_and_size_bounds() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing/tokens");
    assert!(read(&missing).unwrap().is_none());
    assert!(read_tokens(&missing).unwrap().is_none());
    write(&missing, None).unwrap();
    assert!(!missing.parent().unwrap().exists());
    assert!(parent_and_name(Path::new("relative")).is_err());
    assert!(parent_and_name(Path::new("/")).is_err());
    for name in [
        "actor.lock".to_string(),
        "publication.json".into(),
        "tokens.refresh".into(),
        "x".repeat(121),
    ] {
        assert!(write(&root.path().join(name), Some(b"{} ")).is_err());
    }
    let dir = root.path().join("deep/private/cache");
    prepare(&dir).unwrap();
    prepare(&dir).unwrap();
    let path = dir.join("tokens");
    write(&path, Some(&vec![b'x'; LIMIT])).unwrap();
    assert_eq!(read(&path).unwrap().unwrap().len(), LIMIT);
    // Oversize publication is rejected without damaging the previous record.
    assert!(write(&path, Some(&vec![b'x'; LIMIT + 1])).is_err());
    assert_eq!(read(&path).unwrap().unwrap().len(), LIMIT);
    std::fs::write(&path, vec![b'x'; LIMIT + 1]).unwrap();
    assert!(read(&path).is_err());
    assert!(read_tokens(&path).is_err());
}

#[cfg(unix)]
#[test]
fn cache_rejects_symlinks_and_nonprivate_parents() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let private = root.path().join("private");
    prepare(&private).unwrap();
    assert_eq!(
        std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let path = private.join("tokens");
    write(&path, Some(b"null")).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let alias = root.path().join("alias");
    symlink(&private, &alias).unwrap();
    assert!(read(&alias.join("tokens")).is_err());
    let linked = private.join("linked");
    symlink(&path, &linked).unwrap();
    assert!(read(&linked).is_err());
    assert!(write(&linked, Some(b"changed")).is_err());
    assert_eq!(read(&path).unwrap().unwrap(), b"null");
    let public = root.path().join("public");
    std::fs::create_dir(&public).unwrap();
    std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(write(&public.join("tokens"), Some(b"null")).is_err());
    let file = root.path().join("file");
    std::fs::write(&file, b"untouched").unwrap();
    assert!(prepare(&file.join("nested")).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"untouched");
}
