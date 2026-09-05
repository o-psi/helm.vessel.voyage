# Durable completion ledger storage

Tracking: [#81](https://github.com/o-psi/voyage/issues/81), prerequisite for
[#82](https://github.com/o-psi/voyage/issues/82) and [#83](https://github.com/o-psi/voyage/issues/83).
See the [completion contract and remaining integration work](completion-gate.md).

`completion::store::{RunScope, RunLedgerStore}` is implemented but **not yet called
by the runtime**. It is not a new tool, CLI command, session migration or enabled
gate. Exporting this API does not modify existing runs or stores.

## Scope and persistence

- A scope combines an existing canonical workspace directory and a session UUID;
  each ledger retains its distinct run UUID. Envelopes validate all three identities.
  Copying a ledger under another session filename does not authorize it.
- Open a dedicated directory under a trusted existing data directory. New Unix
  directories/files are private from creation (0700/0600); nonprivate existing
  directories/files are rejected, not silently chmodded. Windows relies on the
  trusted parent ACL. Symlink and nonregular final components are rejected; Unix
  opens also use `O_NOFOLLOW`. This is not a sandbox against hostile same-user code
  or replacement of trusted ancestor directories.
- `create` never overwrites a run. `load` requires the known run to exist; missing,
  corrupt, future-version, wrong-scope and oversized records fail closed. There is
  no empty-ledger fallback, implicit adoption, expiry or deletion.
- `update` reloads under a stable OS sidecar lock and checks the expected revision
  before applying a closure to a candidate. Mutation/write failures before atomic
  replacement preserve the prior file. Writes use private temporary files, file
  sync and atomic replacement; Unix also syncs the directory and its parent
  during initialization. Changed content must advance the revision; updates cannot
  remove existing membership or review history, even via a decoded replacement.
- Inner ledger JSON remains a string inside a strict version-1 envelope so its
  decoder still detects duplicate fields. Reads cap the envelope at 16 MiB + 64 KiB,
  allowing JSON escaping of the bounded 8 MiB inner ledger. Inner limits still
  apply. Scope metadata and disposition reasons are sensitive local data; callers
  must redact known secrets before submitting reasons through normal runtime paths.

## Concurrency, failure and recovery

- Lock acquisition is nonblocking: contention or unsupported locking returns an
  error immediately. Guard drop explicitly unlocks, even with duplicated/inherited
  handles; process exit releases OS locks. Never remove/replace `writer.lock` while
  any handle is active. This serializes **cooperating ledger callers only**, not
  todo, agent or session edits, and is not a cross-store acceptance transaction.
- These synchronous filesystem methods belong on a blocking persistence executor,
  never around provider/tool calls or approval waits. File locking does not wait,
  but filesystem latency itself is not bounded by this API.
- A directory-sync error after replacement is an **uncertain durability outcome**:
  reload and inspect the revision; never blindly retry consequential work. There
  is no claim of power-loss testing or transactional session publication. Publish
  session run references only after creation succeeds; recovery must inspect the
  obligations, never infer that terminal processes survived.
- Old binaries do not use this separate format. Rolling back this currently unused
  API does not migrate/discard sessions, todos or agents. Preserve experimental
  ledgers for inspection rather than deleting them on rollback.

On Windows, atomic visibility is implemented, but power-loss-durable rename is
**not guaranteed** by the current `tempfile` replacement path. This API must not
be used as the durable Windows session-publication boundary until an appropriate
commit strategy is implemented and verified. Portable compilation/reopen tests
do not resolve that limitation.

## Tests and limitations

Ten disk tests cover round-trip/reopen, stale revisions, failed mutations/writes,
no-clobber creation, missing/corrupt/oversized data, strict envelopes and inner fields,
scope/identity isolation, private permissions, symlinks/nonregular targets,
nonblocking contention, duplicated descriptors, fresh-process reads/writes, and
forced process termination while holding the lock. Child test waits are bounded.

```sh
cargo test -p helm --lib completion::store:: --all-features
```

These are storage tests, **not completion-gate E2E evidence**. Windows/macOS execution
and power-loss durability remain unverified for this API. There is no shared wire
change, TUI behavior, provider dispatch or tool authority expansion in this slice;
those integration layers remain mandatory before enabling the gate.
