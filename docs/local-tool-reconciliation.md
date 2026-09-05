# Explicit reconciliation of interrupted local tools

Cancelling a managed run can leave a canonical assistant tool call with no result.
The tool may have stopped before returning, performed some effects, or finished
without a durable result. Helm preserves that uncertainty and refuses another
turn while unresolved calls remain. Ordinary Interrupted recovery does not create
results, retry tools, or infer that effects rolled back.

After actual cleanup or an explicit operator cleanup attestation, the local
managed executor can expose `Journal::reconcile_local_tools(&guard, &request)` as a
separately selected recovery action. `LocalReconcileRequest` contains exact
session/run/installation/principal identifiers and the expected session revision.
The caller must derive the local actor from verified installation authority; UUIDs
are attribution, not authentication. No remote endpoint or sharing grant is added.

First reconciliation requires the execution guard, a terminal latest run, its
completed durable cleanup obligation, and the original expected revision. It
appends one failed tool result for each unambiguously unresolved call, in call
order. The fixed text states that execution was interrupted, the outcome is
unknown, the tool was not retried, and the record establishes neither success nor
rollback. It never launches a provider or tool. The existing canonical prefix,
provider continuation envelopes, usage, provisional partial text, and immutable
terminal classification are preserved. Existing actual tool results remain intact.

The helper refuses empty, control-containing, oversized, or repeated call/result
identifiers and histories with later conversation interleaved after unresolved
calls. Refusal leaves the history unchanged; it is not permission to delete or
rewrite evidence. A reconciliation contains at most 128 unresolved calls, each ID
at most 1024 bytes, within the existing bounded canonical snapshot. The retained
receipt is bounded to 256 KiB and at most one per retained run.

Canonical append, revision increment, and local audit receipt commit in one
SQLite writer transaction. The receipt retains the exact operator actor, selected
run/session, original expected revision, resulting revision, and reconciled IDs.
`LocalReconcileOutcome` returns those IDs, the resulting revision, and whether this
was a duplicate observation. An exact retry observes the immutable receipt before
checking the latest run or current revision, so a lost response remains recoverable
after reopen or later turns. Never replace the original revision when retrying.
Changed scope, actor, or original revision fails. Busy or failed writes cannot
claim durable reconciliation; no partial append or receipt survives rollback.

Schema 6 adds the receipt table through explicit quiescent migration. Schema 5
remains usable for its catalogue, cancellation, and cleanup APIs until upgrade;
reconciliation itself requires 6. Stop old processes, recover active runs, and
release session owners before upgrading. A held execution fence or active run
blocks migration, and older open connections fail after version change. Do not
silently downgrade or remove receipts to run an older binary.

This is a local session edit, not a resumed run. It changes the snapshot/list
revision without emitting activity after a run's terminal event. A future remote
adapter needs an authorized session-change notification or snapshot contract before
exposing this operation remotely; the terminal replay stream alone cannot announce
this change. The dedicated remote worker currently exposes recovery only through
its explicit local command. Broader coordinator/sharing work in issue 78 remains
open; a planned [voyage](voyages.md) handoff must not reinterpret reconciliation
as successful execution or automatically repeat uncertain work.

The companion frontend documents its explicit recovery flags and prerequisite
cleanup selection. Library tests cover malformed and ambiguous histories, bounds,
wrong actor/guard/revision, missing cleanup, active/stale run selection, persistence,
SQLite contention and injected write failures, immutable retries, migration, and
real HTTP continuation through native Chat Completions, Anthropic, and Responses
adapters. Those deterministic fixtures are not live-provider or OS-containment
validation.
