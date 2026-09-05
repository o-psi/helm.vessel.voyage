use serde_json::{Value, json};
use voyage_protocol::stream::Frame;
const ID: &str = "00000000-0000-4000-8000-000000000001";
fn event(cursor: u64) -> Value {
    json!({"cursor":cursor,"run_id":ID,"event":{"type":"running"}})
}
fn replay() -> Value {
    json!({"type":"replay","connection_id":ID,"request_id":ID,"session_id":ID,"after":0,"latest":2,"events":[event(1),event(2)]})
}
fn valid(v: &Value) -> bool {
    Frame::decode(&serde_json::to_vec(v).unwrap()).is_ok()
}
#[test]
fn replay_wire_and_gap_contract() {
    let v = replay();
    assert!(valid(&v));
    let decoded = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&decoded.encode().unwrap()).unwrap(),
        v
    );
    for cursors in [vec![2], vec![1, 3], vec![1, 1], vec![]] {
        let mut bad = v.clone();
        bad["events"] = json!(cursors.into_iter().map(event).collect::<Vec<_>>());
        assert!(!valid(&bad));
    }
    let mut page = v.clone();
    page["events"] = json!([event(1)]);
    assert!(valid(&page));
}
#[test]
fn event_variants_and_explicit_snapshot() {
    for payload in [
        json!({"type":"accepted","command_id":ID,"revision":0}),
        json!({"type":"running"}),
        json!({"type":"canonical_checkpoint","revision":1}),
        json!({"type":"text_delta","text":"hi 🦀"}),
        json!({"type":"tool_started","tool_call_id":ID,"name":"read"}),
        json!({"type":"tool_finished","tool_call_id":ID,"outcome":"failed"}),
        json!({"type":"usage","input_tokens":2,"output_tokens":3}),
        json!({"type":"cancellation_requested"}),
        json!({"type":"terminal","state":"incomplete"}),
    ] {
        let v = json!({"type":"event","connection_id":ID,"session_id":ID,"event":{"cursor":1,"run_id":ID,"event":payload}});
        assert!(valid(&v), "{v}");
    }
    let mut v = json!({"type":"snapshot_required","connection_id":ID,"request_id":ID,"session_id":ID,"after":1,"latest":9});
    assert!(valid(&v));
    v["after"] = json!(10);
    assert!(!valid(&v));
}

use uuid::Uuid;
use voyage_protocol::events::{
    EventCursor, EventSequence, Feature, Features, Observation, SequencedEvent,
};
fn cursor(n: u64) -> EventCursor {
    EventCursor::new(n).unwrap()
}
fn typed(n: u64) -> SequencedEvent {
    serde_json::from_value(event(n)).unwrap()
}
fn features() -> Features {
    Features::new(vec![
        Feature::SequencedEvents,
        Feature::Replay,
        Feature::ToolActivity,
        Feature::Usage,
    ])
    .unwrap()
}
#[test]
fn negotiation_and_legacy_omission() {
    let basic = Features::default();
    let all = features();
    assert!(all.negotiate(&basic).unwrap().is_empty());
    assert!(basic.confirm(&all).is_err());
    assert!(all.confirm(&basic).is_ok());
    assert!(Features::new(vec![Feature::Replay]).is_err());
    assert!(Features::new(vec![Feature::SequencedEvents, Feature::SequencedEvents]).is_err());
    let v = json!({"type":"welcome","version":2,"connection_id":ID,"machine_id":ID,"owner_id":ID,"epoch":1,"lease_ms":1000});
    let f = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&f.encode().unwrap()).unwrap(),
        v
    );
    for fs in [
        json!(["unknown"]),
        json!(["replay"]),
        json!(["sequenced_events", "sequenced_events"]),
    ] {
        let mut bad = v.clone();
        bad["features"] = fs;
        assert!(!valid(&bad));
    }
    let frame = Frame::decode(&serde_json::to_vec(&replay()).unwrap()).unwrap();
    assert!(frame.validate_features(&basic).is_err());
    assert!(frame.validate_features(&all).is_ok());
    // A negotiated wire feature never manufactures an execution Capability.
    assert!(
        serde_json::from_value::<voyage_protocol::attachment::Capability>(json!(
            "sequenced_events"
        ))
        .is_err()
    );
}
#[test]
fn receiver_scope_duplicates_and_atomic_gap_rejection() {
    let id = Uuid::parse_str(ID).unwrap();
    let other = Uuid::new_v4();
    let mut seq = EventSequence::new(id, id, cursor(0), features()).unwrap();
    assert!(seq.observe(other, id, typed(1)).is_err());
    assert!(seq.observe(id, other, typed(1)).is_err());
    assert_eq!(seq.cursor(), cursor(0));
    assert_eq!(seq.observe(id, id, typed(1)).unwrap(), Observation::Applied);
    assert_eq!(
        seq.observe(id, id, typed(1)).unwrap(),
        Observation::Duplicate
    );
    let mut conflict = typed(1);
    conflict.event = serde_json::from_value(json!({"type":"cancellation_requested"})).unwrap();
    assert!(seq.observe(id, id, conflict).is_err());
    assert!(
        seq.replay(id, id, cursor(1), cursor(4), &[typed(2), typed(4)])
            .is_err()
    );
    assert_eq!(seq.cursor(), cursor(1));
    seq.replay(id, id, cursor(1), cursor(4), &[typed(2), typed(3)])
        .unwrap();
    assert_eq!(seq.cursor(), cursor(3));
    assert!(seq.observe(id, id, typed(1)).is_err());
    let mut terminal = typed(4);
    terminal.event =
        serde_json::from_value(json!({"type":"terminal","state":"cancelled"})).unwrap();
    assert_eq!(
        seq.observe(id, id, terminal.clone()).unwrap(),
        Observation::Applied
    );
    assert_eq!(
        seq.observe(id, id, terminal).unwrap(),
        Observation::Duplicate
    );
    assert!(
        seq.replay(id, id, cursor(3), cursor(4), &[typed(4)])
            .is_err()
    );
}
#[test]
fn strict_event_limits_and_projection_boundaries() {
    let base = json!({"type":"event","connection_id":ID,"session_id":ID,"event":event(1)});
    for (path, value) in [
        ("cursor", json!(0)),
        ("cursor", json!(u64::MAX)),
        ("run_id", json!(Uuid::nil())),
    ] {
        let mut v = base.clone();
        v["event"][path] = value;
        assert!(!valid(&v));
    }
    for payload in [
        json!({"type":"terminal","state":"running"}),
        json!({"type":"text_delta","text":""}),
        json!({"type":"text_delta","text":"\u{0}"}),
        json!({"type":"text_delta","text":"x".repeat(65537)}),
        json!({"type":"running","session":{}}),
        json!({"type":"provider_state"}),
        json!({"type":"pty","bytes":"secret"}),
        json!({"type":"usage","input_tokens":u64::MAX,"output_tokens":1}),
    ] {
        let mut v = base.clone();
        v["event"]["event"] = payload;
        assert!(!valid(&v));
    }
    let mut v = base.clone();
    v["event"]["event"] = json!({"type":"text_delta","text":"x".repeat(65536)});
    assert!(valid(&v));
    v["event"]["event"] = json!({"type":"text_delta","text":"\u{1}".repeat(65536)});
    assert!(!valid(&v));
    let mut r = replay();
    r["latest"] = json!(128);
    r["events"] = json!((1..=128).map(event).collect::<Vec<_>>());
    assert!(valid(&r));
    r["latest"] = json!(129);
    r["events"] = json!((1..=129).map(event).collect::<Vec<_>>());
    assert!(!valid(&r));
    let mut request = json!({"type":"replay_request","connection_id":ID,"session_id":ID,"request_id":ID,"after":0,"limit":128});
    assert!(valid(&request));
    for n in [0, 129] {
        request["limit"] = json!(n);
        assert!(!valid(&request));
    }
}
#[test]
fn receiver_rejects_unnegotiated_tail_without_partial_application() {
    let id = Uuid::parse_str(ID).unwrap();
    let mut seq = EventSequence::new(
        id,
        id,
        cursor(0),
        Features::new(vec![Feature::SequencedEvents, Feature::Replay]).unwrap(),
    )
    .unwrap();
    let mut usage = typed(2);
    usage.event =
        serde_json::from_value(json!({"type":"usage","input_tokens":1,"output_tokens":2})).unwrap();
    assert!(
        seq.replay(id, id, cursor(0), cursor(2), &[typed(1), usage])
            .is_err()
    );
    assert_eq!(seq.cursor(), cursor(0));
}
#[test]
fn golden_event_and_duplicate_field_rejection() {
    let wire = format!(
        r#"{{"type":"event","connection_id":"{ID}","session_id":"{ID}","event":{{"cursor":1,"run_id":"{ID}","event":{{"type":"running"}}}}}}"#
    );
    assert_eq!(
        Frame::decode(wire.as_bytes()).unwrap().encode().unwrap(),
        wire
    );
    let duplicate = wire.replace("\"cursor\":1", "\"cursor\":1,\"cursor\":2");
    assert!(Frame::decode(duplicate.as_bytes()).is_err());
    let unknown = wire.replace(
        "\"type\":\"running\"",
        "\"type\":\"running\",\"system\":\"secret\"",
    );
    assert!(Frame::decode(unknown.as_bytes()).is_err());
    for kind in ["event", "replay_request", "replay", "snapshot_required"] {
        let v = match kind {
            "event" => serde_json::from_str::<Value>(&wire).unwrap(),
            "replay" => replay(),
            "replay_request" => {
                json!({"type":kind,"connection_id":ID,"request_id":ID,"session_id":ID,"after":0,"limit":1})
            }
            _ => {
                json!({"type":kind,"connection_id":ID,"request_id":ID,"session_id":ID,"after":0,"latest":1})
            }
        };
        let f = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(f.validate_features(&Features::default()).is_err());
        assert!(f.validate_features(&features()).is_ok());
        assert!(!format!("{f:?}").contains(ID));
    }
}
#[test]
fn filtered_global_sequences_cannot_silently_advance() {
    assert!(serde_json::from_value::<EventCursor>(json!(u64::MAX)).is_err());
    let id = Uuid::parse_str(ID).unwrap();
    let mut seq = EventSequence::new(
        id,
        id,
        cursor(0),
        Features::new(vec![Feature::SequencedEvents, Feature::Replay]).unwrap(),
    )
    .unwrap();
    seq.observe(id, id, typed(1)).unwrap();
    // Cursor 2 is a private or unsupported tool record. Sending it is forbidden;
    // omitting it must not make cursor 3 look like a valid contiguous projection.
    assert!(seq.observe(id, id, typed(3)).is_err());
    assert!(
        seq.replay(id, id, cursor(1), cursor(3), &[typed(3)])
            .is_err()
    );
    assert_eq!(seq.cursor(), cursor(1));
    // Decoding an authorized retention-gap signal does not mutate observations.
    let v = json!({"type":"snapshot_required","connection_id":ID,"request_id":ID,"session_id":ID,"after":1,"latest":3});
    assert!(valid(&v));
    assert_eq!(seq.cursor(), cursor(1));
}
#[test]
fn accepted_reply_rejects_unexpected_projection_fields() {
    let mut v =
        json!({"type":"result","connection_id":ID,"command_id":ID,"reply":{"type":"accepted"}});
    let clean = Frame::decode(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&clean.encode().unwrap()).unwrap(),
        v
    );
    v["reply"]["unexpected_secret_field"] = json!("must not be accepted");
    assert!(!valid(&v));
}
