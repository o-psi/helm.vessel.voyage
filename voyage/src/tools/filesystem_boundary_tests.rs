//! Filesystem effects stay inside owned temporary roots. Source prepared only.
use super::*;
use crate::tools::{Tool, ToolError};
use serde_json::json;
use sha2::{Digest, Sha256};
#[tokio::test]
async fn patch_missing_hash_wrong_context_and_malformed_body_leave_original_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let path = root.path().join("file");
    std::fs::write(&path, b"original\n").unwrap();
    let patch = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-mismatch\n+replacement\n";
    assert!(matches!(
        ApplyPatch
            .execute(json!({"path":path,"patch":patch}), &context)
            .await,
        Err(ToolError::InvalidArguments(_))
    ));
    let hash = hex::encode(Sha256::digest(b"original\n"));
    assert!(matches!(
        ApplyPatch
            .execute(
                json!({"path":path,"patch":patch,"base_sha256":hash}),
                &context
            )
            .await,
        Err(ToolError::Failed(_))
    ));
    for malformed in [
        "--- a/file\n+++ b/file\n@@ not a hunk @@\n",
        "--- a/file\n+++ b/file\n@@ -1 +1 @@\n!illegal marker\n",
    ] {
        assert!(
            ApplyPatch
                .execute(
                    json!({"path":path,"patch":malformed,"base_sha256":hash}),
                    &context
                )
                .await
                .is_err()
        );
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"original\n");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
#[tokio::test]
async fn cancelled_patch_has_no_publication_or_temporary_file_effect() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    context.cancellation.cancel();
    let result = ApplyPatch
        .execute(
            json!({"path":"new","patch":"--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+content\n"}),
            &context,
        )
        .await;
    assert!(matches!(result, Err(ToolError::Cancelled)));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn default_directory_listing_never_follows_linked_outside_tree() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("local"), b"fixture").unwrap();
    std::fs::write(external.path().join("outside"), b"fixture").unwrap();
    std::os::unix::fs::symlink(external.path(), root.path().join("link")).unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let listing = ListDirectory
        .execute(json!({"recursive":true}), &context)
        .await
        .unwrap();
    assert!(listing.contains("local"));
    assert!(listing.contains("link"));
    assert!(!listing.contains("outside"));
    context.cancellation.cancel();
    assert!(matches!(
        ListDirectory.execute(json!({}), &context).await,
        Err(ToolError::Cancelled)
    ));
}
#[tokio::test]
async fn search_treats_flag_shaped_pattern_as_data_and_classifies_empty_and_invalid_matches() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("fixture.txt"), b"--help\nmatching value\n").unwrap();
    std::fs::write(root.path().join("other.rs"), b"matching value\n").unwrap();
    let mut context = crate::tools::reliability_tests::context(root.path());
    context
        .environment
        .insert("PATH".into(), "/usr/bin:/bin".into());
    let found = SearchFiles
        .execute(json!({"query":"--help","glob":"*.txt"}), &context)
        .await
        .unwrap();
    assert!(found.contains("fixture.txt:1:--help"));
    assert!(!found.contains("Usage:"));
    let found = SearchFiles
        .execute(json!({"query":"matching","glob":"*.txt"}), &context)
        .await
        .unwrap();
    assert!(found.contains("fixture.txt"));
    assert!(!found.contains("other.rs"));
    assert!(
        SearchFiles
            .execute(json!({"query":"absent"}), &context)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        SearchFiles.execute(json!({"query":"["}), &context).await,
        Err(ToolError::Failed(_))
    ));
    context.cancellation.cancel();
    assert!(matches!(
        SearchFiles
            .execute(json!({"query":"matching"}), &context)
            .await,
        Err(ToolError::Cancelled)
    ));
}
