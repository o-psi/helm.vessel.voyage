# Durable completion ledger storage

Tracking: [#81](https://github.com/o-psi/voyage/issues/81), prerequisite for
[#82](https://github.com/o-psi/voyage/issues/82) and [#83](https://github.com/o-psi/voyage/issues/83).
See the [completion contract](completion-gate.md).

`completion::store::{RunScope, RunLedgerStore}` supplies the durable ledgers used
by the [scoped runtime](completion-runtime.md) and enabled completion gate. It is
a local persistence API, not a cross-machine voyage ledger or an independent
CLI command. The runtime coordinates its writers with local todo and agent stores.

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
- Older binaries reject sealed schema-2 ledgers. Rolling back must preserve
  ledgers, sessions, todos and agents; never strip the seal or downgrade its version.

On Windows, the flushed temporary file is closed and published with
[`MoveFileExW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
and `MOVEFILE_WRITE_THROUGH`; replacement additionally uses
`MOVEFILE_REPLACE_EXISTING`. Both paths remain in the same directory and never
allow a cross-volume copy/delete fallback. Creation keeps no-clobber semantics.
The Win32 error propagates without publishing acceptance; temporary-file cleanup
remains armed on failure. Filesystem and device
flush semantics still bound the guarantee; this is not proof from power-loss tests
or a multi-file session transaction. Windows CI must execute the storage and seal
regressions before claiming platform verification.

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
and power-loss durability remain unverified for this API. The
[acceptance evidence map](completion-validation.md) separately records runtime,
frontend and provider integration evidence. Storage tests alone do not establish
those workflows or the planned cross-machine voyage contract.
