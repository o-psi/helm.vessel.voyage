# Local session execution ownership

Tracking [#78](https://github.com/o-psi/voyage/issues/78). This closes a local JSON
writer race; it does not expose attachment or satisfy atomic remote command
admission. The [attachment production contract](attachment-production-contract.md)
and journal migration requirements still apply.

A saved or resumed CLI/TUI session now has one execution owner, held with the
existing permanent `.<session-id>.lock` sidecar in the canonical session store.
Ownership is acquired before authoritative history is loaded, before model changes
or accepted prompts are saved, and before provider/tool execution. An independent
frontend waits at most two seconds and then reports `session busy` rather than
waiting indefinitely or overwriting the active session. An idle chat frontend
retains ownership until it exits or changes sessions. Listing remains read-only.

`SessionStore::with_execution` returns an immutable lease-bound clone. All its
matching-session saves/deletes reuse the same owner and still compare revisions;
clones retain the lock until the last owner is dropped. Different session IDs use
separate leases. Cancellation of an acquisition waiter does not release another
process's owner, and process exit releases OS ownership without removing sidecars.
Storage/private-path checks and the existing Windows durability limitations remain.

`load_owned` resolves an ID, name or permitted local path, acquires ownership, then
reloads authoritative state and revalidates the reference. Stale pre-lock content
never drives execution. Paths must name the canonical `ID.json` in this store;
foreign snapshots, arbitrary filename aliases and symlinks are rejected. OS-managed
macOS directory aliases retain the existing narrowly validated exception. Names
remain convenience lookups, not durable remote identities.

Plain `/new`, TUI `/new`/Ctrl+N and `/branch`/Ctrl+B obtain the new session's owner
before switching. Branch publication occurs under its new owner. The old owner
releases after outstanding references are gone. TUI holds the active owner through
its shutdown path and owned-subagent drain, then releases it before same-session
CLI handoff. Session checkpoints and accepted steering use the same bound store.
`--no-save --resume` also fences execution while leaving the saved transcript
unchanged; a no-save run does not become durable command deduplication.

No session JSON format changes. Existing sessions, canonical provider continuation,
steering receipts, summaries and original text remain local and readable. A lock or
CAS failure never authorizes provider/tool retries. Run-long JSON ownership cannot
atomically commit a remote command digest, prompt, event sequence and run state;
those require the future single-authority journal/coordinator migration. Legacy
clients that do not honor these locks are not fenced by application policy.

## Verification

The real-process `tests/system/session_fencing.py` regression starts an owner in a
native provider request, then tries a conflicting metadata mutation. Before the
fix the second CLI renamed the active session and exited successfully; afterward
it must fail busy without a request or session change. The fixture also covers a
resumed no-save run, canonical-path and name lookup, idle ownership, plain `/new`
transfer, old-session release and successful new-session execution.

Session unit tests cover clone retention, reload-after-lock, changed names,
foreign/alias paths, busy deadlines, cancelled waiters, deletion/resurrection, and
owned branch publication. TUI command tests cover new/branch rebinding and retained
ownership through clearing. Existing steering, questions, context recovery,
completion, interruption and PTY handoff regressions remain required.
