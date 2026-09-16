use super::*;
use crate::fixture_tests::{Fixture, set_acquire};
use std::sync::atomic::Ordering;
const SUCCESS: &str = r#"
import sys,json,pathlib,hashlib,platform
root=pathlib.Path(sys.argv[2]); binary=root/'bin'; binary.mkdir()
hashes={}
for name in ['helm','vessel','voyage','voyage-installer']:
 p=binary/name; p.write_bytes(b'fixture executable'); p.chmod(0o700)
 hashes[name]={'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
(root/'release.json').write_text(json.dumps({'schema_version':1,'version':'fixture','target':'linux-'+platform.machine(),'binaries':hashes}))
(root/'prepared.json').write_text(json.dumps({'bin_dir':str(binary),'description':'pinned '+sys.argv[1]}))
"#;
#[test]
fn preparation_reads_pinned_metadata_and_drop_cleans_only_staging() {
    let f = Fixture::new();
    set_acquire(SUCCESS);
    for (source, name) in [(Source::Latest, "latest"), (Source::Main, "main")] {
        let prepared = prepare(source, &AtomicBool::new(false)).unwrap();
        assert_eq!(prepared.description, format!("pinned {name}"));
        assert!(prepared.bin_dir.join("helm").is_file());
        let root = prepared.root.clone();
        assert!(root.starts_with(&f.root));
        drop(prepared);
        assert!(!root.exists());
    }
    cleanup_result().unwrap();
    assert!(f.root.exists());
}
#[test]
fn preparation_failure_and_invalid_metadata_clean_staging() {
    let f = Fixture::new();
    for script in [
        "raise RuntimeError('fixture failure')".to_owned(),
        "pass".to_owned(),
        "import sys,pathlib; (pathlib.Path(sys.argv[2])/'prepared.json').write_text('bad json')"
            .to_owned(),
        format!(
            "{SUCCESS}\n(root/'prepared.json').write_text(json.dumps({{'bin_dir':'/outside','description':'escape'}}))"
        ),
        format!("{SUCCESS}\n(binary/'helm').write_bytes(b'changed')"),
    ] {
        set_acquire(&script);
        assert!(prepare(Source::Latest, &AtomicBool::new(false)).is_err());
        assert_eq!(
            std::fs::read_dir(f.root.join(".cache/voyage/upgrades"))
                .unwrap()
                .count(),
            0
        );
    }
}
#[test]
fn precancelled_source_never_creates_cache() {
    let f = Fixture::new();
    assert!(prepare(Source::Latest, &AtomicBool::new(true)).is_err());
    assert!(!f.root.join(".cache").exists());
}
#[test]
fn cancellation_terminates_owned_acquisition_group() {
    let f = Fixture::new();
    set_acquire("import time; time.sleep(60)");
    let flag = std::sync::Arc::new(AtomicBool::new(false));
    let worker_flag = flag.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        worker_flag.store(true, Ordering::Relaxed);
    });
    let error = prepare(Source::Main, &flag).err().unwrap();
    worker.join().unwrap();
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(
        std::fs::read_dir(f.root.join(".cache/voyage/upgrades"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn retained_and_already_missing_staging_are_not_removed_twice() {
    let f = Fixture::new();
    let root = f.root.join("retained");
    std::fs::create_dir(&root).unwrap();
    drop(Prepared {
        bin_dir: root.clone(),
        description: String::new(),
        root: root.clone(),
        retain: true,
    });
    assert!(root.exists());
    drop(Prepared {
        bin_dir: root.clone(),
        description: String::new(),
        root: f.root.join("missing"),
        retain: false,
    });
    cleanup_result().unwrap();
}

#[test]
fn prepared_upgrade_is_shared_and_reused_without_reacquisition() {
    let f = Fixture::new();
    set_acquire(SUCCESS);
    let mut options = crate::cli::Options::parse(&["upgrade".into()]).unwrap();
    options.prepare(&AtomicBool::new(false)).unwrap();
    let root = options.prepared.as_ref().unwrap().root.clone();
    assert_eq!(options.source_label(), "pinned latest");
    let copy = options.clone();
    set_acquire("raise RuntimeError('must not reacquire')");
    options.prepare(&AtomicBool::new(true)).unwrap();
    assert_eq!(options.bin_dir, copy.bin_dir);
    drop(options);
    assert!(root.exists());
    drop(copy);
    assert!(!root.exists());
    assert!(f.root.exists());
}
#[test]
fn acquisition_error_details_are_sanitized_and_bounded() {
    let _f = Fixture::new();
    set_acquire(
        "import pathlib,sys; (pathlib.Path(sys.argv[2])/'error.txt').write_text('detail\\x1b\\n'+'x'*3000); sys.exit(1)",
    );
    let error = prepare(Source::Latest, &AtomicBool::new(false))
        .err()
        .unwrap()
        .to_string();
    assert!(error.starts_with("detail"));
    assert_eq!(error.chars().count(), 2048);
    assert!(!error.chars().any(char::is_control));
}
