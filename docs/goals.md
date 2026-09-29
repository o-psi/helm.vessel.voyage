# Persistent Goals (#378)

This is an implementation checkpoint for the v1.0.3 Goal feature. Automatic continuation and model
reporting are implemented in Voyage, with Helm Web and TUI controls. Vessel
advertises `goals` and `execution_budget`; capabilities do not grant authority.
Release qualification remains in progress.
The release gate remains [#378](https://github.com/o-psi/helm.vessel.voyage/issues/378).

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
rejected by the public mutation validator. Human Goal updates use the existing
two-second SQLite contention budget on a blocking worker, rechecking live grant
authority while waiting. Only pending storage statements wait; the mutation is
dispatched once. Exhaustion or revoked authority remains a refusal.

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
cancellation. Schema 20 fences writers that cannot preserve these observations, child
allocations, delegated-run receipts, late accounting, dispatch/non-admission
records and evidence-linked Goal assessments.

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
single-command contract on both local and authenticated remote routes.

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

The ledger checks are supplemented by the separate-process journeys below.
Participant-specific transport fixtures remain synthetic; they do not establish
remote deployment or native-platform behavior.

Requested output is capped to the remaining unreserved allowance; input/billing
totals are known only after dispatch, so this is not a strict billing ceiling.
The meter is not serializable authority. Merely configuring a remote route no longer
marks otherwise complete usage unknown: actual durable allocations determine
aggregate completeness. The offline process journeys exercise interruption and nested delegation;
production deployment, final coverage and packaged-build evidence remain separate.

Empty turns and previously seen successful tool-result batches increase the
consecutive no-progress counter. Novel batches reset it. This is a bounded
heuristic, not verification of the objective. Tool invocation timing is excluded
from repeated-result comparisons. Cancelled/interrupted/failed runs,
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
settlement records.

One Voyage-local wake loop runs after startup recovery, owner mutations and terminal
settlement. Each wake re-reads the private authority and the current host grant;
installation/principal changes, revoked/expired grants and loss of human owner
scope stop the Goal. It reserves one exact Submit and uses ordinary admission.
The admission transaction checks again for pending input or effects after the
reservation, and refuses a paused or changed Goal. Idle suspension waits while
an authorized Goal remains active; client disconnect is not cancellation.

Unresolved command bindings for human input, steering or cancellation block
continuation, even across restart. Expiry alone does not prove non-admission;
exact receipt resolution closes the binding. Accepted ordinary input/steering
stops future continuation with `user_input` while preserving the current run and
usage. Required decisions, pending workflow input and unresolved effects stop
automatic admission. An explicit owner Resume is required after the obstruction
has been handled. A delayed failure cannot stop a replacement or undo a Pause.
The continuation prompt contains bounded Goal state as user task data.

Snapshots expose the History-authorized objective. Ordered `goal` observations
carry only identity, revision, status, usage and a fixed stop reason; they omit
objective text and private authority. Clients refresh the authenticated canonical
snapshot after Goal metadata events; they do not reconstruct objectives or
assessments from metadata. The root of a metered Goal run receives a `goal` model tool with
`read` and `report` actions. Local children and independently budgeted child
Voyages do not receive authority to report the parent's Goal. The owner mutation
API cannot claim completion, and the model tool cannot create, resume, clear or
increase a Goal. Reading/reporting this bound metadata is permitted during a
read-only run; filesystem, process and remote-tool policies remain authoritative.

A report binds its completion/blocked assessment to the current objective revision,
run, incarnation and exact canonical tool call. It needs bounded explanations and
actual tool-result IDs from the same run. Completion evidence must be successful
and complete. Invented, repeated, ambiguous, failed, stale and other-run evidence
is refused. Configured secrets are rejected before report persistence. A run can
record one immutable assessment; it remains pending until terminal settlement.

Settlement revalidates the evidence digest against the final canonical transcript.
A completed run, known aggregate usage, current authority and observed cleanup are
required. Pause, intervening input, cancellation, interruption, decisions, token/time
limits or unknown effects prevent promotion. A valid assessment may complete the
last allowed run; a run limit or no-progress limit alone never means success.
Blocked assessments carry their evidence separately from a limit stop. The Goal
snapshot retains the accepted assessment and evidence digest; public invalidation
events omit its text. Evidence validation establishes provenance and freshness;
it does not mechanically prove that the model's interpretation of an arbitrary
objective is correct. Goal-tool reads/reports do not count as automatic progress.

## Helm controls

Both clients read the canonical Goal snapshot and pin the Goal revision, identity
and process incarnation during review. An unrelated conversation checkpoint can
advance; a changed Goal or owner requires fresh review. Owner-only mutations use
the usual exact command and durable receipt path. Unknown or mismatched positive
receipts retain their pending identity; recovery observes/resolves that identity
without replaying the mutation. Runtime authority remains authoritative.

Helm Web's Goal panel shows status, usage, limits, stopped reasons and the recorded
model assessment. Set/replacement has an unchecked continuation option; replacing
or clearing requires explicit confirmation. Goal form drafts remain in memory.
The browser stores only the command identity for receipt recovery.

Helm TUI shows compact Goal status above the conversation. `/goal` reviews its
objective, limits, usage and assessment. These commands open an Enter-to-confirm
review; Esc cancels and PageUp/PageDown scroll even on small terminals:

```text
/goal set OBJECTIVE
/goal edit OBJECTIVE
/goal limits RUNS TOKENS SECONDS NO_PROGRESS_TURNS
/goal pause
/goal resume
/goal clear
```

Set creates/replaces a paused Goal with default limits; Resume is a separate
explicit authorization for automatic continuation while Helm is disconnected.
Edit/limits retain usage and pause continuation. Replacement and Clear name the
existing Goal in the confirmation. Pause can stop future continuation during a
run; `/stop` separately requests current-run cancellation. Other mutations wait
for idle and observed cleanup. History-only connections can read the review but
cannot mutate. TUI recovery retains the exact public command in its existing
private receipt store; it never persists unsent form/composer drafts.

## Offline process verification

`voyage/tests/goals.py` uses synthetic named accounts, loopback scripted inference
and actual Vessel-supervised independent Voyage processes. It covers continuation,
current-tool-result completion and impasse, false evidence, finite run/token/time
limits, missing usage, provider failure, approval-required work, human steering,
Pause, cancellation, suspension and killed-owner recovery without replay. A real
Helm PTY checks stale reviews against another authenticated client. Authenticated
remote owner/scoped grants check secret rejection and revocation during a run.

Nested journeys use both existing child voyages and model-requested creation,
with a separate grandchild. Local and explicitly paired remote routes check
finite allocations, identity pinning and exact aggregate usage. A parent cancelled while awaiting its children retains unknown usage;
late authenticated reconciliation adds counts once and never restores authority.
Normal control cleanup uses a child cancellation token so it cannot accidentally
cancel terminal accounting. Positive, exact suspension evidence can settle a
creation receipt even if a healthy owner retired before the first health probe.
Browser-state initialization uses a blocking checkpoint with the same bounded
SQLite statement wait, so a brief catalogue reader does not abort a fresh owner.
A separate regression holds an actual SQLite reader while the runtime initializer
waits; no browser or provider effect is dispatched by that storage operation.

The optional Web journey loads the separate Web repository's built production
bundle and pairs an isolated owner connection. Real browser socket credentials,
commands, snapshots and receipts cross a fixture TLS proxy; only the destination
address is redirected. Web creates a paused Goal, TUI observes and edits it, stale
Web review is refused, reconnect reads canonical limits, and confirmed Clear is
observed by the runtime. No provider inference is sent by these controls. This
does not test production OAuth, external TLS or public provider behavior.

From the runtime checkout, after building `helm`, `vessel` and `voyage`:

```sh
python3 voyage/tests/goals.py --bin-dir target/debug
python3 voyage/tests/goals.py --bin-dir target/debug --only web --web-root /absolute/path/to/webhelm
```

The Web checkout needs its existing production build, Playwright and Chromium;
these commands do not install dependencies. Private fixture records and logs stay
outside publication. Runtime source delivery still requires final workspace
coverage and hosted build/artifact verification. Passing state-layer tests alone
does not establish #378 acceptance.

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
