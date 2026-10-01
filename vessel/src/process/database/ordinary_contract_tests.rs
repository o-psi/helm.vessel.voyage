//! Ordinary catalogue metadata never grants execution or fabricates freshness.
use super::*;

#[tokio::test]
async fn invalid_registration_publications_are_atomic_and_leave_prior_receipt() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let original = f.registration();
    admit(&f.0, &original, bytes(&original)).await.unwrap();
    let cursor = catalogue_changes(&f.0, None, 128).await.unwrap().cursor;
    for variant in 0..4 {
        let mut attempted = original.clone();
        match variant {
            0 => {
                attempted.incarnation = Uuid::new_v4();
                attempted.restart_from = None;
            }
            1 => {
                attempted.session_id = Uuid::new_v4();
            }
            2 => {
                attempted.name = Some("x".repeat(17000));
            }
            _ => {
                attempted.peer_uids = Some(ProcessPeerUids {
                    supervisor: unsafe { libc::geteuid() },
                    runtime: 1001,
                });
            }
        }
        assert!(save(&f.0, &attempted).await.is_err());
        assert_eq!(
            registration(&f.0, original.session_id)
                .await
                .unwrap()
                .incarnation,
            original.incarnation
        );
        assert_eq!(
            catalogue_changes(&f.0, Some(cursor), 128)
                .await
                .unwrap()
                .cursor,
            cursor
        );
    }
    assert!(
        creation_receipt(&f.0, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn malformed_or_stale_process_projection_never_overrides_current_registration() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    for raw in ["{}".to_owned(), "[]".into(), {
        let mut stale = ProcessInfo::from(&r);
        stale.incarnation = Uuid::new_v4();
        stale.state = ProcessState::Live;
        serde_json::to_string(&stale).unwrap()
    }] {
        let db = open(&f.0).unwrap();
        db.execute(
            "UPDATE catalogue SET process_info=?1,observed_at_ms=?2",
            params![raw, now()],
        )
        .unwrap();
        drop(db);
        let info = catalogue(&f.0).await.unwrap().pop().unwrap();
        assert_eq!(info.incarnation, r.incarnation);
        assert_ne!(info.state, ProcessState::Live);
    }
    let mut live = ProcessInfo::from(&r);
    live.state = ProcessState::Live;
    let db = open(&f.0).unwrap();
    db.execute(
        "UPDATE catalogue SET process_info=?1,observed_at_ms=?2",
        params![serde_json::to_string(&live).unwrap(), now() - 6000],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        catalogue(&f.0).await.unwrap()[0].state,
        ProcessState::Unavailable
    );
    let db = open(&f.0).unwrap();
    db.execute("UPDATE catalogue SET observed_at_ms=-1", [])
        .unwrap();
    drop(db);
    assert!(
        catalogue(&f.0).await.unwrap()[0]
            .catalogue
            .as_ref()
            .unwrap()
            .observed_at_ms
            .is_none()
    );
}

#[tokio::test]
async fn catalogue_summary_name_and_error_do_not_discard_retained_observations() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let summary = CatalogueSummary {
        session_id: r.session_id,
        revision: 10,
        observation_cursor: 19,
        name: Some("Retained actual name".into()),
        model: "fixture".into(),
        created_at: None,
        last_turn_end: None,
        total_messages: 9,
        run_id: None,
        run_state: None,
        archived: false,
        deleted: false,
        pending_cleanup_run: Some(Uuid::new_v4()),
    };
    let db = open(&f.0).unwrap();
    db.execute(
        "UPDATE catalogue SET summary=?1,error_code='journal_unavailable',observed_at_ms=?2",
        params![serde_json::to_string(&summary).unwrap(), now()],
    )
    .unwrap();
    drop(db);
    let info = catalogue(&f.0).await.unwrap().pop().unwrap();
    assert_eq!(info.name, summary.name);
    let metadata = info.catalogue.unwrap();
    assert!(metadata.stale);
    assert_eq!(metadata.summary, Some(summary));
    assert_eq!(metadata.error_code.as_deref(), Some("journal_unavailable"));
    let db = open(&f.0).unwrap();
    db.execute("UPDATE catalogue SET summary='{}'", []).unwrap();
    drop(db);
    assert!(catalogue(&f.0).await.is_err());
}

#[tokio::test]
async fn catalogue_cursor_bounds_and_future_cursor_report_recovery_without_writes() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    let cursor = catalogue_changes(&f.0, None, 128).await.unwrap().cursor;
    for (after, limit) in [(Some(cursor), 0), (Some(cursor), 129), (Some(u64::MAX), 1)] {
        assert!(catalogue_changes(&f.0, after, limit).await.is_err());
    }
    let gap = catalogue_changes(&f.0, Some(cursor + 1), 1).await.unwrap();
    assert!(gap.replay_gap);
    assert_eq!(gap.cursor, cursor);
    assert!(gap.entries.is_empty());
}

#[tokio::test]
async fn failed_forced_refresh_backoff_is_bounded_and_does_not_claim_running() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    for _ in 0..9 {
        refresh_now(&f.0, &r).await.unwrap();
    }
    let db = open(&f.0).unwrap();
    let (failures, next): (i64, i64) = db
        .query_row(
            "SELECT failures,next_attempt_ms FROM catalogue",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(failures, 6);
    assert!(next > now());
    assert!(next <= now() + 60000);
    drop(db);
    let before = catalogue_changes(&f.0, None, 128).await.unwrap().cursor;
    refresh(&f.0, &r).await.unwrap();
    assert_eq!(
        catalogue_changes(&f.0, Some(before), 128)
            .await
            .unwrap()
            .cursor,
        before
    );
    let info = catalogue(&f.0).await.unwrap().pop().unwrap();
    assert_eq!(info.state, ProcessState::Unavailable);
    assert!(info.catalogue.unwrap().stale);
}

#[tokio::test]
async fn registration_reads_and_serial_guard_refuse_malformed_authoritative_state() {
    let f = Fixture::new();
    initialize(&f.0).await.unwrap();
    let r = f.registration();
    admit(&f.0, &r, bytes(&r)).await.unwrap();
    assert!(registration(&f.0, Uuid::new_v4()).await.is_err());
    let db = open(&f.0).unwrap();
    db.execute("UPDATE voyages SET registration='{}'", [])
        .unwrap();
    drop(db);
    assert!(registration(&f.0, r.session_id).await.is_err());
    assert!(Registrations::new(f.0.clone()).lock().await.is_err());
}
