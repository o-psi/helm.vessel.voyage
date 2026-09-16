use super::*;
struct Fixture {
    _root: tempfile::TempDir,
    broker: Arc<BrowserBroker>,
    context: ToolContext,
    principal: Uuid,
    binding: BrowserBinding,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let session = crate::session::Session::new(root.path().into(), "fixture".into());
        let mut journal = Journal::open(directory.clone()).unwrap();
        journal.create_session(&session).unwrap();
        let incarnation = Uuid::new_v4();
        let broker = BrowserBroker::open(directory, session.id, incarnation).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal
            .admit_turn(
                &guard,
                &crate::attachment::journal::TurnAdmission {
                    coordination: None,
                    operator_name: None,
                    command_id: Uuid::new_v4(),
                    machine_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                    session_id: session.id,
                    expected_revision: 0,
                    expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
                    prompt: "browser fixture".into(),
                    parts: vec![],
                },
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap()
            .run
            .id;
        broker.begin_run(run, Default::default()).unwrap();
        let binding = BrowserBinding {
            session_id: session.id,
            incarnation,
            run_id: Some(run),
            browser_id: Uuid::new_v4(),
            resource_id: Uuid::new_v4(),
            executor_id: Uuid::new_v4(),
            controller_epoch: 1,
            capture_epoch: 1,
            expires_at_ms: now() + 50000,
        };
        let principal = Uuid::new_v4();
        let context = crate::tools::reliability_tests::context(root.path());
        broker
            .operate(
                principal,
                BrowserOperation::Offer {
                    command_id: Uuid::new_v4(),
                    binding: binding.clone(),
                },
            )
            .unwrap();
        Self {
            _root: root,
            broker,
            context,
            principal,
            binding,
        }
    }
    fn enqueue(&self, action: BrowserAction) -> BrowserRequest {
        let id = Uuid::new_v4();
        self.broker.enqueue(action, &self.context, id).unwrap();
        let BrowserReply::Pending { requests } = self
            .broker
            .operate(
                self.principal,
                BrowserOperation::Pending {
                    binding: self.binding.clone(),
                    limit: 16,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        requests.into_iter().find(|r| r.request_id == id).unwrap()
    }
    fn claim(&self, r: &BrowserRequest) -> BrowserOperation {
        BrowserOperation::Claim {
            command_id: Uuid::new_v4(),
            binding: r.binding.clone(),
            request_id: r.request_id,
            action_sha256: r.action_sha256.clone(),
        }
    }
    fn result(&self, r: &BrowserRequest, state: BrowserRequestState) -> BrowserResult {
        BrowserResult {
            request_id: r.request_id,
            action_sha256: r.action_sha256.clone(),
            state,
            text: "fixture result".into(),
            page_id: None,
            observation_id: None,
            image: None,
            file: None,
            local_reason: None,
        }
    }
}
#[test]
fn shared_browser_request_claim_result_and_exact_receipts() {
    let f = Fixture::new();
    assert!(f.broker.available());
    let request = f.enqueue(BrowserAction::Inspect { page_id: None });
    let claim = f.claim(&request);
    let receipt = f.broker.operate(f.principal, claim.clone()).unwrap();
    assert!(
        matches!(receipt,BrowserReply::Receipt{receipt} if receipt.state==BrowserRequestState::Dispatched&&receipt.cleanup_pending)
    );
    assert!(matches!(
        f.broker.operate(f.principal, claim).unwrap(),
        BrowserReply::Receipt { .. }
    ));
    assert!(f.broker.operate(f.principal, f.claim(&request)).is_err());
    let mut result = f.result(&request, BrowserRequestState::Completed);
    result.page_id = Some(Uuid::new_v4());
    result.observation_id = Some(Uuid::new_v4());
    let operation = BrowserOperation::Result {
        command_id: Uuid::new_v4(),
        binding: request.binding.clone(),
        result: result.clone(),
    };
    f.broker.operate(f.principal, operation.clone()).unwrap();
    f.broker.operate(f.principal, operation).unwrap();
    let (receipt, capture) = f.broker.poll(request.request_id, false).unwrap();
    assert_eq!(receipt.state, BrowserRequestState::Completed);
    assert_eq!(capture.unwrap().text, "fixture result");
    assert!(
        f.broker
            .poll(request.request_id, false)
            .unwrap()
            .1
            .is_none()
    );
    let target = BrowserTarget {
        page_id: result.page_id.unwrap(),
        observation_id: result.observation_id.unwrap(),
    };
    let click = f.enqueue(BrowserAction::Click {
        target,
        element: "button".into(),
    });
    assert_eq!(click.binding.run_id, f.binding.run_id);
    assert!(
        f.broker
            .enqueue(
                BrowserAction::Inspect { page_id: None },
                &f.context,
                request.request_id
            )
            .is_err()
    );
    assert!(f.broker.finish_run().unwrap());
}
#[test]
fn private_takeover_withholds_capture_and_preserves_uncertain_cleanup() {
    let f = Fixture::new();
    let request = f.enqueue(BrowserAction::Inspect { page_id: None });
    f.broker.operate(f.principal, f.claim(&request)).unwrap();
    let mut advanced = f.binding.clone();
    advanced.controller_epoch += 1;
    advanced.capture_epoch += 1;
    f.broker
        .operate(
            f.principal,
            BrowserOperation::Control {
                command_id: Uuid::new_v4(),
                binding: advanced.clone(),
                control: BrowserControl::Private,
            },
        )
        .unwrap();
    assert!(!f.broker.available());
    let (receipt, result) = f.broker.poll(request.request_id, false).unwrap();
    assert_eq!(receipt.state, BrowserRequestState::Unresolved);
    assert!(receipt.cleanup_pending && result.is_none());
    assert!(!f.broker.finish_run().unwrap());
    assert!(
        f.broker
            .begin_run(Uuid::new_v4(), Default::default())
            .is_err()
    );
    f.broker
        .operate(
            f.principal,
            BrowserOperation::Cleanup {
                command_id: Uuid::new_v4(),
                binding: request.binding.clone(),
                request_id: request.request_id,
                observed: true,
            },
        )
        .unwrap();
    assert!(f.broker.finish_run().unwrap());
    let BrowserReply::Receipt { receipt } = f
        .broker
        .operate(
            f.principal,
            BrowserOperation::Receipt {
                binding: request.binding.clone(),
                request_id: request.request_id,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(receipt.state, BrowserRequestState::Unresolved);
    assert!(!receipt.cleanup_pending);
}
#[test]
fn cancelled_pending_and_dispatched_requests_have_distinct_outcomes() {
    for dispatched in [false, true] {
        let f = Fixture::new();
        let request = f.enqueue(BrowserAction::Inspect { page_id: None });
        if dispatched {
            f.broker.operate(f.principal, f.claim(&request)).unwrap();
        }
        let (receipt, capture) = f.broker.poll(request.request_id, true).unwrap();
        assert_eq!(
            receipt.state,
            if dispatched {
                BrowserRequestState::Unresolved
            } else {
                BrowserRequestState::Cancelled
            }
        );
        assert_eq!(receipt.cleanup_pending, dispatched);
        assert!(capture.is_none());
        assert!(f.broker.operate(f.principal, f.claim(&request)).is_err());
    }
}
#[test]
fn authority_identity_and_invalid_results_refuse_without_losing_pending_work() {
    let f = Fixture::new();
    let request = f.enqueue(BrowserAction::Inspect { page_id: None });
    for limit in [0, 17] {
        assert!(
            f.broker
                .operate(
                    f.principal,
                    BrowserOperation::Pending {
                        binding: f.binding.clone(),
                        limit
                    }
                )
                .is_err()
        );
    }
    assert!(f.broker.operate(Uuid::new_v4(), f.claim(&request)).is_err());
    let mut stale = f.binding.clone();
    stale.incarnation = Uuid::new_v4();
    assert!(
        f.broker
            .operate(
                f.principal,
                BrowserOperation::Pending {
                    binding: stale,
                    limit: 1
                }
            )
            .is_err()
    );
    f.broker.operate(f.principal, f.claim(&request)).unwrap();
    for state in [
        BrowserRequestState::Pending,
        BrowserRequestState::Dispatched,
    ] {
        assert!(
            f.broker
                .operate(
                    f.principal,
                    BrowserOperation::Result {
                        command_id: Uuid::new_v4(),
                        binding: request.binding.clone(),
                        result: f.result(&request, state)
                    }
                )
                .is_err()
        );
    }
    let mut invalid = f.result(&request, BrowserRequestState::Completed);
    invalid.action_sha256 = "bad".into();
    assert!(
        f.broker
            .operate(
                f.principal,
                BrowserOperation::Result {
                    command_id: Uuid::new_v4(),
                    binding: request.binding.clone(),
                    result: invalid
                }
            )
            .is_err()
    );
    assert_eq!(
        f.broker.poll(request.request_id, false).unwrap().0.state,
        BrowserRequestState::Dispatched
    );
}

#[test]
fn result_bounds_observation_fencing_and_reopen_never_replay_effects() {
    let f = Fixture::new();
    let request = f.enqueue(BrowserAction::Inspect { page_id: None });
    f.broker.operate(f.principal, f.claim(&request)).unwrap();
    for field in ["text", "image", "file"] {
        let mut result = f.result(&request, BrowserRequestState::Completed);
        match field {
            "text" => result.text = "x".repeat(MAX_BROWSER_RESULT_BYTES + 1),
            "image" => {
                result.image = Some(BrowserImage {
                    mime_type: "image/png".into(),
                    data_base64: "invalid".into(),
                })
            }
            "file" => {
                result.file = Some(BrowserFile {
                    name: "x".into(),
                    mime_type: "text/plain".into(),
                    data_base64: "aGVsbG8=".into(),
                })
            }
            _ => unreachable!(),
        };
        assert!(
            f.broker
                .operate(
                    f.principal,
                    BrowserOperation::Result {
                        command_id: Uuid::new_v4(),
                        binding: request.binding.clone(),
                        result
                    }
                )
                .is_err(),
            "{field}"
        );
    }
    let reopened = BrowserBroker::open(
        f.broker.directory.clone(),
        f.binding.session_id,
        Uuid::new_v4(),
    )
    .unwrap();
    assert!(!reopened.available());
    assert!(reopened.blocks_suspension().unwrap());
    assert!(
        reopened
            .begin_run(Uuid::new_v4(), Default::default())
            .is_err()
    );
    let BrowserReply::Receipt { receipt } = reopened
        .operate(
            f.principal,
            BrowserOperation::Receipt {
                binding: request.binding.clone(),
                request_id: request.request_id,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(receipt.state, BrowserRequestState::Unresolved);
    assert!(receipt.cleanup_pending);
}
#[test]
fn observation_identifiers_and_cancelled_context_are_checked_before_enqueue() {
    let f = Fixture::new();
    let target = BrowserTarget {
        page_id: Uuid::new_v4(),
        observation_id: Uuid::new_v4(),
    };
    assert!(
        f.broker
            .enqueue(
                BrowserAction::Click {
                    target,
                    element: "button".into()
                },
                &f.context,
                Uuid::new_v4()
            )
            .is_err()
    );
    let context = f.context.clone();
    context.cancellation.cancel();
    assert!(
        f.broker
            .enqueue(
                BrowserAction::Inspect { page_id: None },
                &context,
                Uuid::new_v4()
            )
            .is_err()
    );
    assert!(f.broker.inner.lock().unwrap().durable.entries.is_empty());
}
