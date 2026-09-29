# Persistent Goals (#378)

This is an implementation checkpoint for the v1.0.3 Goal feature. The protocol
and canonical state layer is under development. It is not advertised as a
capability and does not yet schedule automatic continuation or provide Helm
controls. The release gate remains [#378](https://github.com/o-psi/helm.vessel.voyage/issues/378).

Voyage owns one Goal per session UUID in its canonical journal. The public record
contains its objective, status, finite limits, usage, timestamps and explicit
continuation authorization. A separate private record binds authorization to
the initiating installation, principal and optional current grant revision.
Goal text is user-authored data; it grants no filesystem, tool, provider or
approval rights. A Goal is separate from a configuration draft and a run.

`goal_read` requires History. `goal_update` requires an authenticated human owner
and Execute; an ordinary scoped Execute grant cannot set, resume, clear or
increase a Goal's limits. The private Voyage boundary independently enforces
the same distinction. Updates use existing command IDs, session revisions,
deadlines, durable reservations and exact receipts. Configured secrets are
rejected by the public mutation validator.

Set uses the command ID as the new Goal ID. Replacing a current Goal requires
its exact ID. Edit preserves accumulated usage and pauses continuation; Resume
records fresh authorization for the edited objective and limits. Pause may
stop future continuation during a run. Other changes require safe idle and no
unresolved continuation reservation. Clear preserves the Goal revision counter;
old commands cannot affect a later replacement. Session deletion scrubs Goal
state and continuation requests. A branch starts without a Goal and inherits
no continuation authority.

The finite defaults are 20 admitted runs, 200,000 total input/output tokens,
one hour of execution time, and three consecutive runs without progress.
Limits are owner-configurable within the bounds in
[`goals.rs`](../crates/voyage-protocol/src/goals.rs). The journal records an
admitted-run charge before a continuation effect; a failed reservation does
not refund its charge. Token, elapsed-time and subordinate usage settlement
still need to be connected to the execution path.

A continuation reservation contains one exact Submit command, the current
process incarnation, Goal identity and the private authorization binding. Only
one reservation may be outstanding per session. Pause or competing input changes
the canonical revision before that command can be admitted. An unadmitted
abandoned command receives a permanent `not_admitted` receipt. Recovery must
reconcile the retained reservation; it must never redispatch it just because
the owner process restarted. The execution scheduler and recovery integration
are still pending.

Snapshots expose the History-authorized objective. Ordered `goal` observations
carry only identity, revision, status, usage and a fixed stop reason; they omit
objective text and private authority. Client reducers and controls still need
integration. Model completion/blocked reporting and its evidence checks are
also pending; the owner mutation API cannot claim completion.

Before delivery, complete runtime continuation, usage/evidence enforcement,
restart/cancellation/approval handling, both Helm clients, offline process
journeys, full workspace coverage and hosted build/artifact verification. The
state-layer tests alone do not establish #378 acceptance.
