use super::*;
fn draft(action: Action) -> Draft {
    Draft {
        object: Object::parse("https://github.com/example/project/pull/1").unwrap(),
        action,
    }
}
fn inline() -> InlineComment {
    InlineComment {
        path: "src/lib.rs".into(),
        line: 2,
        side: Side::Right,
        start_line: None,
        start_side: None,
        body: "Please explain this".into(),
    }
}
#[test]
fn publication_validation_and_digests_bind_exact_content() {
    let actor = Actor {
        id: 1,
        login: "actor".into(),
    };
    let comment = draft(Action::Comment {
        body: "hello".into(),
    });
    comment.validate().unwrap();
    assert_eq!(comment.body(), serde_json::json!({"body":"hello"}));
    assert_eq!(comment.path(), "/repos/example/project/issues/1/comments");
    let digest = comment.digest(&actor, "policy").unwrap();
    assert_eq!(digest.len(), 64);
    assert_ne!(digest, comment.digest(&actor, "other").unwrap());
    assert_ne!(
        digest,
        comment
            .digest(
                &Actor {
                    id: 2,
                    ..actor.clone()
                },
                "policy"
            )
            .unwrap()
    );
    for body in ["".into(), " \n".into(), "a".repeat(65537)] {
        assert!(draft(Action::Comment { body }).validate().is_err());
    }
    for event in [
        ReviewEvent::Comment,
        ReviewEvent::Approve,
        ReviewEvent::RequestChanges,
    ] {
        let mut review = draft(Action::Review {
            event,
            commit_id: "a".repeat(40),
            body: "review".into(),
            comments: vec![inline()],
        });
        review.validate().unwrap();
        assert_eq!(review.path(), "/repos/example/project/pulls/1/reviews");
        assert_eq!(review.body()["comments"].as_array().unwrap().len(), 1);
        if let Action::Review { body, .. } = &mut review.action {
            body.clear();
        }
        assert_eq!(review.validate().is_ok(), event == ReviewEvent::Approve);
        review.object.kind = ObjectKind::Issue;
        assert!(review.validate().is_err());
    }
    let mut comments = vec![];
    for bad in 0..9 {
        let mut c = inline();
        match bad {
            0 => c.path.clear(),
            1 => c.path = "../escape".into(),
            2 => c.path = "/absolute".into(),
            3 => c.line = 0,
            4 => c.body.clear(),
            5 => c.body = "x".repeat(16385),
            6 => c.start_line = Some(1),
            7 => {
                c.start_line = Some(3);
                c.start_side = Some(Side::Right)
            }
            _ => {
                c.start_line = Some(1);
                c.start_side = Some(Side::Left)
            }
        }
        comments.push(c);
    }
    for c in comments {
        assert!(
            draft(Action::Review {
                event: ReviewEvent::Approve,
                commit_id: "a".repeat(40),
                body: String::new(),
                comments: vec![c]
            })
            .validate()
            .is_err()
        );
    }
}
#[test]
fn complete_unified_hunks_define_reviewable_sides_and_ranges() {
    let patch = "@@ -1,3 +1,3 @@ function\n context\n-old\n+new\n tail\n\\ No newline at end of file\n@@ -10 +10 @@\n-last\n+next";
    let parsed = parse_patch(patch).unwrap();
    assert_eq!(parsed.additions, 2);
    assert_eq!(parsed.deletions, 2);
    assert!(parsed.left.contains(&2));
    assert!(parsed.right.contains(&10));
    let mut comment = inline();
    validate_inline(&comment, patch).unwrap();
    comment.start_line = Some(1);
    comment.start_side = Some(Side::Right);
    validate_inline(&comment, patch).unwrap();
    comment.line = 10;
    assert!(validate_inline(&comment, patch).is_err());
    comment.start_line = None;
    comment.start_side = None;
    comment.side = Side::Left;
    validate_inline(&comment, patch).unwrap();
    assert_eq!(parse_patch("@@ -0,0 +1 @@\n+new").unwrap().additions, 1);
    assert_eq!(parse_patch("@@ -1 +0,0 @@\n-old").unwrap().deletions, 1);
    for bad in [
        "",
        "before hunk",
        "@@ -1 +1 @@\n-old",
        "@@ -1 +1 @@\n same\n extra",
        "@@ -0 +1 @@\n x",
        "@@ -1 +1\n x",
        "@@ -4294967295 +1 @@\n x",
        "@@ -1 +1 @@\n?invalid",
        "@@ -1 +1 @@\n x\n@@ -1 +1 @@\n x",
        "@@ -1,2 +1,2 @@\n x\n@@ -10 +10 @@\n y",
    ] {
        assert!(parse_patch(bad).is_err(), "{bad}");
    }
}
