use super::*;
use std::io::{ErrorKind, Read};

#[test]
fn private_roundtrip_and_replacement_have_verified_permissions() {
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    directory.publish("client.json", b"first").unwrap();
    directory.publish("client.json", b"second").unwrap();
    let mut file = directory.open_file("client.json", false).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"second");
    drop(directory);
    PrivateDirectory::open(&root.path().join("private")).unwrap();
}

#[test]
fn independent_directories_are_allowed_but_client_lock_is_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    let a = PrivateDirectory::open(&path).unwrap();
    let b = PrivateDirectory::open(&path).unwrap();
    let lock = a.lock("client.lock").unwrap();
    assert_eq!(
        b.lock("client.lock").unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
    drop(lock);
    b.lock("client.lock").unwrap();
}

#[test]
fn rejects_linked_files_alternate_streams_and_replaced_directory() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    let directory = PrivateDirectory::open(&path).unwrap();
    directory.publish("client.json", b"protected").unwrap();
    std::fs::hard_link(path.join("client.json"), path.join("alias")).unwrap();
    assert!(directory.open_file("client.json", false).is_err());
    assert!(directory.open_file("client.json:stream", true).is_err());
    assert!(directory.open_file("../escape", true).is_err());
    assert!(std::fs::rename(&path, root.path().join("moved")).is_err());
}

#[test]
fn failed_publication_keeps_original_state() {
    let root = tempfile::tempdir().unwrap();
    let directory = PrivateDirectory::open(&root.path().join("private")).unwrap();
    directory.publish("client.json", b"original").unwrap();
    let held = directory.open_file("client.json", false).unwrap();
    assert!(
        directory
            .publish("client.json", b"must not replace")
            .is_err()
    );
    drop(held);
    assert_eq!(
        std::fs::read(directory.path().join("client.json")).unwrap(),
        b"original"
    );
}
