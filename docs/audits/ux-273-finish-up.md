# #272/#273 finish-up delivery

This continues the [initial delivery](ux-272-delivery.md), separating completed
local acceptance from genuine remaining limitations. No live provider, personal
browser profile or native-platform claims are made.

## Fixes found by executed journeys

- **Reasoning finalization:** the deliberate live-to-final key change collapsed
  disclosure but lost its keyboard anchor. Reconciliation now migrates that anchor
  to the final header without transferring expansion. A subsequent explicit toggle
  survives further snapshots. Focused regression covers Summary and Thinking,
  unrelated anchors and repeated observations. The real Anthropic PTY rerun now
  expands/collapses at the same anchor after finalization.
- **Delete on fresh journals:** scrubbing unconditionally deleted from retired
  `remote_events`, which fresh independent Voyage journals do not create. Delete
  now scrubs it only when present, preserving real SQL errors and not recreating
  retired infrastructure. Fresh/legacy tests verify history deletion, legacy scrub
  and exact duplicate receipt identity. The actual lifecycle journey subsequently
  completed Delete with the selected peer's history unchanged.
- **Approval regression completeness:** worker attribution preserves all six gate
  outcomes, request/execution identity and cancellation. Typed denial/expiry/
  invalidation/unavailability/cancellation and fixed text survive canonical message
  serialization. No runtime authority or approval policy was broadened.

## Observed acceptance

| Surface | Actual evidence |
|---|---|
| X02 images | Four owned images, fifth-image refusal, acquisition failure/cancel and helper reap, authored draft retention, one four-image provider submission; final fresh v4 image rerun passed |
| X03 steering | Image steering refused with draft retained; one text steering admission followed by canonical delivered status at safe boundary |
| X09 routing | Same-title voyages on two scoped loopback Vessel gateways retain route-qualified identity and separate drafts |
| X10 lifecycle | Archive/restore exact target; Clear/Delete cancel and stale-review refusal; confirmed Clear and post-fix Delete; unchanged peer history and observed cleanup |
| X13/X15 | Three grouped private setup, pairing/operator and workflow cases passed; masked/cancelled setup, workflow trust/input/focus-loss, stale handle refusal, real secret-bound shell with suppressed stdout/stderr, no synthetic secret in public/persisted/PTY captures |
| X14 helper | Real sandboxed Chromium share dismissal, deny/allow, takeover/private, heartbeat/companion disconnect, explicit reshare and synthetic close-failure/overlap checks passed |
| X14 transport | Real CLI and F6 TUI Helm→Vessel→Voyage→local Chromium inspection/screenshot/click, provider image pixels, local consent, private-content withholding, stable duplex socket, close and suspended-owner reconciliation passed; exact process/listener/lock cleanup observed |
| #255 | Nine local/scoped and extended streamed cases passed, including malformed final arguments, cancellation, explicit provider failure, split configured secret redaction, Anthropic thinking/opaque exclusion and post-fix final keyboard anchor |

Evidence detail and limitations:
[remaining UI](ux-273-remaining-ui.md),
[workflow/private setup](ux-273-workflow-private.md),
[browser verification](../../helm/browser/VERIFICATION.md), and
[streaming verification](../streaming255-verification.md).
Raw synthetic artifacts remain under ignored `target/ux273` and private fixture
roots. Completed worktree evidence is archived under
`target/ux273/workstream-evidence`; source scripts and compact reports are published.

Final workspace Rust coverage after the last relevant edit: **469 passed, 0
failed, 1 ignored** (live-provider test). Lines **35,750/96,160 = 37.1776%**;
functions **3,336/9,391 = 35.5234%**; regions **56,724/157,136 = 36.0987%**.
Versus the initial delivery: +0.2263/+0.1365/+0.2105 percentage points. All 11
current Cargo executable artifacts and 447 logical source files retained; no
mismatched functions or duplicate source totals. `coverage/latest.json` records
exact command, dirty source fingerprint, versions, scope and exclusions. Default
shared-cache report diagnostics remain retained; corrected current-artifact totals
are published. Python/Node syntax, documentation links/shell syntax and diffs pass.

## Remaining, not relabeled as passing

- [#274](https://github.com/o-psi/voyage/issues/274): an early Chromium TUI fixture
  disconnected after a completed click. Journal inspection confirms all three
  dispatched actions completed and cleanup was observed; it does not establish
  the initiating failure cause. See [first-failure triage](ux-273-browser-first-failure.md).
  One v4 image run safely refused upload before Submit with the full draft retained;
  a fresh same-binary rerun passed. Generic diagnostics cannot identify its cause.
  These failed runs are retained; later passes are not claimed to fix intermittency.
- [Pinned competitors](ux-273-competitors.md): locked Pi dependencies install, but
  the pinned source lacks generated provider model data; OpenCode lacks its declared
  Bun runtime/bundle and Codex its matching native payload. No complete competitor
  interactive comparison or ranking follows. Public catalogue hydration would
  change the comparison's data provenance and was not silently substituted.
- Native macOS/Windows, paid provider/authentication/model entitlement, separately
  hosted/TLS deployment, actual unkillable-browser recovery, and exhaustive private
  enrollment interruption variants remain unverified. #273 and #272 retain those
  acceptance obligations; #255 retains its expressly scoped residual matrix.
- Existing deleted release scripts/manifest and unrelated staged render comments
  remain untouched. No installer packaging or release publication is claimed.

This delivery finishes the feasible local implementation fixes and substantially
extends executable acceptance; it does not close broad issues by moving unknowns
into an assumed success state.
