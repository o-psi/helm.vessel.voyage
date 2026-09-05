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
JSON does not authenticate anyone. The compiled frame codec below carries these commands; sequenced event envelopes,
negotiation and adapters remain unfinished. Expired duplicate-outcome lookup must
be integrated separately from new-command deadline validation.

## Compiled stream codec (partial #9)

`voyage_protocol::stream` is now an exported, compiled module. Its public-consumer
fixtures exercise authenticate/welcome, command/result and heartbeat/lease frames.
This is a wire codec, not an authenticated connection or a working transport.
No websocket route, Helm listener, remote command handler or provider dispatch is
added. Typed event subscriptions, replay/snapshot delivery, capability negotiation,
backpressure, lease enforcement and two-component network tests remain required.

Both `Frame::decode` and `Frame::encode` reject invalid nested structures and
oversized canonical JSON. Decoding first limits incoming bytes to 256 KiB. Commands
reuse their identity, operation, label, revision and prompt bounds. Authentication
accepts only a structurally valid version-2 Connect proof: nonnil IDs, bounded epoch,
positive expiry, bounded origin, 32-byte server tag, 64-byte signature, no rotation
signature and at most 16 KiB of canonical proof JSON. These checks do not verify the
signature, origin, challenge consumption, current epoch or expiry against a clock.
The eventual adapter must perform those checks against trusted enrollment state.

Result projections have typed run states, conversation roles and fixed denial codes;
raw provider/storage error messages and system-message roles are not representable.
Session pages allow at most 100 unique IDs, 256-byte safe name/model labels and
signed-range revisions. History pages allow at most 100 user/assistant/tool entries,
64 KiB per entry, no NUL text and signed-range cursors. Other conversation text is
preserved; renderers remain responsible for terminal/HTML escaping. Lease durations
are structurally bounded to 1–30 seconds. Checking that duration does not establish
an authorization lease or renew one. All projections still require sharing checks;
the codec cannot decide whether disclosure is authorized.

`Command::validate_structure` intentionally does not compare the original deadline
with the current clock. After authenticating and authorizing the caller, an adapter
may use this structure to look up an already-admitted command's durable outcome.
An absent record must still pass `Command::validate(now_ms)` before new admission;
`Command::decode(bytes, now_ms)` retains that admission-time validation. Neither
reconnect nor structural decoding rewrites the deadline or admission identity.
Existing journal deduplication/admission behavior is unchanged.

This publishes previously uncompiled source, not a compatibility promise for an
existing deployed stream. Version 1 and unknown fields remain rejected. The v2
wire names for valid existing shapes are preserved; arbitrary state/role/error
strings are intentionally rejected. There is no storage migration or credential
reuse. Generated API documentation is available with
`cargo doc -p voyage-protocol --no-deps`.

Run `cargo test -p voyage-protocol --all-features` for public frame roundtrips,
legacy/unknown/duplicate/nested rejection, identity/count/Unicode/escape boundaries,
expired duplicate lookup versus new admission, proof-shape/context separation and
content-free diagnostics. These deterministic tests establish codec behavior only,
not runtime sharing, revocation, reconnect or transport backpressure.

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

## Current sharing checks

The [in-memory sharing authority](attachment-sharing.md) owns current policies and
intersects installation delegation, authenticated principal capabilities and consent.
Copied snapshots cannot authorize operations; per-session updates use a revision CAS,
and branch checks/insertion share one registry lock. These are local library checks,
not durable sharing or a transport authorization fence. All production adapters,
sharing persistence, dispatch-time rechecks and confirmation/audit integration remain
required before remote exposure.

## Integration requirements still open

- All local CLI/TUI and remote writers must use one coordinator. Local CLI/TUI now
  retain [JSON session execution ownership](session-ownership.md), including
  checkpoint/steering writers and in-process session changes. Those sidecars do not
  consult this journal or atomically commit command identity with canonical history;
  a single-authority coordinator migration remains required.
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

### Platform storage paths

Session and enrollment storage reject user-created symlinks in the directory,
ancestors, data files and lock files. On macOS, the OS-managed `/var`, `/tmp` and
`/etc` aliases are accepted only when root-owned, pointing to the expected
`/private` counterpart, under a root directory that is root-owned and not writable
by other users. This permits normal macOS temporary and data paths without
accepting arbitrary canonicalizable storage redirects.

Session replacement syncs file contents on all platforms and directory entries on
Unix. Windows directory-entry power-loss durability remains unproven; portable
save/load/replace/delete tests do not establish that guarantee. Enrollment still
fails closed on non-Unix platforms pending native ACL validation.

The explicit local [session transfer foundation](session-journal-transfer.md) provides
crash-resumable JSON-to-journal authority transfer and a quiescent schema upgrade,
without frontend wiring or automatic operator-state migration.
