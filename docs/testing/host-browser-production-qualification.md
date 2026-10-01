# Remaining #333 production qualification

This is a source-derived execution route and unexecuted qualification tooling.
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
measures browser replay cost. The executable production route below is separate
from those synthetic harnesses and the retired benchmark.

## Executable production route

The new entry point is:

```sh
python3 voyage/tests/host_browser_production.py --config /private/operator-config.json
```

This source has not been executed. Source checks are Python AST parsing, Node
syntax checking and diff checks. The driver does not create grants, accounts or
Voyages, submit prompts, deploy a proxy, start a service, fetch dependencies or
change runtime permissions. The release coordinator prepares exactly two empty
ordinary fixture Voyages, source/build/deployment evidence and the private input
files before running it. Preserve the actual authentication refusal if admission
is unavailable; never replace it with a loopback socket or fabricated ticket.

| File | Actual responsibility |
| --- | --- |
| `voyage/tests/host_browser_production.py` | Validates the private config and real public catalogue/canonical snapshots; selects only two named fixture Voyages in actual TUI clients, presses F6 and captures their one-use private launchers; owns only those two local PTYs. Rechecks empty history/no run/no cleanup obligation afterward. |
| `voyage/tests/host_browser_production.mjs` | Opens qualification-owned native viewer pages and either real authenticated React tabs or requests actual CUA Web steps; runs the matrix, benign counter/cross-frame actions, CSS/shadow/authenticated-asset/media observations, private exclusion/disconnect/reclaim, real renewal and explicit close. |
| `voyage/tests/host_browser_production_probe.py` | Read-only before/after proof on the executing Linux host. Pins the selected worker lock's browser UUID, exact guardian/worker PID/starttime, owned descendants, short scratch directory and exact session slot; after close requires the fresh guardian marker, pinned identities gone, scratch/lock removed and no session slot. It never repairs or removes runtime evidence. |
| `voyage/tests/host_browser_production_site.py` | Optional owned fixture behind two separately approved HTTPS origins and one explicit temporary prefix. External CSS, cookie-gated SVG, open shadow, cross-origin and nested frames, benign dynamic actions, canvas, pinned synthetic video and nine bounded frames exercise selective fallback. It binds loopback only and exposes no control/account/provider/viewer endpoints. |

### Private configuration

The top-level JSON object has `schema:1` and these required inputs. Every private
JSON input is owned by the invoking ordinary user, mode `0600`, regular, one link,
not a symlink, and read through a bounded checked descriptor. Output is a new
private directory; earlier evidence is not overwritten.

| Fields | Meaning |
| --- | --- |
| `helm`, `node`, `chromium`, `playwright_module` | Absolute paths to already installed/current qualified executables and Playwright module. No build/download step is performed. Chromium sandboxing and normal TLS validation remain enabled. |
| `access_file`, `output` | Existing native paired credential path and new private evidence directory. The credential is consumed by Helm's normal validation/decryption, not exported or parsed by this harness. Existing `VOYAGE_CREDENTIAL_KEY_FILE`, when required, remains a path only. |
| `sessions` | Exactly two `{id,title,workspace,label,host_browser_root}` objects. UUIDs distinct; title begins `qualification-333-` and is at most 64 characters; absolute non-root workspace; distinct short safe labels. The actual catalogue's name/workspace must match and canonical history/run must be empty before effects. `host_browser_root` is each actual `journal/host-browser` directory. |
| `console_origin`, `web_connection`, `web_socket` | Actual HTTPS console origin, saved Web connection database UUID and pinned real `wss://…/v1/vessel/browser-socket` endpoint. The physical Vessel UUID is not the saved connection UUID. |
| `web_mode` | `playwright` or `cua`, as described below. |
| `client_ledger` | `automatic` generates an exact private PID/starttime ledger for this process's two TUI clients and its own Chromium server. Alternatively an existing private ledger may include explicitly owned renderer/native client roots. It must not include unrelated browser tabs/processes. |
| `native_wire_evidence` | Private host-produced native application-payload measurement, schema below. The driver never substitutes localhost HTTP bytes or TCP/TLS bytes for this layer. Missing evidence prevents overall acceptance. |
| `fixture` | `{url,ready_selector,counter_selector,click_name,private:{url,ready_selector,input_label}}` for the approved owned HTTPS fixture. Standard counter is `/ui`, `h1`, `#count`, `Increment task counter`; private page is `/private`, `h1`, `Synthetic private input`. |
| `site_classes` | `{url,ready_selector,asset_selector,asset_width,shadow_selector,shadow_text,css_color,frame_texts}`. Standard values: `/site-classes`, `#result`, `#authenticated-asset`, `40`, `#shadow`, `Open shadow content`, `rgb(17, 51, 85)`, `["Cross-origin child content","Nested child content"]`. |
| `sites` | Three to six explicitly approved `{label,kind,url,ready_selector}` records, including `static`, `dynamic`, `frame-media`. URLs must be pinned canonical final HTTPS URLs, because navigation waits for authoritative URL/new document epoch and the replay marker. Add `{fallback_selector:".browser-next-visual",minimum_fallback_images:3}` to the owned `/media` case. At least one record per class must be an approved representative site if those claims are to be made; using only the owned fixture does not prove broader fidelity. |
| `host_observer` | Mode/paths/hash pins below. Host fixtures and exact ordinary-account ledgers are prepared by the authorized host operator after observing actual PIDs; no permission changes or guessed process identities. |

For **Playwright Web mode**, supply `storage_state` containing a legitimately
provided, existing authorized Web login for this qualification. A fresh owned
Chromium context consumes that private object directly. Do not scrape/export the
ambient human browser's cookies or mint a replacement server session. The driver
uses real `/voyages/<connection>/<session>` routes, selected card, Browser dock,
normal CSRF ticket endpoint and actual WebSocket. Passive observation keeps only
selected fixture bindings, categories/counters and byte totals; auth frames,
private text, raw mirror data and arbitrary messages are not retained.

For **CUA Web mode**, supply `cua_mailbox`, a new private directory. The existing
authenticated browser remains owned by CUA. The driver opens no Web login/context,
attaches to no ambient debugging socket, and sends no direct protocol commands
for Web. The authorized CUA operator opens/keeps two actual fixture tabs, performs
the one-use UI steps below and returns observations from those exact tabs. This
allows native pages to run on CT106 while the human browser renders React on its
actual client machine. Native client measurements on CT106 are separate from
the human browser's per-tab measurements, never presented as the same host.

### Host observation routes

`host_observer` always specifies absolute `python`, `cost_script`, `probe_script`,
`capacity`, `ledger_a`, `ledger_ab`, plus `cost_sha256` and `probe_sha256` pins.
Both maintained helper hashes are verified before observation. `ledger_a` selects
only fixture A's observed Voyage/browser tree; `ledger_ab` selects A and B. Add
gateway/supervisor leaf roots only when their exact identities and measurement
scope are authorized. Distinct roots must not overlap.

- `mode:"local"`: run the fixed helpers as the existing ordinary account on the
  executing host. This is suitable for a CT106 job launched through the authorized
  HomeProxmox chat; the driver performs no CT/service change itself.
- `mode:"ssh"`: add `hostname`, `user`, `port`, `known_hosts` and optional
  `identity_file`. Fixed SSH uses batch mode, explicit strict host-key checking,
  no alternate SSH config, and bounded read-only helper commands. Paths/arguments
  are restricted; no shell snippets or socket rewrite are accepted.
- `mode:"mailbox"`: add a private `mailbox`. The separately authorized host
  conduit executes each exact pinned read-only helper request once and returns
  its private bounded stdout. This avoids treating an inaccessible saved SSH
  route as a reason to change credentials or host policy. Delayed host windows
  record their actual timestamps; mismatched windows leave cost qualification
  incomplete rather than comparing different intervals.

Both samplers use three 10-second windows per condition. The second browser has
already been allocated by its TUI, but its viewer is not opened and its root is
excluded in the first two fixture-A measurements. Each condition's exact scope
is recorded. The common workload is the counter fixture; complex site/media
actions are fidelity observations outside these windows, not a claim about their
CPU cost. A no-viewer capture interval and any historical encoder comparison are
separate evidence, not invented by this driver.

### One-use CUA/host operator handoff

The driver writes `<operation UUID>.request.json`, mode `0600`, in the chosen
private mailbox, with `{schema:1,id,kind,operation,expires_at_ms,digest}`. `digest`
is SHA-256 of the exact request object before adding the digest field. The operator
returns a new regular private `<same UUID>.response.json`:

```json
{"schema":1,"id":"same UUID","digest":"same digest","status":"observed","result":{}}
```

Only actual completed observations receive `observed`; retain the corresponding
CUA/host command evidence separately. Do not fill expected booleans merely
because the request lists them. Failed/unknown/expired steps stop acceptance;
neither the driver nor operator automatically resends an effect. Host requests
contain `{argv,stdin}` for only fixed read-only helpers/hash reads. Their `result`
is the helper's stdout string. `before` probe stdout contains private process
identities and paths and must remain private; `after` stdout contains fixed
cleanup booleans. Credentials are never included in mailbox requests.

CUA requests are explicit UI/observation operations:

| Operation | Actual action and proof |
| --- | --- |
| `open_fixture` | Open exact supplied route in the existing authenticated browser; verify selected title/route; click real Browser dock once; replay marker/counter. Keep the other fixture tab open. |
| `measure_window` | Observe only the supplied qualification Web tabs for the actual 10-second window. Per viewer return label, exact scope `actual CUA qualification Web tab renderer and public WSS application payload`, task seconds, heap-used bytes, sent/received application bytes and `selected_connection_observed:true`. If that observer is unavailable, return `{label,status:"unavailable"}`. Interaction continues but measurements cannot pass. Never substitute whole-browser/task-proxy metrics. |
| `observe_counter`, `observe_public_site`, `observe_site_classes` | Read replay only, no input. Verify exact counter/marker or CSS/shadow/authenticated image/cross+nested frames. Public screenshots may be retained here. |
| `nested_child_once_return` | Observe the native dynamic/child result, click the nested child's benign button exactly once in Web, observe its replay change, explicitly Continue agent. Preserve refusal/unknown and never retry. |
| `observe_private_exclusion` | No replay iframe, empty address and private/watching presentation in the other Web client. No screenshot or private content export. |
| `private_reclaim_return` | Click Browse privately once using the same human principal; confirm private control. Navigate to the supplied ordinary fixture while still private, then explicitly Continue agent, perform the new counter click once, verify one, and retain exact outcomes. |
| `observe_real_renewal` | Actual replacement ticket/socket acknowledgement for both selected Web connections, retained browser identity and no effect replay/rewrite. Do not claim renewal from an elapsed timer or an unchanged screenshot. |
| `close_browser_once`, `close_fixture_panels` | Explicitly close only the owned browser once, confirm stopped/known receipt, then dispose only the two fixture tabs/panels. Host probe supplies independent resource proof. No replay of an uncertain close or unrelated cleanup. |

The exact fixed reply keys are alongside each call in
`host_browser_production.mjs`. The script requires every acceptance observation;
it does not replace CUA with a synthetic transport. Without a usable CUA per-tab
observer, costs remain a real measurement dependency even when UI passes.

Native wire evidence is a private `{schema:1,scope:"owned native Helm public WSS
application bytes",verified_identity:true,roots:[...],windows:[...]}` object.
Its two `{label,pid,start_ticks}` roots must exactly match the invoking TUI
clients, in order. Nine windows have exact `condition`/`index` and nonnegative
integer `sent_bytes`/positive `received_bytes`. Retain the actual authorized collection method, PID/socket
identities, layer and timestamp evidence externally. Only fixed byte totals are
copied to the summary. Such an observer is not supplied by the browser CDP helper;
do not manufacture these numbers to obtain a green result.

### Owned HTTPS site and final evidence

If an owned public fixture is needed, the separately authorized host operator may
start `host_browser_production_site.py` with approved distinct `--origin` and
`--child-origin`, explicit `/release-qualification-333/<run>` prefix,
unprivileged loopback `--port`, at most 900 seconds, and a pre-approved synthetic
WebM path plus its exact `--video-sha256`. The existing TLS proxy must publish only
that reviewed prefix on both origins; the helper neither edits nor bypasses it.
It bounds video/response to 1 MiB, connections to 16, per-connection requests to
64, aggregate successful requests to 4096 and response bytes to 64 MiB. On exit
it reports fixed route/byte counters and stops its own fixture threads. Keep real
provider/account endpoints and unrelated sites outside this fixture.

`production-report.json` contains categories, source scope, measurement values,
fixed cleanup facts and URL hashes. Private configuration, native PTY traces,
launcher/bootstrap paths and before-resource proof remain private. No screenshots
are taken during synthetic private input. Final success requires all UI receipts,
real renewal, complete aligned metrics and exact after-close host proof. Unknown
effects are retained and never replayed; an incomplete run records the exact
owned remote cleanup obligation for the authorized operator. Local PTY exit is
reported separately from remote resource cleanup. This source has made no native,
deployed TLS, site fidelity, cost or release acceptance claim.

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
