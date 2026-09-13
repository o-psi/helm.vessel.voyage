# #273: first Chromium TUI failure — retained-evidence triage

Scope: `browser-integration-hnkd1f6n`, not the subsequent passing fixtures.
Read-only investigation; no Cargo, new browser launch, command replay, receipt
mutation, or cleanup attestation. Parent owns publication and any Rust changes.

## Finding

**The immediate failure was loss of the local companion listener after the
consented click completed, not a failed click. The trigger that ended the browser
runner is not retained.** This remains a diagnostic blocker, not a proven fixture
timeout, owner death, or fixed runtime bug. Later independent passes do not resolve
it. No reproduction is active; the parent may rebuild its binaries.

Evidence lives under worktree `target/ux273-browser/`. The original runtime root
is recorded verbatim by the fixture's `vessel-path.txt` (ending `target/b-12cuq613`).
Session: `df739791-a156-4209-b4b0-7c2928c78c4e`; resumed incarnation:
`5c4e8114-259e-4c79-9441-48778bc189d4`; tool run:
`87faed29-205b-4ae9-a850-9b17a45123f1`.

## What the retained records establish

- `transport-tui-v2.log` reports successful suspended-start and explicit-share
  steps, then `ConnectionRefusedError` from companion `/api/status` polling in
  the consent loop. It does **not** end in the fixture's `wait expired` assertion.
- `browser.log` shows the local-confirmation prompt followed by `Conversation
  unavailable` and the generic unresolved-cleanup/effects warning. It does not
  retain an underlying Rust error or process exit cause. `vessel.log` is empty.
- `errors.json=[]` is only the synthetic provider assertion list; it is not an
  application error log or transport health result.
- The private helper receipts record **completed** for inspect
  `6325794a-1f95-42f7-954d-571bd549f436`, screenshot
  `a2ac7901-df88-400f-9717-baaac3eba326`, and click
  `f57db84b-4134-4977-8430-137574c67964`.
- More importantly, the retained Voyage SQLite `browser_state.commands` contains
  remote completed receipts with `cleanup_pending=false` for all three exact
  request IDs. `browser_state.entries` is empty. A retained disconnect reply
  reports `control=disconnected`, `available=false`, `cleanup_pending=false`.
  Thus this conclusion does not rely solely on local helper exit or local receipts.
- Canonical `sessions.state` contains a successful click tool result timestamped
  **2026-09-13 22:10:03.622013618 UTC**, with `effect=completed` and
  `requires_observation=true`. It contains no successful post-click inspection.
  The click must not be replayed to compensate for the failed journey.
- Local `cleanup.json` says `observed=true`; its filesystem mtime is
  **22:10:03.815537 UTC**. This is an approximate local chronology, not an event
  timestamp. The retained `stopped.json` mtime is **22:10:06.437863 UTC** and
  both guardian records say `cleanup_observed=true`.
- The tool run is retained as **cancelled**, not completed, with
  `terminal_reason="run cancelled"` and `final_checkpointed=false`. Lifecycle
  records contain an explicit stop for the resumed incarnation, consistent with
  the probe's finally-block teardown. Final stopped/guardian records cannot prove
  an earlier unexpected owner death or establish its absence.
- Read-only SQLite `PRAGMA quick_check` on the journal returned `ok`. This does
  not exclude transient contention or another runtime error.

## Why the original trigger cannot be recovered from these artifacts

`helm/src/process_client/browser/runner.rs` can leave its loop on helper status,
lease renewal, pending operation, action result, or socket errors. Afterward it
attempts disconnect and local shutdown even when the loop returned an error.
Consequently observed helper cleanup and companion refusal can follow an error
while Helm, Vessel and Voyage are still alive. They do not identify that error.

`browser/mod.rs` converts a failed runner into generic status text, and
`ui/browser.rs` converts a failed shutdown/join result into the generic unresolved
warning rather than retaining its cause. That warning is not a receipt audit.
`ui/render.rs` renders `Conversation unavailable` for a view error without
connection-unavailable status; this label alone is not evidence of process death.

The probe did not preserve a pre-teardown process-status snapshot or the original
runner error. Finally-block process stops confound postmortem exit evidence.
There is no affirmative retained timeout, panic, unexpected signal, or owner-death
record permitting a narrower causal attribution. In particular, remote completion
of the click must not be confused with completion of the whole consent journey.

## Handoff

Retain this first run as **failed, initiating cause unknown**. Its three dispatched
actions have affirmative completed remote receipts; do not describe them as
unresolved solely because of the generic TUI warning. Post-click observation and
subsequent journey gates were not verified in this fixture.

A productive next diagnostic step requires parent-owned Rust observability:
retain a sanitized structured browser-runner failure category before replacing it
with generic presentation, with socket/process liveness and cleanup results kept
separate. Any future independent synthetic fixture should also snapshot its owned
process statuses before finally-block teardown and retain its exact timeout/error
stage. Do not expose companion tokens or raw subprocess diagnostics in the TUI.
No narrowly proven transport-probe cause was found, so this investigation makes
no harness or Rust fix and does not launch another likely non-diagnostic pass.
