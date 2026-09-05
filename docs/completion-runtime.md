# Runtime ownership and readiness

Tracking: [#81](https://github.com/o-psi/voyage/issues/81), with final acceptance
in [#82](https://github.com/o-psi/voyage/issues/82) and end-to-end evidence in
[#83](https://github.com/o-psi/voyage/issues/83).

## Implemented integration API

`completion::runtime::Coordinator` serializes cooperating durable todo, subagent,
and ledger writers for a workspace. The same coordinator must be installed on
both stores. A private stable sidecar OS lock excludes other cooperating processes;
a shared asynchronous mutex serializes callers in one process. Local contention
has a five-second admission bound; cross-process contention fails immediately.
Filesystem latency is not bounded by these mechanisms.

`RunHandle::create` takes the trusted session and run UUIDs; `resume` requires the
known ledger to exist and validate. A checkpoint's execution UUID should be the
run UUID when available; ordinary local runs can allocate UUIDs without Vessel or
the attachment journal. `ToolContext.completion` carries the handle. Model tool
arguments cannot select their run, session, workspace, or coordinator.

`TodoTool` registers a newly allocated todo ID before publishing its record.
`SubagentTool` registers a new child before scheduling execution. Durable agent
records contain an optional version-compatible `RunReference`, and child execution
contexts inherit the handle. Nested spawns and terminal followups inherit run
ownership. Adopting a terminal subtree does not adopt work independently requested
by another run after that adoption boundary. Following up work from another run requires explicit adoption; archived
agents retain their existing historical-reference behavior and cannot restart.

Registration and record publication are separate durable commits. A publication
failure leaves a missing obligation, which blocks readiness and survives restart.
It does not erase obligations or pretend the requested work ran. Recovery validates
retained owned agents' ledgers before changing their state or archiving them.
Legacy records without a reference remain unowned.

Admitted todo/agent writes retain their coordination lease in an owned task until
the filesystem operation finishes, even if a tool waiter is cancelled. Blocking
ledger mutations likewise own their lease. Cancellation can therefore leave an
uncertain admitted mutation; inspect durable state rather than blindly repeating
it. It cannot release the lease while that mutation still commits.

## Explicit review

Register `completion::tool::CompletionTool` with the same todo and agent stores.
Operations observe the tool timeout and cancellation token. Its actions are:

- `snapshot`: bounded readiness counts, unresolved IDs, revision and fingerprint.
- `read`: bounded details of one owned todo or agent, including archived results.
- `adopt`: explicitly register an existing todo or an entire terminal agent subtree
  using the current ledger revision. Agent adoption includes archived descendants
  atomically. Any active node rejects the operation without adding membership:
  wait or cancel active work before adoption. Already-owned records are a no-op;
  adopting an active agent does not retarget its running execution context.
- `account`: record an evidence/status-compatible disposition and reason against
  both the revision and exact snapshot fingerprint. Concurrent evidence/result
  changes invalidate the review even when ledger membership did not change.

Known secrets are redacted from reasons before persistence and normal tool output
redaction applies. Read-only policy permits snapshot/read, not adoption/accounting.
Accounting never changes todo status, removes work, cancels agents, or proves the
semantic truth of a reason. Deferred and blocked work remains unfinished.

`readiness_lease` retains mutation exclusion while a caller records a final
acceptance decision. Never hold it over provider calls, tools, approvals or waits.
A plain `snapshot` releases the lease immediately and is not an acceptance lock.
All writers must participate; old binaries and uncoordinated direct store users
cannot safely share an enabled acceptance boundary.

## Exclusive subagent writer ownership

A coordinated `SubagentRuntime` acquires a separate lifetime OS lease before
reading/recovering its agent tree. A second runtime/process fails clearly as busy
before it can mark the live owner's records interrupted. The lease is shared by
the runtime's store clones and admitted persistence tasks; dropping a cancelled
waiter does not release it early. Coordinated direct writes without this lease
are rejected. Process exit releases the lease, after which a new runtime can
honestly mark unrecoverable running records interrupted and preserve obligations.

This intentionally permits one cooperating subagent runtime per workspace. Reuse
the existing runtime within a process when building additional clients. Separate
processes must not bypass the busy result, open old uncoordinated writers, or
remove the lease file. This is an admission restriction, not implementation of
#78's multi-client session coordinator. Independent run ledgers and workspace todos
remain isolated for readiness; a foreign unfinished todo does not block a run.


## Delivery boundaries and tests

Ordinary CLI and TUI runs install coordinated stores and the completion tool.
Before provider dispatch they allocate a distinct ledger, record its trusted session
reference, and save the accepted prompt (unless the user selected no-save). Resume
validates every known ledger and fails closed on missing or corrupt ownership.
Branching starts without the source session's run references; old unfinished work
requires explicit adoption. Children share the parent's run ownership and stores.
Legacy session files without references load with empty ownership. Legacy library
callers with the default `None` coordinator retain their prior behavior.

Final-response interception remains #82 work: readiness data alone does not prevent
an early final answer. The attachment checkpoint lifecycle still needs an explicit
scoped entry point before it can participate in this gate.

Deterministic integration tests cover run isolation; explicit adoption; evidence
changes; honest blocked/deferred accounting; archived/deleted records; missing
ledgers; registration and publication failures; restart; nested child/followup
inheritance; cross-run rejection; shared-writer exclusion; fresh-process lock
contention; cancellation of a writer's waiter; strict tool arguments; and redaction
before durable review storage. They complement the existing readiness/status and
ledger storage matrices.

The ledger store now requests [Windows write-through publication](completion-storage.md);
platform CI and filesystem/device flush limitations still apply. No new wire contract
or cleanup behavior is introduced. An offline native HTTP
fixture exercises durable prompt/run publication, resume isolation, nested ownership,
empty runs, corrupt/missing ledgers and real second-process exclusion. Finalization
races, live behavioral evaluation and platform CI remain separate delivery evidence;
unit and offline passes cannot replace them.

## Durable final decisions (#82 core)

`RunHandle::readiness_lease` excludes cooperating todo, agent and ledger writers.
Its `seal(FinalOutcome, reason)` consumes the lease and commits an immutable final
decision using a private, complete observation; changing the public display snapshot
cannot change the decision. `Completed` requires every obligation accounted for
and zero incomplete obligations. `Incomplete` and `Interrupted` retain all unresolved
IDs and require a bounded reason. Reasons must be redacted before this trusted API
is called. The snapshot also lists accounted incomplete IDs, so deferral or a
reviewed child failure cannot disappear behind a zero unresolved count.

The caller must durably checkpoint canonical proposal text **before sealing**, while
holding the lease, and publish accepted status only after sealing succeeds. Do not
hold this lease across provider requests, tools, approvals or child waits. A dropped
lease makes no decision. Cancellation of a seal waiter does not abort an in-progress
blocking write or release its lock early: inspect the durable decision before any
retry. A failed or uncertain write must never trigger an accepted event.

The seal is a historical decision, not a transaction across session and ledger
files. A crash after the proposal checkpoint but before the seal leaves a provisional
proposal. A crash after sealing but before frontend notification leaves a decision
available through `RunHandle::decision`; recovery must validate the saved proposal
and run identity before displaying it as accepted. A seal alone does not establish
that the corresponding session text was saved. Frontend integration remains
responsible for this ordering, including no-save behavior.

After sealing, registration, adoption (including duplicate adoption), reviews and
store updates are rejected. Shared task records remain editable by later independent
runs; their historical acceptance is not retroactively rewritten. To rely on the
accepted records as current, `validate_final_decision` rereads them under the
coordinator and compares the entire original observation. Changed evidence,
results, statuses or missing records cause an error. Reopening work requires a new
run with explicit adoption. This does not coordinate old binaries that ignore the
workspace coordinator.

Ledger schema 2 requires an explicit open/sealed state and rejects malformed or
unknown decision variants and inconsistent counts, ownership or revisions. Schema
1 ledgers migrate as open only when they contain no state field. Older binaries
reject schema 2 rather than treating a sealed run as empty or open. Preserve ledgers
on rollback; do not downgrade their version or strip the decision.
