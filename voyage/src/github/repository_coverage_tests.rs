use super::*;
#[test]
fn supported_remotes_canonicalize_without_host_aliases_or_credentials() {
    let expected = Repository::new("Example", "Project").unwrap();
    assert_eq!(expected.slug(), "example/project");
    expected.validate().unwrap();
    for remote in [
        "git@github.com:Example/Project.git",
        "ssh://git@github.com/Example/Project.git",
        "https://github.com/Example/Project.git",
        "https://github.com/Example/Project",
    ] {
        assert_eq!(Repository::from_remote(remote).unwrap(), expected);
    }
    for remote in [
        "git@alias:Example/Project.git",
        "http://github.com/example/project",
        "https://user@github.com/example/project",
        "https://user:pass@github.com/example/project",
        "ssh://other@github.com/example/project",
        "https://github.com:8443/example/project",
        "https://evil.invalid/example/project",
        "https://github.com/example/project?x=1",
        "https://github.com/example/project#x",
        "https://github.com/example/%70roject",
        "https://github.com/example/../project",
        "https://github.com/example/project/more",
        "https://github.com/example/project\n",
    ] {
        assert!(Repository::from_remote(remote).is_err(), "{remote}");
    }
    for (owner, name) in [
        ("", "project"),
        ("-owner", "project"),
        ("owner-", "project"),
        ("owner", ""),
        ("owner", ".."),
        ("owner", "-project"),
        ("owner", "bad/name"),
        ("non ascii é", "project"),
    ] {
        assert!(Repository::new(owner, name).is_err());
    }
    assert!(
        Repository {
            owner: "Example".into(),
            name: "project".into()
        }
        .validate()
        .is_err()
    );
}
#[test]
fn object_urls_are_exact_resource_identities() {
    for (kind, path) in [
        (ObjectKind::Issue, "issues"),
        (ObjectKind::PullRequest, "pull"),
    ] {
        let object =
            Object::parse(&format!("https://github.com/Example/Project/{path}/17")).unwrap();
        assert_eq!(object.kind, kind);
        assert_eq!(object.number, 17);
        assert_eq!(
            object.url(),
            format!("https://github.com/example/project/{path}/17")
        );
        let mut invalid = object;
        invalid.number = 0;
        assert!(invalid.validate().is_err());
        invalid.number = u64::MAX;
        assert!(invalid.validate().is_err());
    }
    for suffix in [
        "issues/0",
        "pulls/17",
        "pull/-1",
        "pull/no",
        "pull/17/",
        "pull/17?x=1",
        "pull/17#comment",
        "pull/18446744073709551615",
        "pull/%31",
        "pull/../17",
    ] {
        assert!(
            Object::parse(&format!("https://github.com/example/project/{suffix}")).is_err(),
            "{suffix}"
        );
    }
}
