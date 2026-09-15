use super::*;

fn cancel(run: &RunRecord) -> LocalCancelRequest {
    LocalCancelRequest {
        session_id: run.session_id,
        run_id: run.id,
        installation_id: run.machine_id,
        principal_id: run.principal_id,
        expires_at_ms: 61000,
    }
}

#[test]
fn catalogue_paginates_in_identity_order_without_losing_sessions() {
    let (_root, mut j, s, _g) = fixture();
    let mut expected = vec![s.id];
    for _ in 0..4 {
        let session = Session::new(s.workspace.clone(), "another-model".into());
        expected.push(session.id);
        j.create_session(&session).unwrap();
    }
    expected.sort();
    let mut actual = vec![];
    let mut after = None;
    loop {
        let page = j.list_session_summaries(after, 2).unwrap();
        actual.extend(page.sessions.iter().map(|s| s.id));
        assert!(
            page.sessions
                .iter()
                .all(|s| s.active_run.is_none() && s.pending_cleanup_run.is_none())
        );
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(actual, expected);
    assert!(j.list_session_summaries(None, 0).is_err());
    assert!(j.list_session_summaries(None, 101).is_err());
    assert!(
        j.list_session_summaries(expected.last().copied(), 10)
            .unwrap()
            .sessions
            .is_empty()
    );
}

#[test]
fn local_cancel_is_bound_to_actor_and_duplicate_does_not_recheck_clock() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    let request = cancel(&run);
    let mut wrong = request.clone();
    wrong.principal_id = Uuid::new_v4();
    assert!(
        j.request_cancel_local_with_clock(&wrong, || Ok(1000))
            .is_err()
    );
    assert!(!j.local_cancel_requested(s.id, run.id).unwrap());
    assert_eq!(
        j.request_cancel_local_with_clock(&request, || Ok(1000))
            .unwrap(),
        CancelRequestOutcome::Requested { duplicate: false }
    );
    assert_eq!(
        j.request_cancel_local_with_clock(&request, || panic!("duplicate must not consult clock"))
            .unwrap(),
        CancelRequestOutcome::Requested { duplicate: true }
    );
    assert!(j.local_cancel_requested(s.id, run.id).unwrap());
    assert!(j.local_cancel_requested(Uuid::new_v4(), run.id).is_err());
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    assert_eq!(
        j.request_cancel_local_with_clock(&request, || panic!("terminal must not consult clock"))
            .unwrap(),
        CancelRequestOutcome::AlreadyTerminal {
            state: RunState::Cancelled
        }
    );
}

#[test]
fn invalid_cancel_deadlines_leave_no_intent() {
    let (_root, mut j, s, g) = fixture();
    let run = admit(&mut j, &g);
    for (now, expiry) in [(-1, 10), (1000, 1000), (1000, 999), (1000, 301001)] {
        let mut request = cancel(&run);
        request.expires_at_ms = expiry;
        assert!(
            j.request_cancel_local_with_clock(&request, || Ok(now))
                .is_err()
        );
        assert!(!j.local_cancel_requested(s.id, run.id).unwrap());
    }
}

#[test]
fn cleanup_obligation_survives_terminalization_until_observed() {
    let (_root, mut j, _s, g) = fixture();
    let run = admit(&mut j, &g);
    j.register_local_cleanup(&g, run.id).unwrap();
    j.register_local_cleanup(&g, run.id).unwrap();
    let page = j.list_session_summaries(None, 10).unwrap();
    assert_eq!(page.sessions[0].pending_cleanup_run, Some(run.id));
    assert_eq!(page.sessions[0].active_run.as_ref().unwrap().id, run.id);
    assert!(j.confirm_local_cleanup_observed(&g, run.id).is_err());
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    let page = j.list_session_summaries(None, 10).unwrap();
    assert!(page.sessions[0].active_run.is_none());
    assert_eq!(page.sessions[0].pending_cleanup_run, Some(run.id));
    j.confirm_local_cleanup_observed(&g, run.id).unwrap();
    j.confirm_local_cleanup_observed(&g, run.id).unwrap();
    assert!(
        j.list_session_summaries(None, 10).unwrap().sessions[0]
            .pending_cleanup_run
            .is_none()
    );
    assert!(
        j.attest_local_cleanup(&g, run.id, run.machine_id, run.principal_id)
            .is_err()
    );
}

#[test]
fn cleanup_attestation_requires_original_actor_and_is_not_observation() {
    let (_root, mut j, _s, g) = fixture();
    let run = admit(&mut j, &g);
    j.register_local_cleanup(&g, run.id).unwrap();
    j.mark_running(&g, run.id).unwrap();
    assert!(j.register_local_cleanup(&g, run.id).is_err());
    j.finish(
        &g,
        run.id,
        RunState::Cancelled,
        Some("test cancellation"),
        None,
    )
    .unwrap();
    assert!(
        j.attest_local_cleanup(&g, run.id, Uuid::new_v4(), run.principal_id)
            .is_err()
    );
    j.attest_local_cleanup(&g, run.id, run.machine_id, run.principal_id)
        .unwrap();
    j.attest_local_cleanup(&g, run.id, run.machine_id, run.principal_id)
        .unwrap();
    assert!(j.confirm_local_cleanup_observed(&g, run.id).is_err());
}
