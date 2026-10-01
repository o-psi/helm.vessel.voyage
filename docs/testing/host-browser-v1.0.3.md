# v1.0.3 browser qualification

The current Linux x86-64 archive passed the expanded four-phase synthetic
journey on 2026-10-01. Historical qualification remains below. These checks
qualify the actual scoped Linux/native viewer and matching Web adapter; full
deployed Vessel TLS/WSS, representative public-site and client-cost acceptance
remain open in #333.

The remaining production route is prepared in
[the separate executable qualification guide](host-browser-production-qualification.md).
It supports two actual TUI launchers and two authenticated React dock viewers,
including CUA/authorized-host orchestration without login export. That new driver
and its measurement/site helpers are source-only and unexecuted; they do not add
passing evidence to this record.

## Current archive and observed four-phase result

[Hosted build 36926772323](https://github.com/o-psi/helm.vessel.voyage/actions/runs/36926772323)
published `1.0.3-nightly.20261001.36926772323.1` from actual checked-out source
`76c26aabefa94094c6fec94bc7f29f30589eb5f2`. Its `BUILD.txt` identifies that
source/version and Linux x86-64 scope. This is a development prerelease, not
stable publication. The coordinator verified four binaries, 123 browser assets,
archive/checksum identity and retained worker assets after isolated upgrade.

The final process journey used native executables from that unpacked archive:

| Executable | SHA256 |
| --- | --- |
| Helm | `c2b2333e07238f0e628cedda45d84b5d2779d5a5145eefeb09e1dd7db116bb88` |
| Vessel | `5fc9ce10d4cf854b299803ded5f502625fd8cbebfde630feded2d9bd08e606db` |
| Voyage | `3b70ce659f9bfa2c0ce52d7aef1a6acb727b98a23ac551ff1ddaa533e67c3084` |

Maintained harness source was `dc02f1302650cc7c8a564416a8701e0e2fe906b7`.
Worker SHA256 was `017312f8a3bb81241e0529773e90e45819fe6844ef4aeaede98b611743c60831`;
shared viewer SHA256 was `aed3ac6f562bed0e7bee580e8bcafb019ed426bdfe213eec830bf1adbb247e43`.
Those production files matched current core `76c26aa`. Matching Web source
`5fc8502a4932ced7cf3fe5bcb0703aaf9f7e7205` is deployed, and its shared viewer
matched the core copy byte for byte. The journey used its checked-out production
adapter over real authenticated loopback Vessel sockets, not its deployed React
dock or production TLS connection.

Unit `voyage-v103-browser-current76` exited zero after 2 min 25.939 sec, with
1.7 GiB peak memory and zero swap under the bounded user-service limits. Run
identity `host-browser333-_y6mv3ke` passed all four phases: initial agent/native/
Web viewing, actual suspended native preparation, suspended Web preparation,
and browser adversity/cleanup. Two real independent Voyages made 38 scripted
loopback provider exchanges with zero fixture errors and no paid provider usage.
No extra inference was requested by viewer preparation or reconnection.

The passing source assertions covered:

- Actual scoped TUI F6 launch and native local-owner/access-file routes; the
  matching Web adapter attached to the same browser, excluded private streams,
  preserved private mode on disconnect and explicitly reclaimed that private
  controller without publishing human mode.
- Replayed element input, history/title, modal/IME, external CSS, cookie-gated
  image, open shadow DOM and private-pipe cross-origin frame control. Local
  input-to-visible samples were 394/363 ms, not performance guarantees.
- Native and Web file-picker uploads and downloads with exact synthetic bytes,
  one-use delivery, other-attachment/Voyage exclusion and canonical-history
  privacy. Scroll and New/select/close tab controls passed last-tab protection.
  Old tab/document and viewport fences refused input; a fresh element acted once.
- Fifth-viewer admission refusal with visible recovery and no browser restart or
  input. This qualifies viewer capacity, not host browser-slot exhaustion.
- A held confirmed mirror reply exercised the unchanged 25-second client timeout.
  The native controller and separate Voyage progressed; one held reply peaked
  in the gate. Explicit read recovery rendered the actual counter1, with no
  observer input replay. This is client stalled-read qualification, not TCP
  browser socket backpressure.
- An identity-checked worker SIGKILL produced actual guardian cleanup. With that
  exact evidence temporarily withheld, explicit close stayed unknown and retained
  one capacity slot and the original crash lock. Restoring the unchanged record
  enabled a new explicit cleanup decision: zero owned live/zombie processes,
  removed scratch, observed guardian cleanup and zero retained slots. The lock
  stayed byte-identical and external actions remained unreconciled.
- One explicit replacement-browser admission was refused without a new worker,
  slot or automatic retry. The first close receipt remained unknown. The late
  original-owner receipt read received a definite owner-unavailable refusal;
  no owner was prepared or command retried. A read-only, bounded inspection of
  the fixture's exact private SQLite row proved the command/principal/digest/
  unknown state unchanged. This does not establish a public saved-receipt API
  through a suspended owner.

Overall success was recorded only after no remaining owned PIDs, no forced
teardown and both fixture HTTP threads stopped. Logs and pre-private screenshots
are retained under ignored `target/verification-v103/current-76-native`.
Sanitized phase results and private reports retain these fingerprints:

| Report | SHA256 |
| --- | --- |
| Overall | `20a5411efc685d4951132c144061dd23ed61f9afeee8de4eb1452ee8b091bd1a` |
| Initial viewer | `53b6a5e6102e23e5046fd05ec2ea7f2cb8995371cfa9e04a5d4b1dada7eaa693` |
| Suspended native | `a89c433b60dd40f9d492d673e26a1cde492d12fedc692656eee309441354fc51` |
| Suspended Web | `43f9bfdd3a1f87379dfb85d636b315d42421b0aac42607ec6817e7e195b420fa` |
| Adverse | `13eb9ecda00e03f8bd67d644f71023abf5e3f869530536224703144294307b65` |

The preceding full run `host-browser333-n2hshtle` also passed the final `dc02f13`
harness, all four phases and observed cleanup with 38 synthetic exchanges. It
used earlier executables identified by hashes in its report, not the current
76 archive; its 409/246 ms samples do not establish current-runtime performance.
Earlier failed attempts remain failed and retained. The repairs addressed
confirmed tab/input sequencing, visible inline geometry, historical live replay,
recovery accessible naming and legitimate original-owner receipt refusal; they
did not weaken authority, cleanup or uncertain-effect handling.

## Remaining full acceptance

Production Web deployment/public asset identity and its ordinary suite are
separate passing records. The current loopback journey does not prove browser
operation through the actual deployed React dock and Vessel TLS/WSS proxy.
Gateway command-line compatibility recovery and authenticated owner admission
are being qualified by the coordinator; current public health/service readiness
does not establish browser operation or a completed managed update. The observed
owner-related refusal remains under diagnosis without changing authority checks.
Representative real-site/media/unsupported-frame classes and multiple-viewer/
Voyage client CPU/heap, host RSS, bandwidth and input-visible measurements remain
required. Retain the earlier one-viewer transport baseline without claiming it
includes client replay costs or restoring the retired encoder.

No universal website compatibility, native macOS/Windows, live-provider security,
public Root transport, socket-level browser backpressure, or cleanup of live
unreaped descendants is inferred. Required native installer/system evidence is
owned separately by the release coordinator. #333 remains open for its remaining
acceptance, even though this complete synthetic cohort passed.

## Historical candidate inputs

The earlier optimized candidate used core source
`31dd1150b43e716190d627ce2d556f4621881057` and Helm Web source
`c13a8c5a7e4f468eafca31e13c3ccf3c37215ebd`. The following inputs/results belong
to that qualification, rather than the current archive above.

The three executables came from the unpacked candidate archive. Their digests
matched both `release.json` and the corresponding optimized build outputs:

| Executable | SHA256 |
| --- | --- |
| Helm | `6c68c2fb796500afa6ed571dc0c1b5f8b2c122cf633e59ffa6a7c7811009cd76` |
| Vessel | `48505d93bf2e2c3d5666bf5980abbed369e3e3929278a1599cc05f6e0a139db9` |
| Voyage | `bde5a11b73ebd7c4a89758e1e304cd3ca3716e3e67de92665fb97b5a45e18792` |

The explicitly selected checkout worker matched the packaged worker, SHA256
`199493c6545db715acfbe5cf0e1ff147dbe5e658168337f50616d2582cd200dc`.
The Web shared viewer and browser vendor snapshots matched the core files
byte for byte. The fixture reused pinned Playwright 1.63.0, existing `ws`,
Node 26.8.2 and sandboxed headless Chromium 152.0.7977.82. It downloaded no
dependencies and used fresh private HOME/XDG/state directories.

Run from the matching core checkout, substituting existing absolute paths:

```sh
python3 voyage/tests/host_browser.py --agent-interactions \
  --binaries /absolute/path/to/unpacked/archive/bin \
  --web-resources /absolute/path/to/webhelm/resources/js \
  --ws /absolute/path/to/webhelm/node_modules/ws \
  --node /absolute/path/to/node \
  --evidence-dir /absolute/path/to/ignored/evidence
```

### Historical observed results

The command exited zero. All three journey phases passed: live mounted/shared
viewer plus native Helm local-owner/access-file routes, fresh native preparation
after actual automatic suspension, and fresh Web preparation after suspension.
There were two independent Vessel-supervised Voyages and 38 scripted loopback
provider exchanges with no fixture errors; no real provider or inference budget
was used. Reconnecting viewers did not replay the run or request more inference.

The agent sequence exercised observed references, fill/select/check/key/double
click, child-frame inspection/fill, diagnostics, reload and read. Both viewers
then exercised ordinary first-action control, private takeover, navigation,
history/title, modal responses and text input. Assertions excluded private human
input and private URLs from the completed conversations and checked independent
Voyage state, exact owner/revision preparation, one-time native bootstrap and
explicit close/disconnect behavior.

Both site-class runs observed the external stylesheet, cookie-gated image, open
shadow content and cross-origin child replay/control. Recorded local input-to-
visible samples were 241 ms and 228 ms; these are fixture observations, not a
performance guarantee. Desktop/mobile pre-private screenshots were inspected;
the shared viewer controls and page were rendered without horizontal overflow.

Final cleanup observed no owned processes, no forced PID cleanup, and stopped
both fixture HTTP server threads. Each viewer phase observed browser closure.
Raw logs, phase reports, sanitized summaries and pre-private screenshots are
retained under ignored `target/verification-v103/final/browser-cross-client-31dd115`.
The live viewer phase report SHA256 is
`7106cb917be852fe3ba25f56d868ca40c0121b85f973601edd3db51d1e326854`.

### Historical deployment observations

Read-only requests to the deployed Helm Web origin passed normal TLS certificate
verification: `/up`, `/landing`, the Vite manifest and its referenced scripts
returned HTTP 200. Anonymous `/` redirected to `/landing`. The deployed React
asset had SHA256 `bcc3ea48269381157e6f3ac687427d5a5703439bddb519e28a477ca957f49684`,
which differed from the local candidate asset
`5e838a09f8d5919edb29972ec43635ce923c31111854fd441f4eb55841d16c0d`.
This does not establish that the candidate Web source is deployed.

At that earlier qualification, the browser connector exposed no authenticated
browser surface. Both configured
administrative SSH routes were unreachable, so no updater status command ran and
no deployment request was issued. Current Web deployment is recorded above;
production browser TLS/client acceptance remains open. These historical journeys do not
establish Root-bound public transport, native macOS/Windows, arbitrary public-site
security, installer behavior or observed cleanup on other platforms.

## Visible inline control fragments

The worker now checks at most 32 viewport-clipped nonzero client rectangles,
retaining the existing shadow/ancestor hit test and all authority, privacy,
reference, sequence and enabled-state checks. Agent clicks use Playwright's
non-force actionability checks; cursor and drag positions use a validated
visible fragment rather than the combined bounding rectangle's center.

The full worker suite passed 25 cases with zero failures, cancellations or skips
on Node 26.8.2 and the existing pinned Chromium/Playwright toolchain. Its new
real-Chromium case proves that a wrapped link's combined center can miss all
clickable fragments, requires one agent and one human effect with exact receipt
replay, and refuses opaque overlays. The expanded four-phase client journey above
now includes passing adversity, crash and cleanup assertions. This does not
establish complete #333 or deployed TLS
acceptance. The earlier Web refusal's exact cause remains unproven because its
ephemeral input receipt was removed during observed cleanup.

## Historical live replay recovery

The shared viewer uses rrweb's supported `useVirtualDom: false` option so old
reset mutations apply to the visible DOM during live `addEvent`. The previous
virtual DOM path acknowledged those events without a live Flush, leaving the
view stale after read recovery. Timestamps, private fencing, CSP, authority and
unknown effect handling are unchanged.

A real Chromium regression passed at desktop and mobile widths with natural
recorded timestamps: aged snapshot0 plus mutation1 recovered visible1, then a
subsequent recorded delta progressed to2. It retained the existing frame/focus/private cases
and bounded replay instances, payload and recovery time. Actual recovery samples
were 43.6/43.9 ms with naturally aged events, four recorded events and
17,405/17,404 raw fixture bytes. The regression preserved one top-level and two
child replay instances. The later stopped-state case passed accessible action
naming and retirement of replay instances. These Chromium results use synthetic
transport; the real-process adverse qualification is recorded above.
