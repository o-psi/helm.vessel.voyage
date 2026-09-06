use super::*;
fn setup() -> EnrollmentStore {
    let mut store = EnrollmentStore::initialize(
        Connection::open_in_memory().unwrap(),
        "https://vessel.example".into(),
    )
    .unwrap();
    for n in 1..=3 {
        let id = Uuid::from_u128(n);
        store
            .db
            .execute(
                "INSERT INTO machines VALUES(?1,?2,1,0)",
                params![id.to_string(), vec![n as u8; 32]],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO audit(machine_id,kind,time) VALUES(?1,'enrolled',1)",
                [id.to_string()],
            )
            .unwrap();
    }
    // Persisted source data is not returned, even if credentials contain a canary.
    store.observe_time(1).unwrap();
    store
}
fn request(limit: u16, cursor: Option<String>) -> Request {
    Request {
        limit,
        cursor,
        after: 0,
    }
}
#[test]
fn machines_are_stable_or_fail_on_rotation_without_becoming_authority() {
    let mut s = setup();
    let first = s.inspect_machines(&request(1, None), 1).unwrap();
    assert_eq!(first.machines[0].machine_id, Uuid::from_u128(1));
    let cursor = first.next.unwrap();
    let second = s
        .inspect_machines(&request(1, Some(cursor.clone())), 2)
        .unwrap();
    assert_eq!(second.machines[0].machine_id, Uuid::from_u128(2));
    s.db.execute(
        "UPDATE machines SET epoch=2 WHERE id=?1",
        [Uuid::from_u128(2).to_string()],
    )
    .unwrap();
    s.db.execute(
        "INSERT INTO audit(machine_id,kind,time) VALUES(?1,'rotated',2)",
        [Uuid::from_u128(2).to_string()],
    )
    .unwrap();
    assert_eq!(
        s.inspect_machines(&request(1, Some(cursor)), 2)
            .unwrap_err(),
        EnrollmentError::Conflict
    );
    assert!(s.current(Uuid::from_u128(2), 1).is_err());
    assert_eq!(
        s.inspect_machines(&request(100, None), 2).unwrap().machines[1].epoch,
        2
    );
}
#[test]
fn audit_pins_upper_bound_and_supports_explicit_bookmarks() {
    let mut s = setup();
    let first = s.inspect_audit(&request(1, None), 1).unwrap();
    assert_eq!(first.through, 3);
    s.db.execute(
        "INSERT INTO audit(machine_id,kind,time) VALUES(NULL,'invitation_created',2)",
        [],
    )
    .unwrap();
    let second = s.inspect_audit(&request(1, first.next), 2).unwrap();
    assert_eq!(second.events[0].sequence, 2);
    assert_eq!(second.through, 3);
    let final_page = s.inspect_audit(&request(1, second.next), 2).unwrap();
    assert_eq!(final_page.events[0].sequence, 3);
    assert!(final_page.next.is_none());
    let next = s
        .inspect_audit(
            &Request {
                limit: 100,
                cursor: None,
                after: 3,
            },
            2,
        )
        .unwrap();
    assert_eq!(next.through, 4);
    assert_eq!(next.events[0].sequence, 4);
    assert_eq!(
        s.inspect_audit(
            &Request {
                limit: 1,
                cursor: None,
                after: 5
            },
            2
        )
        .unwrap_err(),
        EnrollmentError::Conflict
    );
}
#[test]
fn cursors_are_authenticated_projection_owner_limit_and_deadline_bound() {
    let mut a = setup();
    let mut b = setup();
    let cursor = a
        .inspect_machines(&request(1, None), 1)
        .unwrap()
        .next
        .unwrap();
    assert_eq!(
        b.inspect_machines(&request(1, Some(cursor.clone())), 1)
            .unwrap_err(),
        EnrollmentError::Denied
    );
    assert_eq!(
        a.inspect_audit(&request(1, Some(cursor.clone())), 1)
            .unwrap_err(),
        EnrollmentError::Denied
    );
    assert_eq!(
        a.inspect_machines(&request(2, Some(cursor.clone())), 1)
            .unwrap_err(),
        EnrollmentError::Denied
    );
    assert_eq!(
        a.inspect_machines(&request(1, Some(cursor.clone())), 300001)
            .unwrap_err(),
        EnrollmentError::Conflict
    );
    let mut tampered = cursor.into_bytes();
    tampered[2] = if tampered[2] == b'A' { b'B' } else { b'A' };
    assert!(
        a.inspect_machines(&request(1, Some(String::from_utf8(tampered).unwrap())), 1)
            .is_err()
    );
}
#[test]
fn bounded_corruption_and_sequence_gaps_preserve_source_rows() {
    let mut s = setup();
    s.db.execute(
        "UPDATE audit SET kind=?1 WHERE sequence=2",
        ["X".repeat(65536)],
    )
    .unwrap();
    assert_eq!(
        s.inspect_audit(&request(100, None), 1).unwrap_err(),
        EnrollmentError::Storage
    );
    assert_eq!(
        s.db.query_row("SELECT length(kind) FROM audit WHERE sequence=2", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        65536
    );
    s.db.execute("DELETE FROM audit WHERE sequence=2", [])
        .unwrap();
    assert_eq!(
        s.inspect_audit(&request(100, None), 1).unwrap_err(),
        EnrollmentError::Storage
    );
    s.db.execute(
        "UPDATE machines SET id='' WHERE id=?1",
        [Uuid::from_u128(1).to_string()],
    )
    .unwrap();
    assert_eq!(
        s.inspect_machines(&request(100, None), 1).unwrap_err(),
        EnrollmentError::Storage
    );
}

#[test]
fn empty_store_invalid_bounds_and_corrupt_nulls_are_explicit() {
    let mut s = EnrollmentStore::initialize(
        Connection::open_in_memory().unwrap(),
        "https://vessel.example".into(),
    )
    .unwrap();
    let page = s.inspect_machines(&request(100, None), 0).unwrap();
    assert!(page.machines.is_empty());
    assert_eq!(page.revision, 0);
    let page = s.inspect_audit(&request(100, None), 0).unwrap();
    assert!(page.events.is_empty());
    assert_eq!(page.through, 0);
    assert_eq!(
        s.inspect_machines(&request(0, None), 0).unwrap_err(),
        EnrollmentError::Invalid
    );
    assert_eq!(
        s.inspect_audit(
            &Request {
                limit: 1,
                cursor: None,
                after: u64::MAX
            },
            0
        )
        .unwrap_err(),
        EnrollmentError::Invalid
    );
    let mut s = setup();
    s.db.execute(
        "UPDATE audit SET machine_id=?1 WHERE sequence=1",
        ["x".repeat(4096)],
    )
    .unwrap();
    assert_eq!(
        s.inspect_audit(&request(100, None), 1).unwrap_err(),
        EnrollmentError::Storage
    );
    assert_eq!(
        s.db.query_row(
            "SELECT length(machine_id) FROM audit WHERE sequence=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4096
    );
    let mut s = setup();
    s.db.execute("DELETE FROM audit WHERE sequence=1", [])
        .unwrap();
    assert_eq!(
        s.inspect_audit(&request(100, None), 1).unwrap_err(),
        EnrollmentError::Storage
    );
}

#[test]
fn cursor_survives_read_only_restart_and_clock_rollback_never_refreshes_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority");
    let mut s = EnrollmentStore::open(&path, "https://vessel.example", false).unwrap();
    for _ in 0..3 {
        s.invite(60000, 1000).unwrap();
    }
    let before: Vec<(i64, String)> =
        s.db.prepare("SELECT sequence,kind FROM audit ORDER BY sequence")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
    let cursor = s
        .inspect_audit(&request(1, None), 1000)
        .unwrap()
        .next
        .unwrap();
    drop(s);
    let mut s = EnrollmentStore::open(&path, "https://vessel.example", false).unwrap();
    let page = s
        .inspect_audit(&request(1, Some(cursor.clone())), 1001)
        .unwrap();
    assert_eq!(page.events[0].sequence, 2);
    assert!(
        s.inspect_audit(&request(1, Some(cursor.clone())), 999)
            .is_err()
    );
    assert_eq!(
        s.inspect_audit(&request(1, Some(cursor)), 301000)
            .unwrap_err(),
        EnrollmentError::Conflict
    );
    let after: Vec<(i64, String)> =
        s.db.prepare("SELECT sequence,kind FROM audit ORDER BY sequence")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
    assert_eq!(before, after);
}

#[test]
fn maximum_pages_are_bounded_and_end_without_duplicates() {
    let mut s = EnrollmentStore::initialize(
        Connection::open_in_memory().unwrap(),
        "https://vessel.example".into(),
    )
    .unwrap();
    for n in 1..=wire::MAX_PAGE + 1 {
        let id = Uuid::from_u128(n as u128);
        let key = vec![n as u8; 32];
        s.db.execute(
            "INSERT INTO machines VALUES(?1,?2,1,0)",
            params![id.to_string(), key],
        )
        .unwrap();
        s.db.execute(
            "INSERT INTO audit(machine_id,kind,time) VALUES(?1,'enrolled',1)",
            [id.to_string()],
        )
        .unwrap();
    }
    let first = s
        .inspect_machines(&request(wire::MAX_PAGE as u16, None), 1)
        .unwrap();
    assert_eq!(first.machines.len(), wire::MAX_PAGE);
    let last = s
        .inspect_machines(&request(wire::MAX_PAGE as u16, first.next), 1)
        .unwrap();
    assert_eq!(last.machines.len(), 1);
    assert!(last.next.is_none());
    assert_eq!(
        last.machines[0].machine_id,
        Uuid::from_u128((wire::MAX_PAGE + 1) as u128)
    );
    let first = s
        .inspect_audit(&request(wire::MAX_PAGE as u16, None), 1)
        .unwrap();
    assert_eq!(first.events.len(), wire::MAX_PAGE);
    let last = s
        .inspect_audit(&request(wire::MAX_PAGE as u16, first.next), 1)
        .unwrap();
    assert_eq!(last.events.len(), 1);
    assert!(last.next.is_none());
    assert_eq!(last.events[0].sequence, (wire::MAX_PAGE + 1) as u64);
}
