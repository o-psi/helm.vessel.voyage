# Explicit local session transfer foundation

Tracking: [#78](https://github.com/o-psi/voyage/issues/78). This library foundation
is exercised against disposable test data. No CLI, TUI, network handler, automatic
scan, or operator-state transfer invokes it. Ordinary chat/run JSON frontends
retain authority over their own sessions;
[managed frontends](local-managed-sessions.md) use separate journal sessions.
Unified frontend admission remains unfinished. This does not complete the
attachment epic.

`attachment::migration::transfer` requires an explicitly selected session UUID,
source revision, SHA-256 of the exact original JSON bytes, fresh transfer UUID,
unowned SessionStore, destination journal directory, and the configured workspace
Coordinator. Consent, process quiescence, authorization and any eventual presentation
of these choices remain caller responsibilities. A bound frontend store cannot be
reused to bypass its active execution lease. The coordinator path must match the
source data directory's `completion/<original workspace path hash>` and canonical
workspace; a different lock directory for the same workspace is rejected.

The engine acquires the stable session execution lease and workspace agent-writer
lease before changing authority. Work runs on a blocking worker which owns both
leases until it finishes, including when its async caller is dropped. Cancellation
is checked before publication boundaries; after a durable commit the caller may
receive cancellation and must retry the same transfer identity to learn the result.
Session lock contention is bounded by the existing two-second limit; workspace and
backup-capacity locks fail immediately when busy.

The durable ordering is:

1. Save exact original bytes to a private, create-only backup under
   `journal/imports/<transfer UUID>.json`; sync the file and directory.
2. Atomically replace the original JSON with a strict versioned transfer marker;
   sync its file and source directory. The marker has none of Session's required
   fields, so existing JSON readers cannot treat it as executable history.
3. Insert the canonical session and immutable provenance together in one SQLite
   FULL-synchronous transaction. Preserve source revision, model/history, messages,
   usage, run summaries and unresolved tool intent. No run or tool is dispatched.

The marker stays permanently in the source location; there is no second JSON write
after database commit. Before marker publication the original remains authoritative.
After publication but before database commit the transfer is pending and neither
store admits execution for this session. After commit the journal owns it. A retry
verifies the marker, deterministic backup path, original bytes/hash, revision,
workspace and destination against immutable provenance. It returns the current
journal revision without replacing a snapshot that has advanced. Restoring an
original JSON file alongside completed journal provenance is an authority conflict,
not a request to overwrite journal state. No implicit backup deletion, rollback to
active JSON, or orphan cleanup occurs.

Imports are independently capped at 256 backup files and 256 MiB of logical bytes,
in addition to the journal's 256 MiB database cap and 16 MiB source/snapshot bound.
A stable `imports.lock` serializes capacity checks and publication across sessions.
An exact existing backup consumes no additional reservation, allowing retry at the
cap. Interrupted pre-marker attempts retain their backups and count against these
limits. Any future cleanup must prove they are unreferenced; this API does not delete
them. Legacy System messages and unrecognized top-level Session fields fail before
source transition and need explicit canonical cleanup or schema support first.

Source, destination parent, coordinator, backup directories and files must have
private Unix ownership and modes. Symlink paths, nonregular files, hard links,
corrupt backups, unsupported markers and hash/revision mismatches fail closed.
Non-Unix transfer is explicitly unsupported before any write: private Journal ACLs
and durable replacement/directory-publication support still need implementation
and verification. This restriction does not disable portable journal schema and
provenance tests. Filesystem locks assume local storage and cooperating processes;
hostile code running as the same OS user is outside this boundary.

## Explicit journal schema upgrade

New journals use schema 8, including provenance,
[durable steering receipts](durable-steering.md), managed catalogue, reconciliation
and dedicated remote-session state with permanent local grant withdrawal. Existing
schema 2–7 journals retain their supported
behavior; `open` never upgrades them. Transfer requires the current schema before
touching the source marker. `Journal::upgrade_quiescent` is an explicit operation: stop all other
journal processes first, finish or recover active runs, then upgrade. It holds a
SQLite immediate transaction and every existing session's execution sidecar while
adding missing versioned tables and changing the schema version. The transaction excludes new
sessions and admissions; sidecars exclude effects that outlive a terminal row.
Canonical snapshots, runs, replay and deduplication records remain unchanged.
Failures preserve the original schema. Current handles check schema under each write
transaction and reject stale handles after upgrade. Older binaries reject an unsupported newer schema
when reopening, but an already-open old binary cannot be made to adopt new checks;
explicit process quiescence is required, not inferred from PID files.

## Verification and remaining integration

Routine workspace tests cover successful transfer, all six publication/commit
boundaries both with returned failures and abrupt subprocess termination, same-ID
retry after an advanced journal, restored-source conflict, pending tool admission
rejection, cancellation before acquisition, wrong locks/identity/hash, corrupt
marker/backup, symlink/hardlink/private-mode failures and backup capacity/locking.
Portable journal tests cover schema upgrade fencing and rollback, retention of
canonical/run/replay/dedup state, immutable provenance collisions and transactional
rollback on provenance insertion failure. These tests exercise temporary local
data, not operator state or live provider capacity.

Remaining work includes full frontend coordinator wiring, remote admission under
existing consent/policy checks, operator-facing transfer/recovery, non-Unix transfer
publication support, and release/platform evidence. Provider execution, TUI rendering
and wire protocols are unchanged by this slice; imported pending effects remain
ambiguous and admission rejects them until a later explicit recovery decision.
