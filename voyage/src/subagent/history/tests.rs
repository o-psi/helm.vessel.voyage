use super::*;
fn event() -> SubagentEvent {
    SubagentEvent {
        sequence: 0,
        timestamp: chrono::Utc::now(),
        agent_id: crate::subagent::AgentId::new(),
        kind: crate::subagent::SubagentEventKind::Queued,
    }
}
#[test]
fn replay_reports_eviction_epoch_changes_and_preserves_corrupt_evidence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("events.json");
    let mut history = EventHistory::open(None, 2, false);
    let first = history.push(event(), 2);
    let cursor = history.replay(None).status.cursor;
    history.push(event(), 2);
    history.push(event(), 2);
    assert_eq!(history.events.len(), 2);
    assert_eq!(history.evicted_through, first.sequence);
    assert!(!history.replay(Some(cursor)).status.cursor_gap);
    let mut old = cursor;
    old.sequence = 0;
    assert!(history.replay(Some(old)).status.cursor_gap);
    old.epoch = Uuid::new_v4();
    assert!(history.replay(Some(old)).status.cursor_gap);
    history.persist(&path);
    assert!(history.replay(None).status.durable);
    let restored = EventHistory::open(Some(&path), 2, true);
    assert_eq!(restored.events.len(), 2);
    assert_eq!(restored.replay(None).status.cursor.sequence, 3);
    std::fs::write(&path, b"corrupt retained evidence").unwrap();
    let mut broken = EventHistory::open(Some(&path), 2, true);
    assert!(
        broken
            .replay(None)
            .status
            .notices
            .contains(&HistoryNotice::Corrupt)
    );
    broken.persist(&path);
    assert_eq!(std::fs::read(path).unwrap(), b"corrupt retained evidence");
}
#[test]
fn sequence_overflow_rotates_epoch_and_rejects_invalid_serialized_history() {
    let mut history = EventHistory::open(None, 2, true);
    let old = history.epoch;
    history.sequence = u64::MAX;
    history.push(event(), 2);
    assert_ne!(history.epoch, old);
    assert_eq!(history.sequence, 1);
    assert!(history.notices.contains(&HistoryNotice::Corrupt));
    history.validate().unwrap();
    history.version = 99;
    assert!(history.validate().is_err());
    history.version = 1;
    history.epoch = Uuid::nil();
    assert!(history.validate().is_err());
    history.write_failed();
    assert!(!history.durable);
    assert!(history.notices.contains(&HistoryNotice::WriteFailed));
}
