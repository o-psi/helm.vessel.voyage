use super::*;
use clap::Parser;

#[derive(Parser)]
struct FixtureCli {
    #[command(flatten)]
    args: Args,
}
fn parse(args: &[&str]) -> Result<FixtureCli, clap::Error> {
    FixtureCli::try_parse_from(
        std::iter::once("fixture")
            .chain(["--directory", "/synthetic"])
            .chain(args.iter().copied()),
    )
}

#[test]
fn managed_cli_preserves_explicit_identity_and_revision_requirements() {
    let session = Uuid::new_v4().to_string();
    let command = Uuid::new_v4().to_string();
    for args in [
        vec!["create"],
        vec!["create", "--id", &session, "--name", "fixture"],
        vec!["submit", &session, "--expected-revision", "7", "hello"],
        vec![
            "submit",
            &session,
            "--expected-revision",
            "7",
            "--command-id",
            &command,
            "--expires-at-ms",
            "1000",
            "hello",
        ],
    ] {
        assert!(!parse(&args).unwrap().args.administrative());
    }
    for args in [
        vec!["list"],
        vec!["list", "--after", &session, "--limit", "2"],
        vec!["cancel", &session, "--run", &command],
        vec!["recover", &session],
        vec![
            "recover",
            &session,
            "--acknowledge-cleanup",
            &command,
            "--acknowledge-resource",
            &command,
        ],
        vec![
            "recover",
            &session,
            "--reconcile-tools",
            &command,
            "--expected-revision",
            "2",
        ],
        vec!["upgrade"],
    ] {
        assert!(parse(&args).unwrap().args.administrative());
    }
    for args in [
        vec!["submit", &session, "hello"],
        vec![
            "submit",
            &session,
            "--expected-revision",
            "7",
            "--command-id",
            &command,
            "hello",
        ],
        vec![
            "submit",
            &session,
            "--expected-revision",
            "7",
            "--expires-at-ms",
            "1000",
            "hello",
        ],
        vec!["cancel", &session],
        vec!["recover", &session, "--reconcile-tools", &command],
        vec!["recover", &session, "--expected-revision", "2"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}
