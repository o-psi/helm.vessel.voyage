# Remaining #333 production qualification

This is a source-derived execution plan and unexecuted observation tooling.
The completed current-archive four-phase synthetic evidence is in
[the v1.0.3 record](host-browser-v1.0.3.md). Do not repeat that full cohort merely
to collect production evidence, or call these remaining gates passed.

## Actual paths and prerequisites

| Path | Maintained implementation | Required private input |
| --- | --- | --- |
| Native TUI | `helm connect --access-file ... --no-start`, actual F6; `helm/src/process_client/host_browser.rs`; shared viewer | Existing public paired credential and selected fixture Voyage. Its grant must permit the actual observe/execute/history operations. |
| Deployed React dock | `resources/react/App.tsx`, `HostBrowser.tsx`, `workspace.ts`; `resources/js/host-browser.js` | Existing authenticated operator tenant/session and saved Vessel connection. |
| Browser bootstrap | `ConsoleAuthController::ticket`, `ConsoleAccess::ticket`, `VesselGateway::browser-credentials`, `resources/js/vessel-fleet.js` | Same-origin CSRF POST `/console/ticket`; valid stored grant, pinned Vessel identity and tenant entitlement. |
| Actual Web transport | Origin-bound 120-second credential, `voyage.vessel.v1`, `/v1/vessel/browser-socket` | Real configured Web Origin and normal certificate trust; renewal must remain observable. |
| Native transport | Same pinned public HTTPS origin, `/v1/vessel/socket`; private localhost viewer bootstrap | Original authenticated socket/owner fences. Keep the launcher secret private and one-use. |

The present owner-related connection refusal is a real admission blocker. Require
an authenticated capabilities/snapshot observation for the selected fixture
Voyages through the deployed route before browser effects. Public `/up` and live
service PIDs do not substitute. Do not fix refusal by broadening authority,
changing tenant identity, disabling Origin checks or copying secrets into logs.
Native and Web should have independently authorized grants for the same human
principal when qualifying private disconnect/reclaim. Distinct grant and
participant surfaces retain their existing meanings.

Record exact executable hashes/proc start identities on the executing host,
deployed Web source/manifest, public origin and actual Web/native socket paths.
Use the executing host's installed Node, sandboxed Chromium, worker/guardian
assets and private launch configuration. No build or dependency download belongs
inside this journey.

## Smallest provider-free execution pass

1. Create two explicitly named, ordinary independent fixture Voyages using the
   existing reviewed StartConfigured/private LaunchConfig path. Enroll a
   synthetic host-local account whose endpoint is an owned loopback fixture or
   a closed local port. Do not Submit a prompt or authorize Goal continuation.
   Open each browser through its client UI; browser owner preparation does not
   require inference. Record zero provider requests and no unexpected runs.
2. Use actual TUI F6 and the deployed React Browser dock, without replacing
   WebSocket, redirecting socket destinations or mocking `/console/ticket`.
   Two authenticated Web tabs plus two native viewers allow simultaneous
   two-Voyage/four-viewer measurements; switching one React tab disposes its old
   viewer and is not four simultaneous viewers. Verify exact selected Voyage/
   browser/controller state and the current shared page through both paths.
3. Serve the existing owned fixture site classes through an approved public
   HTTPS test origin: dynamic DOM, external CSS, cookie-gated image, open shadow,
   separate-origin child/nested frames, scroll/tab/dialog/file controls and
   synthetic private sign-in. Keep host/network policy unchanged. Hosting a
   temporary fixture or adding a test prefix is a separate coordinated host
   action; no such production change has been performed by this source batch.
4. Add a bounded canvas and short local video, plus an intentionally unsupported
   frame/closed-shadow case. Require localized visual fallback without promising
   smooth video or arbitrary unsupported-frame interaction. Then qualify approved
   representative public URLs for static content, a dynamic application and a
   frame/media class with benign input. Pin exact URLs/date/expected behavior;
   report unsupported classes. Never use paid providers or consequential site
   actions as a substitute for the owned fixture.
5. Private takeover must exclude the other actual client and model/history,
   disconnect must remain private, and explicit same-principal reclaim/return
   must use fresh references. Inspect only fixed booleans/counters during private
   input; retain screenshots before private entry or after explicit return.
6. Observe real ticket renewal/reconnect without effect replay, then close only
   owned test browsers/Voyages. Keep the unrelated production services and Voyages
   outside cleanup. Overall success requires exact observed browser process/
   scratch/slot cleanup and durable uncertainty preservation; an operator
   attestation or requested cancellation is separate evidence.

The existing `goals.py`/`goals_web.mjs` fixture is useful for built React selectors
and real public protocol sequencing, but uses self-signed loopback TLS,
`ignoreHTTPSErrors` and socket destination rewriting. It does not qualify deployed
TLS. `browser-layout-browser.mjs` and `browser-next-browser.mjs` use synthetic
transport. `helm/browser/transport-probe.py` belongs to the opt-in local browser,
not this host-owned transport. The historical `direct-wss307-benchmark.md` measures
relay/mock sockets and points to retired tooling; do not restore it or claim it
measures browser replay cost. No current host-browser cost runner exists.

## Bounded cost matrix and facts

Use one consistent viewport/site/action cadence and record machine/tool/source
identity. After warm-up, measure three 10-second windows for each condition:
one Voyage/one viewer; one Voyage/native+Web viewers; two Voyages/two native+two
Web viewers. Also observe a no-viewer interval for the owned browsers' capture
cost. Do not restart the retired encoder to create a new baseline. Retain the
historical single-viewer comparison and explicitly limit cross-run comparisons.

| Fact | Collection | Interpretation |
| --- | --- | --- |
| Host CPU/RSS/PSS | `voyage/tests/host_browser_cost.py` on the executing host with explicit PID/starttime ledger | Observe each fixture Voyage/worker/browser tree; gateway/supervisor leaf PID separately. Never include unrelated production descendant trees. RSS sums double-count shared pages; PSS apportions them. Missing permissions are unavailable, not zero. |
| Client renderer CPU/heap/DOM | `host_browser_client_cost.mjs` Performance metrics on each qualification-owned Chromium page | Task/Script/Layout durations and JS heap/DOM counts; renderer task time is not whole-browser OS CPU. Measure the isolated qualification browser process tree with the PID ledger for OS CPU/RSS/PSS. |
| Web payload | Passive CDP counters for the exact actual browser-socket URL | Application frame bytes including auth/renewal, without retaining contents; excludes TLS/TCP framing/compression overhead. Attach before socket creation/renewal; zero selected connections seen means unqualified traffic, not zero cost. |
| Native viewer payload | Passive CDP counters for that owned localhost `/operation` URL | CDP encoded HTTP response bytes. Native Helm's public WSS traffic is outside the viewer page; these bytes must not be represented as its wire cost. Actual native wire bytes require an already-authorized per-socket/proxy counter or a dedicated owned traffic fixture. |
| Latency/fidelity | One explicit benign input and observed replay marker in both clients | Record sample count/median/p95/max plus action/refusal/recovery counts and unsupported classes. Time only the single confirmed intent; never retry an unknown effect to get a number. |

The passive client helper requests no response bodies, parses no private messages
and records no tokens, IDs, URLs, headers or input content. It does not navigate,
log in, create effects, force GC or change timestamps. Import it into an authorized
qualification driver before actual socket establishment. A fresh controlled
browser needs a private authenticated session/profile; do not assume ambient app
login can be exported or reused. Existing Web heap fixtures show public CDP
Performance/heap methods, but their synthetic claims remain narrower.

On the measured Linux host, use a private ledger created from exact owned process
observations, then run this observer separately from application cgroups:

```sh
python3 voyage/tests/host_browser_cost.py \
  --ledger /private/qualification-owned-pids.json \
  --seconds 10 --interval 0.25 \
  --output /private/new-host-cost-summary.json
```

Ledger schema is `{schema:1, roots:[{label,pid,start_ticks,descendants}]}`. The
observer validates root start identity each sample, bounds process/depth/sample
counts and never signals or launches a process. Its published summary excludes
PID/path/command/credential data. Observed CPU deltas can miss short-lived
descendants between samples; declare that limitation rather than claiming totals.
Use distinct output paths to preserve earlier measurements.

## Closure criteria and current state

| Gate | Current state | Required evidence |
| --- | --- | --- |
| Authenticated deployed admission | Blocked by actual owner refusal under diagnosis | Real tenant/grant/Vessel identity admitted; no authority changes to manufacture success. |
| Both actual clients through deployed TLS | Pending | Normal certificate trust, deployed React dock + TUI, real operations/replay/control/private/renewal and exact cleanup. |
| Representative site/media fidelity | Pending | Owned public fixture plus pinned real classes, screenshots/markers and honest unsupported cases. |
| Multiple-viewer/Voyage costs | Pending | Matrix above with exact host/client/payload/latency scope; missing native wire counters explicitly unresolved. |

These helpers and this plan are unexecuted source. They do not close #333 or
replace the passing synthetic four-phase record. The qualified
`host_browser.py`/`host_browser_viewer.mjs` have not been changed by this batch.
