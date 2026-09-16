use super::*;
fn parse(args: &[&str]) -> Result<Options> {
    Options::parse(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>())
}
#[test]
fn actions_defaults_and_source_labels() {
    let default = parse(&[]).unwrap();
    assert!(default.action.is_none());
    assert!(!default.start);
    assert!(!default.dry_run);
    assert!(!default.local_source);
    assert!(default.source_label().contains("Latest"));
    for (action, expected) in [
        ("install", Action::Install),
        ("upgrade", Action::Upgrade),
        ("rollback", Action::Rollback),
    ] {
        let mut options = parse(&[action, "--no-start", "--dry-run"]).unwrap();
        assert_eq!(options.action, Some(expected));
        assert!(!options.start);
        assert!(options.dry_run);
        assert_eq!(expected.label().to_lowercase(), action);
        if expected != Action::Upgrade {
            options.prepare(&AtomicBool::new(true)).unwrap();
        }
    }
    assert!(
        parse(&["install"])
            .unwrap()
            .source_label()
            .starts_with("Local binaries:")
    );
    assert_eq!(
        parse(&["rollback"]).unwrap().source_label(),
        "Recorded previous release"
    );
    assert!(
        parse(&["upgrade", "--dev"])
            .unwrap()
            .source_label()
            .contains("GitHub main")
    );
    let local = parse(&["upgrade", "--bin-dir", ".", "--replace-existing", "--start"]).unwrap();
    assert_eq!(local.bin_dir, PathBuf::from("."));
    assert!(local.local_source && local.replace_existing && local.start);
    assert!(local.source_label().starts_with("Local binaries:"));
}
#[test]
fn invalid_options_and_conflicts_are_rejected() {
    for args in [
        vec!["bogus"],
        vec!["install", "upgrade"],
        vec!["--bin-dir"],
        vec!["--dev", "--bin-dir", "."],
        vec!["install", "--dev"],
        vec!["rollback", "--dev"],
        vec!["rollback", "--bin-dir", "."],
        vec!["rollback", "--replace-existing"],
        vec!["--unknown"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    let options = parse(&["--dev"]).unwrap();
    assert!(options.dev);
    assert!(options.action.is_none());
}
#[test]
fn explicit_local_prepare_never_acquires_source() {
    let mut options = parse(&["upgrade", "--bin-dir", "."]).unwrap();
    let before = options.bin_dir.clone();
    options.prepare(&AtomicBool::new(true)).unwrap();
    assert_eq!(options.bin_dir, before);
    assert!(options.prepared.is_none());
    help();
}
