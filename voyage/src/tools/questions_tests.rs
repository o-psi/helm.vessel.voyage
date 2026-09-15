use super::*;
#[test]
fn question_and_answer_validation_preserve_exact_choices() {
    let question = Question {
        question: "Pick one".into(),
        options: vec!["one".into(), "two".into()],
    };
    question.validate().unwrap();
    for options in [
        vec![],
        vec!["one".into()],
        vec!["one".into(), " one ".into()],
        vec!["one".into(), "bad\noption".into()],
        vec!["x".repeat(257), "two".into()],
    ] {
        assert!(
            Question {
                options,
                ..question.clone()
            }
            .validate()
            .is_err()
        );
    }
    for text in [" ".into(), "bad\nquestion".into(), "é".repeat(1025)] {
        assert!(
            Question {
                question: text,
                ..question.clone()
            }
            .validate()
            .is_err()
        );
    }
    QuestionAnswer::Selected {
        index: 1,
        answer: "two".into(),
    }
    .validate(&question)
    .unwrap();
    for answer in [
        QuestionAnswer::Selected {
            index: 2,
            answer: "two".into(),
        },
        QuestionAnswer::Selected {
            index: 0,
            answer: "two".into(),
        },
        QuestionAnswer::Custom { answer: " ".into() },
        QuestionAnswer::Custom {
            answer: "x".repeat(4097),
        },
        QuestionAnswer::Custom {
            answer: "a\nb".into(),
        },
    ] {
        assert!(answer.validate(&question).is_err());
    }
    let redactor = crate::tools::Redactor::new(["fixture-secret".into()]);
    for answer in [
        QuestionAnswer::Cancelled,
        QuestionAnswer::Unavailable,
        QuestionAnswer::Custom {
            answer: "fixture-secret".into(),
        },
        QuestionAnswer::Selected {
            index: 0,
            answer: "fixture-secret".into(),
        },
    ] {
        let result = redact_result(&serde_json::to_string(&answer).unwrap(), &redactor).unwrap();
        assert!(!result.contains("fixture-secret"));
        serde_json::from_str::<QuestionAnswer>(&result).unwrap();
    }
    for bad in [
        r#"{"status":"cancelled","extra":"private"}"#,
        r#"{"status":"unavailable","answer":"invented"}"#,
        "invalid",
    ] {
        assert!(redact_result(bad, &redactor).is_err());
    }
}
#[tokio::test]
async fn unattended_questions_are_unavailable_not_invented() {
    let root = tempfile::tempdir().unwrap();
    let context = crate::tools::reliability_tests::context(root.path());
    let args = json!({"question":"Pick","options":["one","two"]});
    assert_eq!(
        serde_json::from_str::<QuestionAnswer>(
            &Questions.execute(args.clone(), &context).await.unwrap()
        )
        .unwrap(),
        QuestionAnswer::Unavailable
    );
    context.cancellation.cancel();
    assert!(matches!(
        Questions.execute(args, &context).await,
        Err(ToolError::Cancelled)
    ));
}
