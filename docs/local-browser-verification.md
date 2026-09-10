# Shared local browser verification

Scope: #234 with the #235 duplex transport, on Linux. Browser code was integrated
in an isolated worktree; unrelated staged UI/account edits and deliberately deleted
scripts/tests in the primary checkout were not included. This is source and offline
runtime evidence, not a production deployment or live-provider certificate.

## Actual complete frontend journey

An isolated HOME/XDG fixture ran built `helm`, `vessel` and independent `voyage`
processes, a synthetic Responses provider, and a local synthetic website. No personal
browser profile, clipboard, provider account or paid request was used.

The standalone browser frontend passed the complete journey in
`.local/browser-integration-18a_67ms`. The actual TUI passed it in
`.local/browser-integration-42nijvr4`, and the final expanded journey passed in
`.local/browser-integration-4qndcpk8`:

- Start with an already **suspended** voyage; F6 opens a dedicated private local
  Chromium companion. Explicit local sharing prepares the current owner without
  replacing the session UUID or creating an agent before a prompt.
- A remote browser tool inspects the page, receives an actual JPEG screenshot,
  clicks a locally approved element, and observes the resulting changed page.
  Native provider input contains pixels in `function_call_output.output` with its
  original call ID; canonical history retains the scoped image artifact.
- Pause before submitting the task, so initial pending reads have drained. New
  browser work arrives through durable metadata-only notifications on the existing
  socket; no browser command arguments or pixels are put in general events.
- Enter private mode and navigate to a page containing a unique synthetic marker.
  Keep the human controller alive beyond the renewal interval. A separate actual
  browser tool call is refused while private; the marker is absent from all
  provider requests. Private/human renewals do not accidentally fence the session.
- Linux socket inode inspection observes **one unchanged Helm–Vessel TCP socket**
  across commands, lease control, delayed work notifications and private mode.
- Closing the browser does not cancel or delete the remote voyage. A new ordinary
  turn completes afterward under its own admitted run ID.
- Actual TUI Ctrl+Q cleanup completes, and text entered before F6 remains an unsent
  local draft, absent from provider requests.
- Explicit cleanup reconciliation follows a newly suspended owner using original
  inner browser bindings, without another provider request or browser effect.

The fixture uses short private `/tmp/b234-v-*` Vessel directories because Linux
Unix-domain socket paths are bounded. Private evidence records their paths. A too-long
initial fixture path was refused correctly, not counted as a pass. Fixture read-only
snapshot observations have bounded retries for explicit SQLite contention; starts,
submissions and browser effects are not automatically replayed.

## Defects found by integration

The first complete journey exposed behavior that the isolated broker/helper probes
could not establish:

1. Suspended owners needed an explicit `PrepareBrowser` operation and the returned
   current incarnation before Offer. The helper is not bound to an old incarnation
   merely because it was opened while the voyage was suspended.
2. Independent browser checkpoints could collide with canonical conversation writes.
   Browser admission/result reads and command dispatch now yield and retry only
   SQLite's explicit Busy/Locked transaction failures, with a bounded retry budget.
   The original browser request/command identity is retained. Other errors and
   uncertain external effects are not retried. Waiter guards remain until successful
   checkpoint publication.
3. Request deadlines must be distinct from the currently renewed executor lease.
   Current lease/epoch checks remain mandatory; renewal does not change the original
   action deadline or authorize a stale epoch.
4. A local command timeout winning `Promise.race` is not observed cleanup. The helper
   reports local settlement separately, and Helm does not infer it from `ok:true`,
   an unresolved result or a returned timeout. A dedicated real-Chromium fault probe
   left the underlying command future pending and verified `cleanup_observed:false`,
   followed by separately observed explicit shutdown. Evidence:
   `.local/browser-timeout-proof-bHLvbE`.

The failed journey logs were retained privately; they are not counted as passing
coverage or automatically replayed under their old action identities.

## Scoped checks

- Locked development build of Helm, Vessel and Voyage passed.
- Strict no-dependency Clippy for Helm, Vessel and the protocol crate passed.
- Voyage Clippy identified the existing `ProviderStreamEvent::Completed(ModelResponse)`
  large-enum warning. New browser findings were fixed. The remaining Voyage checks
  passed with **only `clippy::large_enum_variant` explicitly excluded**. This is not
  an unqualified strict-Voyage-Clippy pass; the unrelated provider enum was not changed.
- Native multimodal checks: **10 passed**; Anthropic tool-image encoding: **1 passed**;
  authorized artifact hydration/projection and canonical-reference preservation:
  **1 passed**. These are synthetic encoders, not cloud image reasoning tests.
- The actual offline broker probe passed principal/executor binding, durable
  pre-effect obligations, exact identity conflicts, stale targets, access modes,
  authority changes during approval, cancellation/unknown cleanup, private late-result
  withholding, 4,100 private/human renewals, 150 additional requests and retained
  tombstones, lease expiry, restart and exact old-binding cleanup. Its temporary
  source is retained as local evidence, not a resurrected general suite.
- The final local adapter probe passed sandboxed Chromium, real interactive frames
  and keyboard input, origin/private-network policy, Host/Origin/CSRF/session checks,
  controller exclusion, local effect reviews, real local/remote-staged uploads reaching
  the website, separate download save/disclosure, capture/takeover races, run changes,
  bounded deadlines, no replay, private storage and live disk-budget handling.
  Latest evidence: `.local/helm-browser-probe-IhNChg`.
- The close-failure probe passed overlapping shutdown and rejected Chromium close:
  one retained close promise, no premature `closed:true`, and preserved lock/staging
  despite helper exit. Latest evidence: `.local/helm-browser-cleanup-JGnTdZ`.
- See [adapter evidence](../helm/browser/VERIFICATION.md) and
  [duplex evidence](duplex-verification.md) for independent adversarial checks,
  actual renderer sandbox observations, socket backpressure, private PTY traffic,
  grant revocation and reconnect behavior.

No deleted general scripts/tests were restored. No hosted CI, optimized release
build or installed-service upgrade was used to manufacture verification evidence.

## Remaining qualification boundaries

Native macOS/Windows behavior, arbitrary websites, real provider image quality,
hard OS filesystem quotas and the actual remote TLS/proxy deployment are not
established by these Linux fixtures. The current binary artifact contract refuses
zero-byte remote downloads while permitting local save. The dedicated profile does
not imply access to personal profiles or arbitrary desktop applications.

Publication and preservation of concurrent main edits are tracked in #234/#235.
A tested feature worktree alone is not a delivered main revision. Unknown resources
from an earlier interrupted operator run are not attested clean by a fresh fixture.
