# Managed session lifetime ownership

Tracking: [#78](https://github.com/o-psi/voyage/issues/78). `helm managed` and
`helm remote-worker` use this owner for their explicit Journal-backed sessions.
Ordinary run, plain-chat and TUI sessions use fenced JSON persistence; automatic
transfer and full managed frontend integration are not enabled.

This is the local execution owner on one Helm, not the coordinating Helm role in
a planned [voyage](voyages.md). An interface may eventually observe or steer from
another authorized Helm while execution ownership stays with its executor. The
current owner API does not implement cross-machine handoff or coordinator selection.

`attachment::runtime::ManagedSessionOwner::open(directory, session_id)` acquires
one existing journal session's execution guard before reading authoritative data.
The cloneable owner retains that guard during idle time, turns, cleanup and handoff.
It contains no JSON store. Database work runs on blocking workers; their owned
clones retain the guard even if the async caller aborts its wait. Contending
processes fail through the journal's bounded, nonwaiting execution lock.

`owner.snapshot()` reads current committed state and returns SQLite's revision in
both `VersionedSession.revision` and the returned `Session.revision`. Only this
returned revision is normalized: canonical text, provider continuation, usage and
other metadata are preserved, and snapshot reads do not write the database.

`owner.admit(request)` uses the existing guard. The request must name the
owned session and satisfy journal identity, revision, expiry and durable admission
checks. Identical accepted command retries return `Admission::Existing`, including
while execution or cleanup retains the owner; they never produce another executor.
A conflicting command digest remains an error. The production asynchronous entrypoints sample `SystemClock` inside the blocking
admission transaction after duplicate lookup; waiting for a worker or mutex cannot
reuse an earlier timestamp. Clock failures and invalid values prevent new admission.
Existing identical receipts remain readable without consulting the clock. The
trusted `RuntimeClock` abstraction also supports deterministic boundary tests.
Authenticated/local frontend adapters must still check current authorization;
this API is not proof of sharing or policy permission.

Each new `RunOwner` holds a distinct turn token. `run.checkpoint()` produces a
cloneable `ManagedRunCheckpoint`; every callback and in-flight storage operation
retains the token and session guard. A new turn is refused until the previous run
and all checkpoint handles/workers have been released, even after its terminal
record commits. This allows the caller to finish terminal/subagent cleanup before
a subsequent turn begins. Release completed run/checkpoint handles after cleanup;
do not keep stale callback handles in a permanent frontend cache. Session-owner
clones may remain across many turns. Callback operations verify current run and
session identity, and journal phase checks reject callbacks after terminal state.

The standalone `RunOwner::admit` convenience remains available: it performs the
read-only duplicate lookup, then opens a managed owner and admits through it.
Callers using an existing lifetime owner must call that owner's admission method,
not the standalone method which would contend with their own execution guard.

Dropping a turn does not claim it completed or retry its effects. After all turn
and callback owners are gone, `owner.recover_interrupted()` can explicitly mark an
abandoned active run interrupted using the same journal guard. Recovery refuses
currently owned turns. Unresolved tool intent remains unresolved and blocks later
admission. Cancellation is still cooperative execution behavior: this owner is
not a replacement for resource supervision, scoped cancellation, or awaited cleanup.

Tests cover real child-process exclusion during idle/execution/terminal/callback
lifetimes; repeated turns through one owner; abort of execution and blocking-storage
waiters; checkpoint/terminal failures; explicit recovery; mismatched sessions,
revisions and callbacks; duplicate commands; exactly-once usage and effects; and
canonical/provider-state preservation. Platform execution/storage guarantees remain
those of the journal's verified native storage implementation.

[Durable managed steering](durable-steering.md) now provides receipt-aware queue,
application and rejection persistence. Its frontend projection remains dependent
work alongside guarded metadata and branch/deletion operations and complete
run/plain/TUI integration. Stable local actor identity and explicit `helm managed`
selection are implemented; managed TUI projection is not. A managed frontend must
reload committed
usage after a run rather than add its outcome usage again. No JSON/SQLite dual-write
backend is introduced by this prerequisite.
