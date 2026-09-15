use super::*;
use serde_json::json;
fn request(section: Section) -> Read {
    Read {
        object: Object::parse("https://github.com/example/project/pull/7").unwrap(),
        section,
        page: 1,
        expected_head: None,
        expected_base: None,
    }
}
#[test]
fn typed_context_requests_enforce_resource_and_pagination_boundaries() {
    let sections = vec![
        Section::Details,
        Section::Comments,
        Section::Reviews,
        Section::Files,
        Section::ReviewComments { review: 1 },
        Section::Statuses,
        Section::Checks,
        Section::CheckSuites,
        Section::SuiteChecks { suite: 1 },
        Section::WorkflowRuns,
        Section::Jobs { run: 1 },
        Section::Annotations { check: 1 },
        Section::Threads {
            after: Some("cursor".into()),
        },
        Section::ThreadComments {
            thread: "thread".into(),
            after: None,
        },
    ];
    for section in sections {
        let mut read = request(section.clone());
        read.validate().unwrap();
        let encoded = serde_json::to_value(&read).unwrap();
        serde_json::from_value::<Read>(encoded)
            .unwrap()
            .validate()
            .unwrap();
        read.page = 0;
        assert!(read.validate().is_err());
        read.page = 1_000_001;
        assert!(read.validate().is_err());
        read.page = 1;
        read.object = Object::parse("https://github.com/example/project/issues/7").unwrap();
        assert_eq!(
            read.validate().is_ok(),
            matches!(section, Section::Details | Section::Comments)
        );
        read.expected_head = Some("a".repeat(40));
        assert!(read.validate().is_err());
        read.expected_head = None;
        read.expected_base = Some(Base {
            sha: "a".repeat(40),
            repository: 1,
            reference: "main".into(),
        });
        assert!(read.validate().is_err());
    }
    for (section, maximum) in [
        (Section::Details, 1),
        (Section::Files, 30),
        (Section::WorkflowRuns, 10),
        (Section::Comments, 1_000_000),
    ] {
        let mut read = request(section);
        read.page = maximum;
        read.validate().unwrap();
        read.page += 1;
        assert!(read.validate().is_err());
    }
    for section in [
        Section::Jobs { run: 0 },
        Section::Annotations { check: u64::MAX },
        Section::SuiteChecks { suite: 0 },
        Section::ReviewComments { review: 0 },
        Section::Threads {
            after: Some("".into()),
        },
        Section::ThreadComments {
            thread: "\n".into(),
            after: None,
        },
    ] {
        assert!(request(section).validate().is_err());
    }
    let mut read = request(Section::Threads { after: None });
    read.page = 2;
    assert!(read.validate().is_err());
}
#[test]
fn commit_and_base_identity_are_strict_and_normalized() {
    assert_eq!(sha(&json!("A".repeat(40))).unwrap(), "a".repeat(40));
    for value in [
        json!(null),
        json!(17),
        json!("a".repeat(39)),
        json!("g".repeat(40)),
    ] {
        assert!(sha(&value).is_err());
    }
    let detail = json!({"base":{"sha":"B".repeat(40),"repo":{"id":17},"ref":"main"}});
    assert_eq!(
        base(&detail).unwrap(),
        Base {
            sha: "b".repeat(40),
            repository: 17,
            reference: "main".into()
        }
    );
    for (pointer, value) in [
        ("/base/sha", json!("bad")),
        ("/base/repo/id", json!(0)),
        ("/base/ref", json!("")),
        ("/base/ref", json!("x\ny")),
        ("/base/ref", json!("x".repeat(1025))),
    ] {
        let mut invalid = detail.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(base(&invalid).is_err());
    }
}
#[test]
fn graphql_completeness_never_silently_accepts_missing_pages() {
    let connection = |nodes: Value, total: u64, more: bool| json!({"nodes":nodes,"totalCount":total,"pageInfo":{"hasNextPage":more}});
    assert!(!graphql_connection(&connection(json!([]), 0, false), 100, true).unwrap());
    assert!(graphql_connection(&connection(json!([{}]), 2, true), 100, true).unwrap());
    assert!(!graphql_connection(&connection(json!([{}]), 2, false), 100, false).unwrap());
    for value in [
        json!({}),
        connection(json!({}), 0, false),
        connection(json!([]), 1, true),
        connection(json!([{}]), 0, false),
        connection(json!([{}, {}]), 2, false),
        connection(json!([{}]), 1, true),
        connection(json!([{}]), 2, false),
    ] {
        assert!(graphql_connection(&value, 1, true).is_err(), "{value}");
    }
    assert!(collection_coverage_consistent(Some(101), 2, 1, false));
    assert!(collection_coverage_consistent(Some(101), 1, 100, true));
    assert!(!collection_coverage_consistent(None, 1, 0, false));
    assert!(!collection_coverage_consistent(Some(101), 1, 100, false));
    assert!(!collection_coverage_consistent(Some(99), 1, 100, true));
}
#[test]
fn rest_continuations_rebuild_only_same_resource_and_query() {
    use reqwest::header::{HeaderMap, HeaderValue, LINK};
    let path = "/repos/example/project/issues/7/comments";
    let mut headers = HeaderMap::new();
    assert_eq!(next_page(&headers, 1, path, "").unwrap(), None);
    for url in [
        format!("https://api.github.com{path}?per_page=100&page=2"),
        "https://api.github.com/repositories/17/issues/7/comments?page=2&per_page=100".into(),
    ] {
        headers.insert(
            LINK,
            HeaderValue::from_str(&format!("<{url}>; rel=\"next\"")).unwrap(),
        );
        assert_eq!(next_page(&headers, 1, path, "").unwrap(), Some(2));
    }
    for url in [
        format!("http://api.github.com{path}?per_page=100&page=2"),
        format!("https://evil.invalid{path}?per_page=100&page=2"),
        format!("https://api.github.com{path}?per_page=100&page=3"),
        format!("https://api.github.com{path}?per_page=100&page=2&page=2"),
        format!("https://api.github.com{path}?per_page=100&page=2&extra=1"),
        format!("https://api.github.com{path}?per_page=100&page=2#fragment"),
        "https://api.github.com/repos/other/project/issues/7/comments?per_page=100&page=2".into(),
    ] {
        headers.insert(
            LINK,
            HeaderValue::from_str(&format!("<{url}>; rel=\"next\"")).unwrap(),
        );
        assert!(next_page(&headers, 1, path, "").is_err(), "{url}");
    }
    for text in [
        "malformed".to_owned(),
        format!(
            "<https://api.github.com{path}?per_page=100&page=2>; rel=\"next\", <https://api.github.com{path}?per_page=100&page=2>; rel=\"next\""
        ),
    ] {
        headers.insert(LINK, HeaderValue::from_str(&text).unwrap());
        assert!(next_page(&headers, 1, path, "").is_err());
    }
}
