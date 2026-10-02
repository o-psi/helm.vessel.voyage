# Ordinary Helm execution, workflow and goal journey preparation (#353)

This source cohort starts from clean `e1719c9afc34d3130c22fc33b304d8244842e493`
and addresses the current ordinary Helm UI paths in `execution.rs`,
`workflows.rs` and `goals.rs`. The coordinator's retained
`v103-public-delegation-sse-final/line-accounting.json` planning snapshot reports
217, 161 and 138 aggregate uncovered line units respectively. Its distinct zero
addresses are 214, 151 and 133; the differences are overlapping function groups.
These planning counts are not a measurement of this new source or an exclusion.
The workspace denominator and reachable-production objective remain unchanged.

The three new standard test modules contain **22 parent cases**: eight execution,
six workflow and eight goal journeys. Their matrices check multiple concrete
failure and action variants inside those parents. They use the existing owned
loopback WebSocket fixture and actual `Client`, `App` command dispatch, updates,
pending-command recovery and private receipt storage. Synthetic host replies are
explicitly scripted; no provider, executing Voyage, administrator process or
native desktop is started. Workflow and goal review rendering uses ratatui's
in-memory `TestBackend`.

| Surface | Concrete journey and acceptance oracle |
| --- | --- |
| Execution preparation | A human prepare command freezes three nonnil identities and the selected workspace before one wire request. Restoring the private file observes the same review without another prepare. Transition requires explicit `stop-source` and captures the current session/incarnation. |
| Execution uncertainty and controls | Approval carries the saved review/command/digest and fences another approval on an unavailable outcome. An exact check observes that same review. Cancel retires retained IDs; revoke retains its separately observable authorization state. Only unresolved transition metadata permits explicit reconciliation, with a new command and the frozen digest. |
| Execution refusal and persistence | Observation commands preserve the composer. Malformed syntax, expired or nonpending approvals, disconnected routes and unsafe/malformed private records refuse. Symlink, hardlink, nil and oversized records retain the original evidence. Receipt retention failure cannot replace the earlier IDs or authorize approval. |
| Workflow full journey | Public host inventory is followed by actual rendered/scrollable definition trust, typed default inputs, masked private entry, executing-host preview, actual preview review and exactly one submit. Private handoff precedes submit only when selected. The durable public envelope contains the selected digest, owner/revision, public values and private bundle identity; it never contains private values. |
| Workflow refusal and recovery | A refused private handoff or submit retains the original public identity. Recovery sends only Resolve with that same envelope, never repeats private handoff or submit, and settles only the exact command receipt. Host diagnostics containing a synthetic private sentinel do not enter the update, notice, composer or private receipt files. |
| Workflow review fences | Changed digest/scope/workflow ID, unsupported inventory, mismatched or missing private references, missing/oversized preview, stale voyage revision, active run, recovery, pending cleanup, oversized private bundle and unavailable receipt storage cannot submit. Selection/owner/disconnect/expiry changes discard private state and require acknowledgement before composer input resumes. |
| Goal public dispatch | Fresh owner observation precedes human confirmation for Set, Edit, limits, Pause, Resume and Clear. Wire payloads are checked against independent literal intent, the exact goal identity and voyage revision. Repeat confirmation cannot dispatch another mutation; a later unsent draft survives the exact receipt. |
| Goal freshness and recovery | A late owner answer cannot authorize a replaced review. Restricted scope, stale owner/snapshot, changed incarnation/goal revision, recovery, catalogue-only state, disconnect and cleanup refuse. Pause during a running cleanup changes only future continuation, without cancelling the run. Mismatched positive receipts remain pending and recover through exact Resolve only. Private storage refusal sends no mutation; status/scroll/cancel/quit preserve the objective, usage and composer. |

Execution receipt storage previously used the default host path even in unit
fixtures. The small `receipt_root()` refactor retains that same release path and
uses the established `account_test_support::root` override only under `cfg(test)`.
All new storage effects therefore stay inside the existing per-test temporary
directory; tests do not change HOME/XDG or access the operator's receipts. Other
production changes are test-module declarations only. No runtime policy, deadline,
protocol, execution owner or release version changes.

Standalone rustfmt and diff/source checks are the source agent's only validation.
Cargo, tests, builds, coverage and publication remain unrun for this cohort. The
coordinator must independently review and integrate this source with the other
authorized cohorts, then own the single coordinated source-final test/coverage
and release gate. Existing reports and artifacts must be preserved; no passing
result or coverage gain is inferred from source preparation.

These cases establish neither Root authority/enrollment/launch/cleanup nor live
model behavior, billing, native TTY consent, deployed HTTPS, macOS or Windows
operation. The existing loopback peer uses synthetic replies and its ordinary
Drop cancellation is not new observed native cleanup evidence. Durable scope
remains [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353), with release
follow-through tracked in [#375](https://github.com/o-psi/helm.vessel.voyage/issues/375).
