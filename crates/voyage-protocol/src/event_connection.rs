//! Event-only connection initialization fencing. Staged, not advertised until
//! both clients and the executing owner implement the complete contract.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const VERSION: u32 = 3;
pub const MAX_ENTITY_BYTES: usize = 32 * 1024;
pub const MAX_INITIALIZATION_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub generation: Uuid,
    pub session_id: Uuid,
    pub incarnation: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InitializationEvent {
    Begin {
        fence: Fence,
        cursor: u64,
    },
    Entity {
        fence: Fence,
        sequence: u64,
        entity_kind: EntityKind,
        entity_id: String,
        value: serde_json::Value,
    },
    Complete {
        fence: Fence,
        sequence: u64,
        cursor: u64,
    },
    Reset {
        fence: Fence,
        reason: ResetReason,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Session,
    Run,
    Message,
    Decision,
    CommandOutcome,
    Lifecycle,
    Catalogue,
    Goal,
    Settings,
    Account,
    Usage,
    Tool,
    Reasoning,
    Resource,
    Artifact,
    Browser,
    Terminal,
    Update,
    Notification,
    Workspace,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetReason {
    RetentionGap,
    IncarnationChanged,
    InitializationExpired,
    SlowConsumer,
}

/// Socket correlation and durable command identity have deliberately distinct
/// fields. Admission remains on the executing owner, never in a client reducer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub version: u32,
    pub correlation_id: Uuid,
    pub command_id: Uuid,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub expected_revision: u64,
    pub expires_at_ms: u64,
    pub operation: ControlOperation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlOperation {
    Cancel { run_id: Uuid },
    Rename { name: String },
    SetAccess { access: String },
}
impl CommandEnvelope {
    pub fn validate_identity(
        &self,
        session_id: Uuid,
        incarnation: Uuid,
        now_ms: u64,
    ) -> Result<(), &'static str> {
        if self.version != VERSION {
            return Err("incompatible event connection: upgrade both peers");
        }
        if self.session_id != session_id || self.incarnation != incarnation {
            return Err("stale canonical owner identity");
        }
        if self.expires_at_ms <= now_ms {
            return Err("command expired; observe retained outcome before replacement");
        }
        if self.command_id.is_nil() || self.correlation_id.is_nil() {
            return Err("missing command identity");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandOutcome {
    Accepted {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    },
    Refused {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
        code: String,
    },
    Unknown {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    },
    Completed {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
        revision: u64,
    },
}

/// Bounded UTF-8 content transfer scoped to the initialization generation.
/// Offsets are bytes; clients never guess concatenation after an offset mismatch.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentChunk {
    pub fence: Fence,
    pub entity_id: String,
    pub offset: u64,
    pub total_bytes: u64,
    pub text: String,
}
impl ContentChunk {
    pub fn validate(&self, expected: &Fence, next_offset: u64) -> Result<u64, &'static str> {
        if &self.fence != expected || self.offset != next_offset {
            return Err("content identity or offset mismatch");
        }
        if self.entity_id.is_empty()
            || self.entity_id.len() > 256
            || self.text.len() > MAX_ENTITY_BYTES
        {
            return Err("invalid bounded content chunk");
        }
        let end = self
            .offset
            .checked_add(self.text.len() as u64)
            .ok_or("content offset overflow")?;
        if end > self.total_bytes || (self.text.is_empty() && end != self.total_bytes) {
            return Err("invalid content extent");
        }
        Ok(end)
    }
}

/// Validates a bounded initialization stream before a client publishes staged
/// entities. Duplicate delivery is accepted only by the transport's retained
/// exact-frame deduplication; this fence never silently ignores reordered data.
#[derive(Default, Debug)]
pub struct InitializationFence {
    active: Option<(Fence, u64, u64, usize)>,
}
impl InitializationFence {
    pub fn accept(&mut self, event: &InitializationEvent) -> Result<bool, &'static str> {
        match event {
            InitializationEvent::Begin { fence, cursor } => {
                self.active = Some((fence.clone(), *cursor, 0, 0));
                Ok(false)
            }
            InitializationEvent::Entity {
                fence,
                sequence,
                entity_id,
                value,
                ..
            } => {
                let Some((current, _, next, total)) = self.active.as_mut() else {
                    return Err("initialization not started");
                };
                if current != fence || sequence != next {
                    return Err("initialization fence or sequence mismatch");
                }
                if entity_id.is_empty() || entity_id.len() > 256 {
                    return Err("invalid entity identity");
                }
                let bytes = serde_json::to_vec(value).map_err(|_| "invalid entity value")?;
                if bytes.len() > MAX_ENTITY_BYTES {
                    return Err("entity requires bounded chunks");
                }
                let new_total = total
                    .checked_add(bytes.len())
                    .ok_or("initialization byte limit exceeded")?;
                if new_total > MAX_INITIALIZATION_BYTES {
                    return Err("initialization requires paged scopes");
                }
                *total = new_total;
                *next = next
                    .checked_add(1)
                    .ok_or("initialization sequence exhausted")?;
                Ok(false)
            }
            InitializationEvent::Complete {
                fence,
                sequence,
                cursor,
            } => {
                let Some((current, initial, next, _)) = self.active.as_ref() else {
                    return Err("initialization not started");
                };
                if current != fence || sequence != next || cursor != initial {
                    return Err("initialization barrier mismatch");
                }
                self.active = None;
                Ok(true)
            }
            InitializationEvent::Reset { fence, .. } => {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|(current, _, _, _)| current != fence)
                {
                    return Err("reset fence mismatch");
                }
                self.active = None;
                Ok(false)
            }
        }
    }
}

/// Shared entity reducer used by clients before publication. This stores a
/// selected bounded scope, never constructs a monolithic session snapshot.
#[derive(Default, Debug)]
pub struct EntityReducer {
    gate: InitializationFence,
    entities: std::collections::BTreeMap<String, serde_json::Value>,
}
#[derive(Debug)]
pub struct InitializedScope {
    pub fence: Fence,
    pub cursor: u64,
    pub entities: std::collections::BTreeMap<String, serde_json::Value>,
}
impl EntityReducer {
    pub fn accept(
        &mut self,
        event: &InitializationEvent,
    ) -> Result<Option<InitializedScope>, &'static str> {
        let complete = self.gate.accept(event)?;
        match event {
            InitializationEvent::Begin { .. } | InitializationEvent::Reset { .. } => {
                self.entities.clear();
            }
            InitializationEvent::Entity {
                entity_kind,
                entity_id,
                value,
                ..
            } => {
                let kind = serde_json::to_value(entity_kind).map_err(|_| "invalid entity kind")?;
                let kind = kind.as_str().ok_or("invalid entity kind")?;
                self.entities
                    .insert(format!("{kind}:{entity_id}"), value.clone());
            }
            InitializationEvent::Complete { fence, cursor, .. } if complete => {
                return Ok(Some(InitializedScope {
                    fence: fence.clone(),
                    cursor: *cursor,
                    entities: std::mem::take(&mut self.entities),
                }));
            }
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fence() -> Fence {
        Fence {
            generation: Uuid::nil(),
            session_id: Uuid::nil(),
            incarnation: Uuid::nil(),
        }
    }
    #[test]
    fn shared_reducer_never_publishes_before_barrier() {
        let mut reducer = EntityReducer::default();
        assert!(
            reducer
                .accept(&InitializationEvent::Begin {
                    fence: fence(),
                    cursor: 40
                })
                .unwrap()
                .is_none()
        );
        assert!(
            reducer
                .accept(&InitializationEvent::Entity {
                    fence: fence(),
                    sequence: 0,
                    entity_kind: EntityKind::Message,
                    entity_id: "message:0".into(),
                    value: serde_json::json!({"text":"é"})
                })
                .unwrap()
                .is_none()
        );
        let scope = reducer
            .accept(&InitializationEvent::Complete {
                fence: fence(),
                sequence: 1,
                cursor: 40,
            })
            .unwrap()
            .unwrap();
        assert_eq!(scope.cursor, 40);
        assert_eq!(scope.entities["message:message:0"]["text"], "é");
        assert!(reducer.entities.is_empty());
    }
    #[test]
    fn initialization_scope_memory_and_restart_are_bounded() {
        let mut state = InitializationFence::default();
        state
            .accept(&InitializationEvent::Begin {
                fence: fence(),
                cursor: 40,
            })
            .unwrap();
        let mut event = InitializationEvent::Entity {
            fence: fence(),
            sequence: 0,
            entity_kind: EntityKind::Message,
            entity_id: "m".into(),
            value: serde_json::json!("x".repeat(MAX_ENTITY_BYTES - 2)),
        };
        for sequence in 0..256 {
            if let InitializationEvent::Entity {
                sequence: current, ..
            } = &mut event
            {
                *current = sequence;
            }
            state.accept(&event).unwrap();
        }
        if let InitializationEvent::Entity { sequence, .. } = &mut event {
            *sequence = 256;
        }
        assert!(state.accept(&event).is_err());
        let mut new_fence = fence();
        new_fence.generation = Uuid::new_v4();
        state
            .accept(&InitializationEvent::Begin {
                fence: new_fence.clone(),
                cursor: 90,
            })
            .unwrap();
        assert!(state.accept(&event).is_err());
        assert!(
            state
                .accept(&InitializationEvent::Complete {
                    fence: new_fence,
                    sequence: 0,
                    cursor: 90
                })
                .unwrap()
        );
    }
    #[test]
    fn commands_refuse_stale_owners_and_incompatible_peers() {
        let session = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let mut command = CommandEnvelope {
            version: VERSION,
            correlation_id: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            session_id: session,
            incarnation: owner,
            expected_revision: 9,
            expires_at_ms: 100,
            operation: ControlOperation::Cancel {
                run_id: Uuid::new_v4(),
            },
        };
        assert!(command.validate_identity(session, owner, 99).is_ok());
        assert!(command.validate_identity(session, owner, 100).is_err());
        assert!(
            command
                .validate_identity(session, Uuid::new_v4(), 99)
                .is_err()
        );
        command.version = 1;
        assert!(command.validate_identity(session, owner, 99).is_err());
    }
    #[test]
    fn content_offsets_are_utf8_bytes_and_fenced() {
        let chunk = ContentChunk {
            fence: fence(),
            entity_id: "message:0".into(),
            offset: 0,
            total_bytes: 2,
            text: "é".into(),
        };
        assert_eq!(chunk.validate(&fence(), 0).unwrap(), 2);
        assert!(chunk.validate(&fence(), 1).is_err());
        let mut wrong = fence();
        wrong.incarnation = Uuid::new_v4();
        assert!(chunk.validate(&wrong, 0).is_err());
        let empty = ContentChunk {
            text: String::new(),
            ..chunk
        };
        assert!(empty.validate(&fence(), 0).is_err());
    }
    #[test]
    fn barrier_requires_exact_generation_sequence_and_cursor() {
        let mut state = InitializationFence::default();
        let complete = InitializationEvent::Complete {
            fence: fence(),
            sequence: 0,
            cursor: 40,
        };
        assert!(state.accept(&complete).is_err());
        state
            .accept(&InitializationEvent::Begin {
                fence: fence(),
                cursor: 40,
            })
            .unwrap();
        assert!(
            state
                .accept(&InitializationEvent::Complete {
                    fence: fence(),
                    sequence: 0,
                    cursor: 41
                })
                .is_err()
        );
        assert!(state.accept(&complete).unwrap());
        assert!(state.accept(&complete).is_err());
    }
    #[test]
    fn bounds_and_reordering_cannot_publish_partial_state() {
        let mut state = InitializationFence::default();
        state
            .accept(&InitializationEvent::Begin {
                fence: fence(),
                cursor: 40,
            })
            .unwrap();
        let mut entity = InitializationEvent::Entity {
            fence: fence(),
            sequence: 1,
            entity_kind: EntityKind::Message,
            entity_id: "message:0".into(),
            value: serde_json::json!({}),
        };
        assert!(state.accept(&entity).is_err());
        if let InitializationEvent::Entity {
            sequence, value, ..
        } = &mut entity
        {
            *sequence = 0;
            *value = serde_json::json!("x".repeat(MAX_ENTITY_BYTES));
        }
        assert!(state.accept(&entity).is_err());
        if let InitializationEvent::Entity { value, .. } = &mut entity {
            *value = serde_json::json!({"text":"é"});
        }
        assert!(!state.accept(&entity).unwrap());
        assert!(state.accept(&entity).is_err());
        assert!(
            state
                .accept(&InitializationEvent::Complete {
                    fence: fence(),
                    sequence: 1,
                    cursor: 40
                })
                .unwrap()
        );
    }
}
