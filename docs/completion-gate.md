# Completion readiness and reconciliation

Tracking: [#81](https://github.com/o-psi/voyage/issues/81),
[#82](https://github.com/o-psi/voyage/issues/82), and
[#83](https://github.com/o-psi/voyage/issues/83), under
[#5](https://github.com/o-psi/voyage/issues/5).

## Current delivery status

**The completion gate is not enabled.** `helm/src/completion.rs` implements the
provider-neutral readiness contract and its deterministic tests. Runtime ownership
propagation, durable storage integration, reconciliation tool operations, and the
final-response interceptor still need implementation. The module is exported for
integration but is not called by the agent loop. It does not add a model request,
change streaming, or claim that existing runs are being checked.

This staged delivery avoids implementing a workspace-global cleanup check that
could interfere with unrelated work. It also avoids using a final snapshot without
an atomic acceptance boundary: two consecutive reads alone do not eliminate races.

## Readiness contract implemented so far

- Each `RunLedger` has a distinct `RunId`, unrelated to session IDs, workspace paths,
  or individual execution IDs. Membership is explicit and append-only.
- Trusted callers register created/adopted todos and agents. `adopt_child` requires
  an already-owned parent; the spawn coordinator must verify the actual relationship
  and register descendants and terminal follow-ups before publishing execution.
  Listing, assigning, resuming, or loading legacy work does not adopt it.
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

`to_json`/`from_json` provide versioned serialization, **not disk persistence**.
Malformed, oversized, duplicate-membership, missing-field, and future-version data
are rejected. Existing todo/agent/session formats are unchanged. The integration
must not treat an absent or corrupt ledger for a known run as a new empty ledger.
Legacy sessions without a run reference must not implicitly adopt historical work.

## Remaining integration requirements

### #81: ownership and persistence

1. Propagate trusted run identity from root invocation through tool context, child
   spawn/executor context, nested children, and follow-up semantics. Preserve fresh
   per-execution IDs; they serve a different purpose.
2. Persist membership before work becomes visible/active. Serialize ownership,
   record mutation, result publication, and acceptance at a shared coordinator
   boundary, including other processes that can write the same stores.
3. Add explicit, policy-controlled adoption and disposition operations with fresh
   record reads, durable history, and session/run scope validation. Record text and
   disposition reasons must use existing redaction paths before persistence/output.
4. Use private atomic persistence, expected revisions, honest crash recovery and
   failure handling. Do not downgrade to success on a store error or missing record.
5. Provide bounded retrieval of owned results/evidence for the targeted prompt.
   The current diagnostic snapshot deliberately does not embed those contents.

### #82: final-response gate

Intercept the runtime's actual completion proposal, after pending supervisor input
is processed. Clean runs need no additional request. Otherwise, hold acceptance,
send one targeted reconciliation update, permit normal policy-controlled work,
and recheck fresh state within a bounded cleanup wall time. Repeated final proposals
must not reset the bound or reintroduce the removed general model-turn cutoff.

Define explicit incomplete/interrupted outcomes when reconciliation cannot finish;
account for failed children instead of demanding every todo be completed. Cancellation,
provider failure, approval denial, and exhausted budgets must not trigger unbounded
cleanup or additional requests after cancellation. Bound and observe shutdown of
owned active children on abort. Do not close unrelated persistent terminals or
perform automatic Git/worktree integration/deletion.

Streaming is provisional until acceptance. TUI, plain output, saved history, and
Vessel worker outcomes need consistent status without duplicate finals or synthetic
human turns. Runtime guidance must not be saved as user-authored conversation.

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

Still required: runtime/provider integration and real persistence failure injection;
barrier-controlled spawn/result/edit/finalization races; cross-process writer tests;
CLI/TUI/Vessel outcome and provisional-output coverage; offline reconciliation E2E;
adversarial behavioral evaluations; approved/budgeted real-provider smoke; and
Linux/macOS/Windows delivery evidence. Unit serialization tests are not disk recovery
or E2E evidence, and evaluation manifest validation is not a live evaluation.
