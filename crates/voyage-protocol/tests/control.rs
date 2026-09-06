//! Coordination-control wire regressions are independent of execution capability.
use voyage_protocol::{
    events::{Feature, Features},
    stream::Frame,
};
const ID: &str = "11111111-1111-4111-8111-111111111111";
fn request() -> serde_json::Value {
    serde_json::json!({"type":"control_request","connection_id":ID,"request_id":ID,
        "operation":{"type":"register","command_id":ID,"expires_at_ms":1234,
        "address":{"installation_id":ID,"session_id":ID},"source_revision":0}})
}
#[test]
fn control_registration_is_strict_and_requires_explicit_negotiation() {
    let value = request();
    let frame = Frame::decode(&serde_json::to_vec(&value).unwrap())
        .expect("control registration must decode");
    assert!(frame.validate_features(&Features::default()).is_err());
    assert!(
        frame
            .validate_features(
                &Features::new(vec![Feature::SequencedEvents, Feature::ManagedExecution]).unwrap()
            )
            .is_err()
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&frame.encode().unwrap()).unwrap(),
        value
    );
    for path in ["outer", "operation", "address"] {
        let mut bad = request();
        match path {
            "outer" => bad["secret"] = "canary".into(),
            "operation" => bad["operation"]["secret"] = "canary".into(),
            _ => bad["operation"]["address"]["secret"] = "canary".into(),
        };
        assert!(Frame::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
}
#[test]
fn control_wire_rejects_nil_and_unbounded_values() {
    for field in ["connection_id", "request_id"] {
        let mut bad = request();
        bad[field] = "00000000-0000-0000-0000-000000000000".into();
        assert!(Frame::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    for field in ["installation_id", "session_id"] {
        let mut bad = request();
        bad["operation"]["address"][field] = "00000000-0000-0000-0000-000000000000".into();
        assert!(Frame::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let mut bad = request();
    bad["operation"]["source_revision"] = u64::MAX.into();
    assert!(Frame::decode(&serde_json::to_vec(&bad).unwrap()).is_err());
}

#[test]
fn control_only_negotiation_and_maximum_snapshot_fit_the_existing_frame_bound() {
    use uuid::Uuid;
    use voyage_protocol::control::*;
    let features = Features::new(vec![Feature::CoordinationControl]).unwrap();
    Frame::decode(&serde_json::to_vec(&request()).unwrap())
        .unwrap()
        .validate_features(&features)
        .unwrap();
    let machines = (0..MAX_PARTICIPANTS)
        .map(|_| Machine {
            machine_id: Uuid::new_v4(),
            epoch: i64::MAX as u64,
        })
        .collect::<Vec<_>>();
    let views = (0..MAX_RECORDS)
        .map(|_| {
            let record = Record {
                address: Address {
                    installation_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4(),
                },
                source: machines[0],
                source_revision: i64::MAX as u64,
                revision: i64::MAX as u64,
                scope_revision: i64::MAX as u64,
                coordinator_epoch: i64::MAX as u64,
                coordinator: machines[0],
                participants: machines.clone(),
                status: Status::Configured,
                execution_authority: ExecutionAuthority::None,
            };
            View {
                record,
                lease: Some(Lease {
                    lease_id: Uuid::new_v4(),
                    server_generation: i64::MAX as u64,
                    connection_id: Uuid::new_v4(),
                    machine: machines[0],
                    record_revision: i64::MAX as u64,
                    coordinator: true,
                    participant: true,
                    expires_at_ms: i64::MAX,
                }),
            }
        })
        .collect();
    let frame = Frame::ControlResult {
        connection_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        reply: Reply::Snapshot { views },
    };
    let encoded = frame.encode().unwrap();
    assert!(encoded.len() <= voyage_protocol::attachment::MAX_FRAME_BYTES);
    let decoded = Frame::decode(encoded.as_bytes()).unwrap();
    decoded.validate_features(&features).unwrap();
    let Frame::ControlResult {
        reply: Reply::Snapshot { mut views },
        ..
    } = decoded
    else {
        unreachable!()
    };
    views[0].lease.as_mut().unwrap().coordinator = false;
    assert!(
        Reply::Snapshot {
            views: views.clone()
        }
        .validate()
        .is_err()
    );
    views[0].lease = None;
    views.push(views[0].clone());
    assert!(Reply::Snapshot { views }.validate().is_err());
}

#[test]
fn receipt_absence_and_scope_are_strict_not_execution_authority() {
    use uuid::Uuid;
    use voyage_protocol::control::*;
    let address = Address {
        installation_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
    };
    assert!(
        Reply::NotRegistered {
            command_id: Uuid::new_v4(),
            address,
            expires_at_ms: 3,
            observed_at_ms: 2
        }
        .validate()
        .is_err()
    );
    assert!(serde_json::from_str::<ExecutionAuthority>("\"available\"").is_err());
    let m = Machine {
        machine_id: Uuid::new_v4(),
        epoch: 1,
    };
    for participants in [
        vec![],
        vec![m, m],
        vec![Machine {
            machine_id: m.machine_id,
            epoch: 2,
        }],
    ] {
        let mutation = Mutation {
            command_id: Uuid::new_v4(),
            expires_at_ms: 1,
            address,
            expected_revision: 1,
            operation: MutationKind::Configure {
                coordinator: m,
                participants,
            },
        };
        assert!(mutation.validate().is_err());
    }
    let mut value = request();
    value["operation"] = serde_json::json!({"type":"assignment","prompt":"canary"});
    assert!(Frame::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let duplicated = format!(
        "{{\"type\":\"control_request\",\"connection_id\":\"{ID}\",\"connection_id\":\"{ID}\",\"request_id\":\"{ID}\",\"operation\":{{\"type\":\"refresh\"}}}}"
    );
    assert!(Frame::decode(duplicated.as_bytes()).is_err());
}
