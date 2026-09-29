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
cancellation. Schema 19 fences writers that cannot preserve these observations, child allocations, delegated-run receipts late accounting and dispatch/non-admission records.

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
when deleting a session. The `execution_budget` capability advertises this bounded
single-command contract; Goal continuation remains unadvertised.

Parent allocation records precede remote process creation or submission. Each
allocation binds the destination Vessel, child session, command and parent run.
The shared meter reserves half of its available tokens, retaining parent capacity;
the journal independently clamps the allocation against retained usage, outstanding
reservations and the parent deadline. Each turn admits at most 128 child allocations
within its existing 10,000-request ledger limit. Local cloned child configurations
share this meter; independent child Voyages receive explicit budgets and propagate
bounded allocations for their own children.

Both participant assignments and Vessel-tool Create/Submit require a peer advertising
`execution_budget`. Unknown or unsupported peers cannot fall back to unbounded
submission. Existing-child Submit also requires a History-authorized snapshot
showing idle state and observed cleanup before allocation. Goal-driven steering of an existing child is refused because steering
cannot establish a budget for its current run. Model tool arguments cannot supply
or enlarge an allocation.

One observer per allocation polls exact authenticated receipts without resubmitting
work. Participant observations wait for the child's usage receipt before freezing
a terminal result and stopping the child process. Receipt import checks all stored
identities, persists the child counters once and rejects changed receipts. Unknown
outcomes retain their reservation. Known usage and observed cleanup remain separate:
a complete usage report with unobserved cleanup keeps its known counts and blocks
further substantive work with an unresolved-effects condition.

A completed parent waits for pending allocation accounting within its remaining
deadline plus five seconds for cleanup observation. Cancellation ends that wait;
it does not certify child cleanup. New submission cannot replace the active handle
while terminal accounting is still finishing. Outstanding allocations survive parent
failure/restart as unresolved records and known lower bounds.

History-authorized `goal_reconcile` reads a bounded page of retained allocations
(`offset`, `limit` in 1..128) from an idle owner and uses current trusted Vessel or
participant routes. The optional `fence_children` action is separate: it requires
human owner authority with History, Execute and Cancel rights. It resolves an exact
retained Submit without executing it; participant fencing may cancel an admitted
assignment through its existing cancellation boundary. A pass has a 15-second network deadline. It authenticates the
destination and exact child budget/command before importing a receipt. The response
reports observed/pending entries and the next page. Missing routes, revoked History
rights and unavailable peers retain pending obligations. It never submits child work,
restores continuation or treats a timeout/unknown command as zero usage.

Late receipt imports survive owner restart and add only previously uncounted token
usage. The original terminal receipt remains immutable. An exact command receipt
also exposes `execution_usage_observed`, a separately named current lower bound
that can carry nested late usage and observed cleanup. Increasing counts are imported
once; changed attribution, changed original receipts, decreasing counters and false
completeness upgrades are rejected. Cleanup improvement additionally needs retained
local cleanup evidence and resolved canonical obligations. Diagnostic progress text
or an operator attestation is insufficient.

The originating Goal receives the delta; a replacement Goal is never charged for
old work. Unknown-run markers and stopped/paused status remain unchanged. If usage
was interrupted, explicit replacement is still required to authorize a new budget.
Each new allocation has a durable dispatch gate. The exact child Submit or participant
assignment is retained before execution dispatch. A terminal parent can close a gated
allocation that never reached dispatch; legacy allocations without this gate remain
unknown. A dispatched allocation needs a permanent, authenticated non-admission
receipt. These records are distinct from a fabricated zero-token child run. They
close the command identity, preserve the parent's admitted-run charge, and cannot be
replaced by an admitted child receipt. Known non-admission releases unused token
capacity while the parent is live; a late proof does not reset an already retained
unknown-run marker or restore authority.

Participant observations carry immutable child usage and a separate monotonic lower
bound before the final result is available. A terminal child may run bounded,
read-only reconciliation of its own descendants. Its process is stopped only after
positive local and subordinate cleanup evidence, with the final result saved first.
Counters survive incomplete cleanup. Both the original result and original usage
remain immutable. Final parent measurement and settlement share a serialization
lock with asynchronous allocation imports, including cancellation paths.

These ledger and scripted transport checks still require separate-process restart,
cancellation and nested delegation journeys before #378 acceptance.

Requested output is capped to the remaining unreserved allowance; input/billing
totals are known only after dispatch, so this is not a strict billing ceiling.
The meter is not serializable authority. Merely configuring a remote route no longer
marks otherwise complete usage unknown: actual durable allocations determine
aggregate completeness. Separate-process interruption, nested delegation and both
client journeys remain release verification obligations.

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
