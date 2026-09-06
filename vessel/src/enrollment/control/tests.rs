use super::*;
fn setup() -> (EnrollmentStore, u64, Peer, Address) {
    let mut store = EnrollmentStore::initialize(
        Connection::open_in_memory().unwrap(),
        "https://vessel.example".into(),
    )
    .unwrap();
    let m = Uuid::new_v4();
    store
        .db
        .execute(
            "INSERT INTO machines VALUES(?1,?2,1,0)",
            params![m.to_string(), vec![1u8; 32]],
        )
        .unwrap();
    let g = store.enable_control().unwrap();
    let peer = Peer {
        machine: control::Machine {
            machine_id: m,
            epoch: 1,
        },
        owner: store.owner,
        connection: Uuid::new_v4(),
        live: Arc::new(|| true),
    };
    (
        store,
        g,
        peer,
        Address {
            installation_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
        },
    )
}
fn register(address: Address) -> ClientOperation {
    ClientOperation::Register {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address,
        source_revision: 0,
    }
}
#[test]
fn registration_retry_cas_revocation_and_historical_receipt() {
    let (mut s, g, p, a) = setup();
    let op = register(a);
    let control::Reply::Registered {
        record,
        duplicate: false,
    } = s.control_peer(g, &p, &op, || Ok(1000)).unwrap()
    else {
        panic!()
    };
    assert_eq!(record.execution_authority, ExecutionAuthority::None);
    assert!(matches!(
        s.control_peer(g, &p, &op, || Err(EnrollmentError::Invalid))
            .unwrap(),
        control::Reply::Registered {
            duplicate: true,
            ..
        }
    ));
    let mut request = Mutation {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address: a,
        expected_revision: 1,
        operation: MutationKind::Configure {
            coordinator: p.machine,
            participants: vec![p.machine],
        },
    };
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000))
            .unwrap()
            .record
            .revision,
        2
    );
    assert!(
        s.control_mutate(g, &request, || Ok(3000))
            .unwrap()
            .duplicate
    );
    request.expected_revision = 2;
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Conflict
    );
    request.command_id = Uuid::new_v4();
    request.operation = MutationKind::Revoke {};
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000))
            .unwrap()
            .record
            .status,
        Status::Revoked
    );
    let control::Reply::Registered { record, .. } =
        s.control_peer(g, &p, &op, || Ok(1000)).unwrap()
    else {
        panic!()
    };
    assert_eq!(record.revision, 1);
    let control::Reply::Snapshot { views } = s
        .control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap()
    else {
        panic!()
    };
    assert!(views[0].lease.is_none());
}
#[test]
fn scope_and_connection_epochs_fence_control_leases() {
    let (mut s, g, mut p, a) = setup();
    s.control_peer(g, &p, &register(a), || Ok(1000)).unwrap();
    let control::Reply::Snapshot { views } = s
        .control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap()
    else {
        panic!()
    };
    let lease = views[0].lease.as_ref().unwrap();
    assert_eq!(lease.expires_at_ms, 11000);
    let control::Reply::Snapshot { views } = s
        .control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1001))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(lease.lease_id, views[0].lease.as_ref().unwrap().lease_id);
    assert!(s.control_inspect(g, a, || Ok(11002)).unwrap().1.is_empty());
    p.live = Arc::new(|| false);
    assert_eq!(
        s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(12000))
            .unwrap_err(),
        EnrollmentError::Denied
    );
    p.live = Arc::new(|| true);
    p.machine.epoch = 2;
    assert_eq!(
        s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(12000))
            .unwrap_err(),
        EnrollmentError::Denied
    );
    p.machine.epoch = 1;
    let next = s.enable_control().unwrap();
    assert!(next > g);
    assert_eq!(
        s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(12000))
            .unwrap_err(),
        EnrollmentError::Conflict
    );
    assert!(
        s.control_inspect(next, a, || Ok(12000))
            .unwrap()
            .1
            .is_empty()
    );
}
#[test]
fn denied_registration_and_expiry_leave_no_record() {
    let (mut s, g, mut p, a) = setup();
    let op = register(a);
    assert_eq!(
        s.control_peer(g, &p, &op, || Ok(2000)).unwrap_err(),
        EnrollmentError::Invalid
    );
    assert!(s.control_inspect(g, a, || Ok(2000)).is_err());
    p.owner = Uuid::new_v4();
    assert_eq!(
        s.control_peer(g, &p, &op, || Ok(1000)).unwrap_err(),
        EnrollmentError::Denied
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM coordination_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn corrupt_release_and_unknown_restart_schema_preserve_evidence() {
    let (mut s, g, p, a) = setup();
    s.control_peer(g, &p, &register(a), || Ok(1000)).unwrap();
    s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap();
    s.db.execute(
        "UPDATE coordination_leases SET lease=?1",
        ["X".repeat(4097)],
    )
    .unwrap();
    assert_eq!(
        s.control_peer(g, &p, &ClientOperation::Release {}, || Ok(1000))
            .unwrap_err(),
        EnrollmentError::Storage
    );
    assert_eq!(
        s.db.query_row("SELECT length(lease) FROM coordination_leases", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        4097
    );
    s.db.execute("UPDATE coordination_schema SET version=999", [])
        .unwrap();
    assert_eq!(s.enable_control().unwrap_err(), EnrollmentError::Conflict);
    assert_eq!(
        s.db.query_row("SELECT version FROM coordination_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        999
    );
}
#[test]
fn generation_and_record_revision_overflow_fail_without_replacement() {
    let (mut s, g, p, a) = setup();
    s.control_peer(g, &p, &register(a), || Ok(1000)).unwrap();
    let mut record = s.control_inspect(g, a, || Ok(1000)).unwrap().0;
    record.revision = i64::MAX as u64;
    s.db.execute(
        "UPDATE coordination_records SET record=?1",
        [json(&record).unwrap()],
    )
    .unwrap();
    let request = Mutation {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address: a,
        expected_revision: record.revision,
        operation: MutationKind::Revoke {},
    };
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Capacity
    );
    assert_eq!(s.control_inspect(g, a, || Ok(1000)).unwrap().0, record);
    s.db.execute("UPDATE coordination_schema SET generation=?1", [i64::MAX])
        .unwrap();
    assert_eq!(s.enable_control().unwrap_err(), EnrollmentError::Capacity);
    assert_eq!(
        s.db.query_row("SELECT generation FROM coordination_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        i64::MAX
    );
}
#[test]
fn ordinary_capacity_preserves_exact_retry_and_one_final_revocation_per_record() {
    let (mut s, g, p, a) = setup();
    let op = register(a);
    s.control_peer(g, &p, &op, || Ok(1000)).unwrap();
    let mut addresses = vec![a];
    for _ in 1..control::MAX_RECORDS {
        let address = Address {
            installation_id: a.installation_id,
            session_id: Uuid::new_v4(),
        };
        s.control_peer(g, &p, &register(address), || Ok(1000))
            .unwrap();
        addresses.push(address);
    }
    for i in control::MAX_RECORDS..control::MAX_ORDINARY_RECEIPTS {
        s.db.execute(
            "INSERT INTO coordination_receipts VALUES(?1,X'00','{}','operator')",
            [format!("reserved-{i}")],
        )
        .unwrap();
    }
    let mut request = Mutation {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address: a,
        expected_revision: 1,
        operation: MutationKind::Configure {
            coordinator: p.machine,
            participants: vec![p.machine],
        },
    };
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Capacity
    );
    assert_eq!(s.control_inspect(g, a, || Ok(1000)).unwrap().0.revision, 1);
    for address in addresses {
        request.command_id = Uuid::new_v4();
        request.address = address;
        request.operation = MutationKind::Revoke {};
        assert_eq!(
            s.control_mutate(g, &request, || Ok(1000))
                .unwrap()
                .record
                .status,
            Status::Revoked
        );
    }
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM coordination_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        control::MAX_RECEIPTS as i64
    );
    assert!(matches!(
        s.control_peer(g, &p, &op, || Err(EnrollmentError::Invalid))
            .unwrap(),
        control::Reply::Registered {
            duplicate: true,
            ..
        }
    ));
    assert!(
        s.control_mutate(g, &request, || Err(EnrollmentError::Invalid))
            .unwrap()
            .duplicate
    );
}

#[test]
fn receipt_observation_requires_deadline_and_serializes_against_late_admission() {
    let (mut s, g, p, a) = setup();
    let operation = register(a);
    let ClientOperation::Register {
        command_id,
        expires_at_ms,
        ..
    } = operation
    else {
        unreachable!()
    };
    let observe = ClientOperation::ObserveRegistration {
        command_id,
        expires_at_ms,
        address: a,
    };
    assert_eq!(
        s.control_peer(g, &p, &observe, || Ok(1999)).unwrap_err(),
        EnrollmentError::Conflict
    );
    assert!(matches!(
        s.control_peer(g, &p, &observe, || Ok(2000)).unwrap(),
        control::Reply::NotRegistered {
            observed_at_ms: 2000,
            ..
        }
    ));
    assert!(s.control_peer(g, &p, &operation, || Ok(1999)).is_err());
    assert!(s.control_peer(g, &p, &operation, || Ok(2000)).is_err());
    let newer = ClientOperation::Register {
        command_id: Uuid::new_v4(),
        expires_at_ms: 3000,
        address: a,
        source_revision: 7,
    };
    s.control_peer(g, &p, &newer, || Ok(2000)).unwrap();
    let ClientOperation::Register { command_id, .. } = newer else {
        unreachable!()
    };
    let observed = s
        .control_peer(
            g,
            &p,
            &ClientOperation::ObserveRegistration {
                command_id,
                expires_at_ms: 3000,
                address: a,
            },
            || Err(EnrollmentError::Invalid),
        )
        .unwrap();
    assert!(matches!(
        observed,
        control::Reply::Registered {
            record: Record {
                source_revision: 7,
                ..
            },
            duplicate: true
        }
    ));
}

#[test]
fn malformed_refresh_never_replaces_corrupt_lease_and_receipt_failure_rolls_back() {
    let (mut s, g, p, a) = setup();
    s.control_peer(g, &p, &register(a), || Ok(1000)).unwrap();
    s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap();
    s.db.execute(
        "UPDATE coordination_leases SET lease=?1",
        ["X".repeat(4097)],
    )
    .unwrap();
    assert_eq!(
        s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1001))
            .unwrap_err(),
        EnrollmentError::Storage
    );
    assert_eq!(
        s.db.query_row("SELECT length(lease) FROM coordination_leases", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        4097
    );
    s.db.execute_batch("CREATE TRIGGER reject_receipt BEFORE INSERT ON coordination_receipts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let request = Mutation {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address: a,
        expected_revision: 1,
        operation: MutationKind::Revoke {},
    };
    assert!(s.control_mutate(g, &request, || Ok(1001)).is_err());
    assert_eq!(load(&s.db, a).unwrap().revision, 1);
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM coordination_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn explicit_source_rebinding_preserves_address_receipts_and_requires_separate_role_configuration() {
    let (mut s, g, mut p, a) = setup();
    let original_peer = p.machine;
    let registration = register(a);
    s.control_peer(g, &p, &registration, || Ok(1000)).unwrap();
    s.control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap();
    let fresh = control::Machine {
        machine_id: Uuid::new_v4(),
        epoch: 1,
    };
    s.db.execute(
        "INSERT INTO machines VALUES(?1,?2,1,0)",
        params![fresh.machine_id.to_string(), vec![2u8; 32]],
    )
    .unwrap();
    p.machine = fresh;
    let ClientOperation::Register {
        command_id,
        expires_at_ms,
        ..
    } = registration
    else {
        unreachable!()
    };
    let observation = ClientOperation::ObserveRegistration {
        command_id,
        expires_at_ms,
        address: a,
    };
    assert_eq!(
        s.control_peer(g, &p, &observation, || Ok(1000))
            .unwrap_err(),
        EnrollmentError::Denied
    );
    let mut request = Mutation {
        command_id: Uuid::new_v4(),
        expires_at_ms: 2000,
        address: a,
        expected_revision: 1,
        operation: MutationKind::RebindSource {
            expected_source: fresh,
            source: original_peer,
        },
    };
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Conflict
    );
    request.operation = MutationKind::RebindSource {
        expected_source: original_peer,
        source: fresh,
    };
    s.db.execute(
        "UPDATE machines SET revoked=1 WHERE id=?1",
        [fresh.machine_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Denied
    );
    s.db.execute(
        "UPDATE machines SET revoked=0 WHERE id=?1",
        [fresh.machine_id.to_string()],
    )
    .unwrap();
    let receipt = s.control_mutate(g, &request, || Ok(1000)).unwrap();
    assert_eq!(receipt.record.address, a);
    assert_eq!(receipt.record.source, fresh);
    assert_eq!(receipt.record.coordinator, original_peer);
    assert_eq!(receipt.record.participants, vec![original_peer]);
    assert!(s.control_inspect(g, a, || Ok(1000)).unwrap().1.is_empty());
    assert!(
        s.control_mutate(g, &request, || Err(EnrollmentError::Invalid))
            .unwrap()
            .duplicate
    );
    let control::Reply::Registered { record, .. } =
        s.control_peer(g, &p, &observation, || Ok(1000)).unwrap()
    else {
        panic!()
    };
    assert_eq!(record.source, original_peer);
    assert_eq!(record.revision, 1);
    let control::Reply::Snapshot { views } = s
        .control_peer(g, &p, &ClientOperation::Refresh {}, || Ok(1000))
        .unwrap()
    else {
        panic!()
    };
    assert!(views[0].lease.is_none());
    request.command_id = Uuid::new_v4();
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Conflict
    );
    request.expected_revision = 2;
    request.operation = MutationKind::Revoke {};
    s.control_mutate(g, &request, || Ok(1000)).unwrap();
    request.command_id = Uuid::new_v4();
    request.expected_revision = 3;
    request.operation = MutationKind::RebindSource {
        expected_source: fresh,
        source: original_peer,
    };
    assert_eq!(
        s.control_mutate(g, &request, || Ok(1000)).unwrap_err(),
        EnrollmentError::Conflict
    );
}
