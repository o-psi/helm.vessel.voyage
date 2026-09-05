# Attachment foundations — implementation status

Refs [#9](https://github.com/o-psi/voyage/issues/9) and
[#78](https://github.com/o-psi/voyage/issues/78) under the
[full production contract](attachment-production-contract.md).

**These are library foundations, not a working `helm attach` system.** No new network
endpoint, invitation command, service or remote execution path is enabled. The
full requested scope remains unfinished; these tests do not establish production
readiness or safe remote access to existing local sessions.

## Command domain

`crates/voyage-protocol/src/attachment.rs` defines version-2 command envelopes and
strict decoding. Commands carry connection, installation, principal and command
IDs, a deadline, and typed operations for session management, sharing and approval
decisions. Confirmations and digests are identifiers, not authorization grants.

Decoding rejects unknown/duplicate fields, unsupported versions, nil identities,
invalid revisions, oversized inputs and expired commands with content-free errors.
Names/model labels are bounded and reject control/bidi-format characters; prompts
are bounded separately. Canonical JSON must fit the frame budget even after string
escaping. Admission identity omits renewable connection IDs but retains authority,
payload and expiry. A transport must authenticate those claimed identities; parsing
JSON does not authenticate anyone. Event envelopes, negotiation and adapters remain
unfinished. Expired duplicate-outcome lookup must be integrated separately from
new-command deadline validation.

## Atomic command/run journal

`helm/src/attachment/journal.rs` owns a dedicated `journal.sqlite3` within an explicitly
provided private directory. It does not open the existing SessionStore, legacy Helm
enrollment file or Vessel database. Import is explicit and create-only. No implicit
scan or publication of existing sessions occurs.

The implemented storage boundary includes:

- Create-only session snapshots, excluding ephemeral runtime system guidance.
- Atomic admission: accepted user turn, revision increment, command digest, run
  attribution and first event commit together, before acknowledgment or effects.
- Immutable command identity: identical retries retrieve the original run; changed
  authority/payload/deadline is rejected. Expired retries can inspect an existing
  journal outcome but cannot admit new work.
- Stable per-session OS execution locks across independent handles/processes.
  Unknown sessions cannot allocate lock files. Locks are never unlinked on release.
- Separate accepted and running states. Dispatch intent commits once before effects;
  no journal method itself executes tools. Duplicate dispatch is rejected.
- Durable partial text separate from accepted final assistant content. Failed,
  cancelled or interrupted outcomes cannot publish a successful final.
- Immutable terminal outcomes. Recovery requires acquired execution ownership,
  marks abandoned active runs interrupted, and does not replay tools. Interrupted
  terminal metadata becomes disconnected rather than falsely live.
- Monotonic bounded replay, with snapshot-required when a cursor is evicted or events
  are missing. Persisted record identity and lifecycle inconsistencies fail closed.
- Bounded SQLite busy behavior, full synchronous transactions, private Unix files
  and symlink rejection. Output and reason text are excluded from RunRecord Debug.

Current internal limits are explicit foundation constants, not user configuration:
256 MiB database, 4,096 sessions, 100,000 durable command records, 16 MiB serialized
session snapshot, 64 KiB prompt/delta, 1 MiB partial/final text and 1,024 replay events
per session. SQLite rollback journals can consume additional transient disk space.
Capacity failure rejects further writes; command evidence is not pruned to make room.
Replay eviction alone does not erase partial text or deduplication history. Product
retention/export/archival controls remain necessary for long-term operation.

## Integration requirements still open

- All local CLI/TUI and remote writers must use one coordinator. Today existing
  SessionStore JSON writes do not consult this journal or its execution locks.
- Canonical tool-call/result checkpoints, usage accounting, cancellation intent,
  live runtime ownership and cleanup must integrate with the agent loop. Text-only
  journal fixtures are not proof of complete provider/tool durability.
- Full session lifecycle mutations, privacy-safe projections, sharing/principal
  checks, credential epochs and authorization leases must guard every operation.
- Enrollment, key lifecycle, authenticated WebSocket transport, Vessel UI,
  services, opt-in remote approvals and secure notifications/handoff remain pending.
- Synchronous SQLite work must run off async reactor threads in the coordinator.
  Trusted local storage and cooperating processes are assumed; network filesystems
  and hostile processes with the same OS identity are not isolated by this API.
- Windows ACL verification, platform-specific durability, macOS/Windows tests,
  actual deployment/backup/revocation drills and reviewed provider evidence remain
  production gates. Unix permission tests cannot substitute for those checks.

## Tests

`cargo test -p helm attachment::journal` covers admission rollback, modified retries,
stale revisions, accepted-versus-running semantics, output limits, immutable outcomes,
restart, real subprocess lock exclusion and abrupt termination, replay gaps/eviction,
SQLite FULL/busy failures, corrupt identities/schema, revision overflow and Unix
private-path/symlink safety. These tests use temporary stores, not operator data.

`cargo test -p voyage-protocol` covers strict domain decoding, all defined operation
roundtrips, deadline/identity/revision/Unicode boundaries, duplicate/nested/unknown
fields, unsupported versions, deterministic reconnect identity and JSON-escape
expansion. No live network or provider is contacted by these tests.
