# Completion readiness and reconciliation

Tracking: [#81](https://github.com/o-psi/voyage/issues/81),
[#82](https://github.com/o-psi/voyage/issues/82), and
[#83](https://github.com/o-psi/voyage/issues/83), under
[#5](https://github.com/o-psi/voyage/issues/5).

## Current delivery status

The provider-neutral agent loop gates final acceptance for explicitly scoped root
runs. The [durable ledger](completion-storage.md) and [runtime ownership
API](completion-runtime.md) register owned work before publication and serialize
record writers with the final decision. Inherited child ownership does not activate
a second root gate. Unscoped library calls do not enable this gate.

An embedding must configure `Agent::with_completion_gate` with the coordinated
todo store, agent store, and runtime, then pass the trusted run handle through
`run_scoped` or `run_checkpointed_scoped`. Scoped checkpoints must use that run's
exact ID. Missing gate resources fail closed. Durable frontends must supply a
canonical checkpoint; the uncheckpointed embedding API provides in-memory history
only and does not establish durable transcript-before-seal ordering. An explicit
no-save run must retain that distinction.

A run is one execution within a session. Its accepted final response does not close
an open-ended [voyage](voyages.md), bind that voyage to the executing Helm, or
prove that work on another Helm has been reviewed. Distributed voyage obligations
and result collection remain planned; current membership covers explicitly owned
local todos and subagents.

## Root final acceptance

Streamed assistant output and no-tool proposals remain provisional. The runtime
first consumes accepted steering, then acquires a fresh readiness lease. A fully
accounted snapshot needs no extra provider request: all-success work completes,
while accounted failures, blockers, and deferrals produce an explicit incomplete
outcome.

The first unresolved proposal is preserved in canonical history and triggers one
targeted, request-only system update containing bounded IDs, counts, states, and
review instructions. It does not create a synthetic human message or persist
runtime guidance. Normal tools, policy, approvals, and budgets remain in effect.
The next final proposal is checked against fresh state and either completes or
ends incomplete with structured IDs. Repeated proposals and tool turns cannot
restart the fixed reconciliation wall deadline (60 seconds by default).

The steering channel stays usable through reconciliation. Final acceptance closes
it atomically while the readiness lease excludes coordinated writers. The runtime
checkpoints canonical history before sealing the durable decision, then invokes
`RunCheckpoint::accepted` explicitly. Unscoped runs invoke that acceptance hook
as well; consumers must not infer acceptance from a no-tool message.

Cancellation, provider failure, exhausted context, checkpoint failure, and deadline
expiry return recoverable interrupted state, including partial assistant text.
Abort requests cancellation only for active descendants with the exact run/session
reference, including nested descendants whose parent already finished. Shutdown
requires both finished runtime transitions and durable terminal records within a
separate bounded budget (5 seconds by default). Failed or blocked persistence
leaves observation inconclusive; an in-memory terminal flag alone is insufficient. An incomplete finish also stops owned active work
before its final fresh check. Neither path closes unrelated terminals or performs
Git/worktree cleanup. No new provider request starts after cancellation.

A durable seal and frontend acknowledgement are distinct boundaries. If cancellation
or checkpoint acknowledgement fails after a successful seal, the caller receives
an interrupted recovery and no completed event; the already persisted decision
remains historical evidence. Failure debug formatting omits canonical content,
continuation state, and provider error bodies.

The optional compatibility bridge handles reconciliation by rebuilding its provider
thread with the changed system instructions. Its lack of active user steering does
not prevent this request-boundary operation. Native transports remain independent
of a Codex executable.

## Readiness contract implemented so far

- Each `RunLedger` has a distinct `RunId`, unrelated to session IDs, workspace paths,
  or individual execution IDs. Membership is explicit and append-only.
- Trusted callers register created/adopted todos and agents. `adopt_child` requires
  an already-owned parent; the spawn coordinator must verify the actual relationship
  and register descendants and terminal follow-ups before publishing execution.
  Listing, assigning, resuming, or loading unowned work does not adopt it.
- A completed todo needs nonblank evidence plus an explicit review. Cancellation
  needs a reason. Blocked work needs blockers and an impact review. Deferral retains
  pending/in-progress/blocked status: there is no invented `Deferred` todo status.
- An active child cannot be accounted for by a disposition. Completed results must
  be explicitly incorporated or declared unnecessary with a reason. Failed,
  cancelled, interrupted, and timed-out children need an impact or not-needed
  disposition; they never count as successful completions.
- Dispositions bind to a SHA-256 fingerprint of the complete reviewed record. A
  change to evidence, results, status, assignments, archive metadata, or other fields
  invalidates the review. Previous disposition reasons remain in append-only history.
- Snapshots include archived records. Missing/deleted records remain unresolved;
  deleting a store cannot make a nonempty ledger ready. Nothing automatically
  removes, archives, completes, unassigns, integrates, or cleans up work.
- A ready snapshot means **accounted for**, not that every task succeeded. Separate
  completed and incomplete counts preserve truthful blockers and failures.
- Readiness diagnostics contain IDs, reason codes, counts, and fingerprints—not
  task descriptions, result text, or evidence. Detailed records must be fetched
  through normal scoped policy/redaction paths. Unrelated records do not affect
  the snapshot fingerprint or appear in its diagnostics.

These are mechanical checks. Nonblank evidence/reasons cannot prove that a model
actually verified a result or described its impact correctly. Behavioral evaluations
and result synthesis remain necessary; a model can still make false semantic claims.

## Bounds, revisions, and serialization

The initial contract permits 1,024 obligations, 16 reviews per obligation, 4,096
UTF-8 bytes per reason, and an 8 MiB serialized ledger. Exceeding a bound fails
without changing the ledger; it never discards history to make room. Reasons reject
control characters. The output cap limits unresolved diagnostics, not evaluation:
all obligations are checked, omitted counts remain visible, and an output cap of
zero cannot turn unresolved work into readiness.

Mutations require an expected ledger revision. Duplicate adoption is idempotent
but still rejects a stale revision. The readiness fingerprint covers membership,
disposition history, and current owned-record fingerprints. It deliberately ignores
unrelated store revisions. Revision overflow fails without partial mutation.

`RunLedger::to_json`/`from_json` provide versioned serialization. The separate
[RunLedgerStore API](completion-storage.md) provides disk persistence, with coordinated runtime writers and atomic final sealing.
Malformed, oversized, duplicate-membership, missing-field, and future-version data
are rejected. Existing todo/agent/session formats are unchanged. The integration
must not treat an absent or corrupt ledger for a known run as a new empty ledger.
Sessions without a run reference must not implicitly adopt historical work.

## Implementation and acceptance status

### #81: ownership and persistence

The local runtime now propagates trusted root identity through tools, children and
follow-ups, while preserving separate execution IDs. The coordinated stores publish
ownership before dispatch, serialize owned mutations and acceptance, and fence
competing processes. Explicit adoption/accounting requires fresh scoped reads;
private persistence, expected revisions and recovery fail closed on missing or
corrupt known-run state. Bounded `completion read` retrieves owned evidence without
embedding it in the diagnostic snapshot.

These are implemented local contracts, not remaining foundation work. Their
executable coverage includes `completion::runtime`, `agent::gate_tests` and the
real-process `completion_readiness.py` fixture. The
[acceptance evidence map](completion-validation.md) records passing integration
commits and distinguishes local ownership from future remote dispatch.

### #82: delivery verification

`agent::gate_tests` covers clean and accounted-incomplete single-request acceptance,
real completion-tool reconciliation, ignored updates, fixed deadlines through tool
turns and provider waits, provider failure, scoped child shutdown and unrelated-work
isolation, inherited child scope, canonical proposal retention, explicit checkpoint
acceptance, post-seal cancellation/failure, missing stores, and a fake compatibility
bridge process. Steering during reconciliation and content-free error diagnostics
have dedicated regressions.

Frontend status, saved-session annotations, managed Journal outcomes and native
HTTP reconciliation have additional integration coverage in the acceptance map;
gate unit tests alone do not prove those surfaces. Dedicated remote execution now
uses the managed scoped checkpoint path; its broader reconciliation acceptance
remains separate from the local fixture matrix. Run both focused suites with:

```sh
cargo test -p helm --lib agent::gate_tests --all-features
cargo test -p helm --lib completion:: --all-features
```

### #83: verification

Current unit coverage in `completion::tests` exercises:

| Contract | Deterministic evidence |
| --- | --- |
| Run isolation and legacy non-adoption | Empty run with unrelated real todo/agent records; distinct run identities |
| Honest dispositions | Full todo and agent status/disposition matrices; evidence/result/reason rejection |
| Hiding/deleting work | Archive re-review, missing record, reopened todo |
| Freshness | Changed evidence and late result, new membership, expected revisions |
| Descendants/follow-ups | Owned-parent requirement, nested adoption, stale/self-parent rejection |
| Bounds | 1,024 obligations, capped/zero diagnostics, retained review history and atomic capacity failure |
| Serialization | Round trip; malformed, missing, duplicate, oversized, future-version and invalid fingerprint rejection |
| Failure safety | Revision overflow and mismatched record identity rejected |

Run the focused tests with:

```sh
cargo test -p helm --lib completion:: --all-features
```

Runtime/provider integration, cross-store failure injection, barrier-controlled
ownership/finalization races, cross-process writers, local CLI/TUI outcomes and
offline reconciliation are covered by the integration suites. The reviewed
`3950fef` Linux release passed 713 workspace tests both normally and with two-CPU
affinity, all 19 system fixtures, evaluation-definition validation and packaging;
see [the dated evidence](completion-validation.md#managed-integration-and-bounded-local-verification-2026-09-05).
These results belong to that commit, not automatically to later changes.

Full #83 acceptance remains open for successful budgeted real-model evidence and
the full remote worker/Vessel reconciliation matrix. [Dedicated remote sessions](remote-sessions.md)
are implemented, with offline lifecycle evidence; that is not proof of complete
remote reconciliation or multi-Helm voyage behavior. The recorded local
live attempts retained failure/recovery evidence; they did not establish
successful evidence verification or reconciliation. Subscription authentication
and cutover remain separate unverified requirements. Final dependency changes
still need their own reviewed Linux checks. The operator waived required
macOS/Windows runs for cost; Linux evidence does not establish their behavior.
Unit tests, offline fixtures and evaluation manifest validation are not live
semantic evaluations. See the [remaining acceptance checklist](completion-validation.md#remaining-acceptance-and-merge-boundaries).

Frontend handoffs stop accepting new subagents, cancel active/queued children,
and await their tracked persistence/archival tasks before releasing the workspace
writer lease. A ten-second drain timeout refuses the handoff instead of bypassing
ownership. The offline `tests/system/completion_handoff.py` regression exercises a
live child, TUI-to-plain relaunch and a subsequent turn in the same saved session.

## Provisional output and saved outcomes

Root CLI/TUI runs install the completion gate. Plain stdout explicitly marks
`completion: provisional`, reconciliation and terminal completion states; streamed
text is emitted once. A one-shot incomplete outcome saves its session first and
returns an error exit status. Plain chat can continue with another turn. Only a
completed outcome advances automatic-title checkpoints or completed-run counters.

Ctrl-C during a one-shot or plain-chat run requests cooperative cancellation and
waits up to 15 seconds for recovery and owned cleanup. A cleanup timeout is an
unconfirmed interruption, never successful completion; previously checkpointed
history, partial output and usage remain available. Plain chat returns to its
prompt after an interrupted turn. Ctrl-C at that idle prompt exits without waiting
for another line of input. Input is read only while idle, so the prompt reader does
not consume approval input during execution.

Session `run_summaries` retain each run's phase, bounded detail, optional structured
readiness and canonical message range with content fingerprints. These are local
presentation annotations, never provider messages or authority. Earlier no-tool
assistant proposals remain labelled provisional; the final proposal is labelled
completed, incomplete or interrupted. Transcript rendering and Markdown export
preserve original text while displaying these classifications separately. A latest
run banner remains visible after resume even if no assistant output was saved.

A provisional/reconciling summary loads as interrupted after restart. Branches copy
historical classifications but clear execution ownership references. Clearing the
conversation removes its annotations. Compaction/recovery reanchors only exact
message sequences; missing or ambiguous sequences retain an unlinked historical
summary instead of attributing an old outcome to different text. Old sessions with
no summaries remain readable. `--no-save` still does not create a session file.
