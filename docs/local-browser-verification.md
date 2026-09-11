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
`.local/browser-integration-4qndcpk8`. After integrating the latest capacity and
Settled-status changes, the same full journey passed again in
`.local/browser-integration-cw3kyqwf`:

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

## Final workspace coverage

The clean integrated source `b19f0a93aa3f13272e505645c0dccf446c3428c5`
completed workspace Rust coverage with **116 passed, 0 failed, 1 ignored**.
All tracked input bytes were unchanged during measurement. Published counts are in
[coverage/latest.json](../coverage/latest.json): lines **12,319 / 72,710 (16.94%)**,
functions **1,207 / 7,372 (16.37%)**, regions **19,127 / 118,103 (16.20%)**.
These exclude the separately passing Python/JavaScript/TUI journeys.

The default `--no-clean` report scanned historical binaries from other checkouts in
the shared target, doubled source totals and reported mismatches. That report is
retained but **not** the published measurement. The corrected export uses all
11 current workspace executable artifacts identified by Cargo, including all 10
executed test binaries, with the tool's default filename exclusions. It reports
386 unique logical source files and no mismatched functions. Twenty-one reused
installer source paths were verified byte-identical to the measured checkout; they
were retained rather than excluded. No current package/target was omitted.

Raw/default and corrected reports, current-artifact inventory, source fingerprint,
logs and HTML remain under `target/coverage-report/browser-final-current`. Prior
reports and compiled caches were preserved. [#251](https://github.com/o-psi/voyage/issues/251)
tracks making this shared-target selection routine in the general coverage workflow.
The prior coverage record used a dirty tree containing unrelated work; differences
are recorded, not advertised as a controlled coverage improvement.
