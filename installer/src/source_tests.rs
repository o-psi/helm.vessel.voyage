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
    for (source, name) in [(Source::Latest, "latest"), (Source::Nightly, "nightly")] {
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
    let error = prepare(Source::Nightly, &flag).err().unwrap();
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
fn development_upgrade_selects_published_nightly_acquisition() {
    let _f = Fixture::new();
    set_acquire(SUCCESS);
    let mut options = crate::cli::Options::parse(&["upgrade".into(), "--dev".into()]).unwrap();
    options.prepare(&AtomicBool::new(false)).unwrap();
    assert_eq!(options.source_label(), "pinned nightly");
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

#[test]
fn system_acquisition_uses_explicit_home_public_mode_and_fixed_environment() {
    let f = Fixture::new();
    set_acquire(&format!(
        "{SUCCESS}\nimport os\nassert sys.argv[3]=='public'\nassert os.environ['HOME']==str(root.parents[3])\nassert os.environ['PATH']=='/usr/bin:/bin'\nassert 'GH_TOKEN' not in os.environ\nassert 'GITHUB_TOKEN' not in os.environ\n"
    ));
    let prepared = prepare_public(Source::Latest, &AtomicBool::new(false), f.root.clone()).unwrap();
    assert!(prepared.staging_root().starts_with(&f.root));
    drop(prepared);
    assert!(prepare_public(Source::Latest, &AtomicBool::new(true), f.root.clone()).is_err());
}

#[test]
fn acquired_metadata_bounds_and_lexical_escape_refuse_and_clean_only_owned_stage() {
    let f = Fixture::new();
    let sentinel = f.root.join("unrelated-sentinel");
    std::fs::write(&sentinel, b"preserve").unwrap();
    for suffix in [
        "(root/'prepared.json').write_bytes(b'x'*65537)",
        "(root/'prepared.json').write_text(json.dumps({'bin_dir':str(root/'../outside'),'description':'escape'}))",
        "(root/'release.json').write_bytes(b'x'*1048577)",
        "(binary/'helm').unlink(); (binary/'helm').symlink_to('/bin/sh')",
        "(root/'prepared.json').unlink(); (root/'prepared.json').symlink_to('/etc/passwd')",
    ] {
        set_acquire(&format!("{SUCCESS}\n{suffix}"));
        assert!(prepare(Source::Latest, &AtomicBool::new(false)).is_err());
        assert_eq!(
            std::fs::read_dir(f.root.join(".cache/voyage/upgrades"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve");
    }
}

#[test]
fn acquired_release_identity_contract_rejection_leaves_no_usable_preparation() {
    let f = Fixture::new();
    for changes in [
        "manifest['schema_version']=2",
        "manifest['target']='wrong-platform'",
        "manifest['binaries'].pop('voyage')",
        "manifest['binaries']['helm']['sha256']='g'*64",
        "manifest['version']='invalid\\nversion'",
        "manifest['assets']={'share/voyage/browser/worker.mjs':{'sha256':'a'*64}}",
    ] {
        set_acquire(&format!(
            "{SUCCESS}\nmanifest=json.loads((root/'release.json').read_text())\n{changes}\n(root/'release.json').write_text(json.dumps(manifest))"
        ));
        assert!(prepare(Source::Nightly, &AtomicBool::new(false)).is_err());
        assert_eq!(
            std::fs::read_dir(f.root.join(".cache/voyage/upgrades"))
                .unwrap()
                .count(),
            0
        );
    }
    cleanup_result().unwrap();
}
