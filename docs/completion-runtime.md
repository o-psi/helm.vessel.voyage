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
ownership. Following up work from another run requires explicit adoption; archived
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
Its actions are:

- `snapshot`: bounded readiness counts, unresolved IDs, revision and fingerprint.
- `read`: bounded details of one owned todo or agent, including archived results.
- `adopt`: explicitly register an existing record using the current ledger revision.
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

The integration API does not itself wire ordinary CLI/TUI session lifecycle or
intercept final responses. The parent delivery must create/recover session run
references, install coordinated stores, propagate the context handle, register the
tool, and apply #82's bounded final gate. Keeping the existing default `None` context
means legacy library callers retain their prior behavior.

Deterministic integration tests cover run isolation; explicit adoption; evidence
changes; honest blocked/deferred accounting; archived/deleted records; missing
ledgers; registration and publication failures; restart; nested child/followup
inheritance; cross-run rejection; shared-writer exclusion; fresh-process lock
contention; cancellation of a writer's waiter; strict tool arguments; and redaction
before durable review storage. They complement the existing readiness/status and
ledger storage matrices.

The ledger store's documented [Windows power-loss-durable rename limitation](completion-storage.md)
still applies. This integration must not be claimed as a verified durable Windows
final-publication boundary until that hold is resolved. No new wire contract,
provider dispatch, or cleanup behavior is introduced by these APIs alone. Native
provider/TUI/worker flows, finalization races, live behavioral evaluation and platform
CI belong to the integrated delivery; existing unit passes cannot replace them.
