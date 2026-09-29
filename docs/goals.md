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
not refund its charge. Terminal settlement records aggregate token/time usage
once, retaining an immutable receipt across reopen. Missing aggregate usage
retains the known lower bound, increments `unmeasured_runs`, stops continuation
and prevents Resume from disguising an unknown cost as fresh budget. An explicit
replacement creates a new Goal. A successful run does not complete the Goal.

Newly admitted Goal turns install a local provider meter shared across cloned
child configurations. A private journal observer records each request before
dispatch and persists cumulative usage before forwarding it to the agent. Request
IDs, ordered revisions and terminal records prevent reset, reordering, decreasing
counts and cross-incarnation reuse. Checkpoint failure stops dispatch or delivery;
restart retains known child counts and marks open requests uncertain. The meter
distinguishes explicit zero from missing counters and retains partial counts on
cancellation. Schema 16 fences writers that cannot preserve these observations and delegated-run receipts.

The meter refuses subsequent requests after uncertainty or observed token/time
limits. It also adds a deny-only check to the existing execution authority, so
tools and local children cannot continue after the budget ends or ignore an
upstream revocation. A time limit requests cancellation and awaits the same
execution through cleanup. Goal settlement releases the active steering callback
and observes cleanup under the admission lock before allowing a new turn.
Elapsed settlement includes admission, setup and cleanup time.

A delegated `Submit` can carry a deny-only `ExecutionBudget` bound to its exact
command, child session and parent session/run identities. Its finite token allowance, elapsed
limit and absolute expiry are enforced by the receiving Voyage without creating
a Goal or granting continuation. The budget is part of admission deduplication;
ordinary submissions omit it and retain their existing serialized form.

A budgeted run publishes an immutable `execution_usage` snapshot receipt with its
budget, child session/run identity, known counters and separate completeness and
observed-cleanup flags. The existing History-authorized command receipt also
returns this usage after later child runs, preserving exact-command lookup.
Recovery retains known counters and marks interrupted
accounting incomplete; it never redispatches a request. Schema 16 migrates the
existing request ledger and scrubs both ordinary Goal and delegated meter records
when deleting a session. Receiving-side support is staged; parent allocation,
participant transport and capability admission still need integration.

Requested output is capped to the remaining allowance; input/billing totals are
known only after dispatch, so this is not a strict billing ceiling. The meter is
not serializable authority. Remote subordinate budget/accounting is still
unfinished: currently, enabling Vessel tool routes or participant endpoints
makes the aggregate incomplete even if all local requests reported usage. The
native execution checks exercise this distinction explicitly. Remove this staged
limitation by completing remote accounting before enabling automatic continuation
or claiming feature acceptance.

Empty turns and previously seen successful tool-result batches increase the
consecutive no-progress counter. Novel batches reset it. This is a bounded
heuristic, not verification of the objective. Cancelled/interrupted/failed runs,
approval decisions and unconfirmed cleanup stop automatic continuation; an
explicit concurrent Pause retains its status while usage is still charged.

A continuation reservation contains one exact Submit command, the current
process incarnation, Goal identity and the private authorization binding. Only
one reservation may be outstanding per session. Pause or competing input changes
the canonical revision before that command can be admitted. An unadmitted
abandoned command receives a permanent `not_admitted` receipt. Startup recovery
reconciles the retained reservation after ordinary interrupted-run recovery:
unadmitted work is abandoned permanently, and accepted terminal work settles its
known usage once with an unknown-aggregate marker. Neither path redispatches a
command. The journal schema upgrade fences older writers before introducing
settlement records. The execution scheduler is still pending.

Snapshots expose the History-authorized objective. Ordered `goal` observations
carry only identity, revision, status, usage and a fixed stop reason; they omit
objective text and private authority. Client reducers and controls still need
integration. Model completion/blocked reporting and its evidence checks are
also pending; the owner mutation API cannot claim completion.

Before delivery, complete runtime continuation, usage/evidence enforcement,
restart/cancellation/approval handling, both Helm clients, offline process
journeys, full workspace coverage and hosted build/artifact verification. The
state-layer tests alone do not establish #378 acceptance.

Focused offline checks during implementation (from the repository root, with
private test fixtures) are:

```sh
umask 077
cargo test -p voyage -p vessel -p voyage-protocol --locked --lib goal -j 4
cargo test -p voyage --locked --lib -j 4
cargo check -p helm -p vessel --locked -j 4
```

The library suite also exercises the read-only catalogue against current journals;
schema changes must preserve that reader alongside owner recovery. These checks
do not replace final workspace coverage, client journeys or hosted builds.
