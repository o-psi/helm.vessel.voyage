# Remaining #333 production qualification

This is a source-derived execution route and unexecuted qualification tooling.
The completed current-archive four-phase synthetic evidence is in
[the v1.0.3 record](host-browser-v1.0.3.md). Do not repeat that full cohort merely
to collect production evidence, or call these remaining gates passed.

## Actual paths and prerequisites

| Path | Maintained implementation | Required private input |
| --- | --- | --- |
| Native TUI | Explicit `helm connect --access-file ... --no-start` **or** `helm connect --directory ... --no-start`, actual F6; `helm/src/process_client/host_browser.rs`; shared viewer | Existing public paired credential, or the same ordinary account’s private local discovery directory; selected fixture Voyage. The public grant must permit actual observe/execute/history operations. |
| Deployed React dock | `resources/react/App.tsx`, `HostBrowser.tsx`, `workspace.ts`; `resources/js/host-browser.js` | Existing authenticated operator tenant/session and saved Vessel connection. |
| Browser bootstrap | `ConsoleAuthController::ticket`, `ConsoleAccess::ticket`, `VesselGateway::browser-credentials`, `resources/js/vessel-fleet.js` | Same-origin CSRF POST `/console/ticket`; valid stored grant, pinned Vessel identity and tenant entitlement. |
| Actual Web transport | Origin-bound 120-second credential, `voyage.vessel.v1`, `/v1/vessel/browser-socket` | Real configured Web Origin and normal certificate trust; renewal must remain observable. |
| Native transport | Public mode: pinned HTTPS `/v1/vessel/socket`. Local mode: maintained private directory discovery and loopback `/v1/vessel/socket`. Both use the private localhost viewer bootstrap | Original authenticated socket/owner fences. Local is existing executing-account authority, never a copied Web grant or public TLS claim. Keep the launcher private and one-use. |

The present owner-related connection refusal is a real admission blocker. Require
an authenticated capabilities/snapshot observation for the selected fixture
Voyages through the deployed route before browser effects. Public `/up` and live
service PIDs do not substitute. Do not fix refusal by broadening authority,
changing tenant identity, disabling Origin checks or copying secrets into logs.
Native and Web retain their independently authorized grants/principals. The
initiating Native principal performs private disconnect/reclaim in a fresh Native
viewer; Web never borrows that private authority. Distinct grant and participant
surfaces retain their existing meanings.

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
python3 -I voyage/tests/host_browser_production.py --config /private/operator-config.json
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
| `native_route`, `access_file`, `output` | Optional `{mode:"public"}` preserves the existing paired credential path. Explicit `{mode:"local",directory:"/canonical/private/Vessel",expected_vessel_id:"UUID"}` instead uses existing local discovery and forbids `access_file`. New private evidence directory. The credential is consumed by Helm's normal validation/decryption, not exported or parsed by this harness. Existing `VOYAGE_CREDENTIAL_KEY_FILE`, when required, remains a path only. |
| `sessions` | Exactly two `{id,title,workspace,label,host_browser_root}` objects. UUIDs distinct; title begins `qualification-333-` and is at most 64 characters; absolute non-root workspace; distinct short safe labels. The actual catalogue's name/workspace must match and canonical history/run must be empty before effects. `host_browser_root` is each actual `journal/host-browser` directory. |
| `console_origin`, `web_connection`, `web_socket` | Actual HTTPS console origin, saved Web connection database UUID and pinned real `wss://…/v1/vessel/browser-socket` endpoint. The physical Vessel UUID is not the saved connection UUID. |
| `web_mode` | `playwright` or `cua`, as described below. |
| `client_ledger` | `automatic` generates an exact private PID/starttime ledger for this process's two TUI clients and its own Chromium server. Alternatively an existing private ledger may include explicitly owned renderer/native client roots. It must not include unrelated browser tabs/processes. |
| `native_wire_evidence` | New private result path. The harness produces nine windows from the opt-in source-owned native socket collector, explicitly labelled `public_wss` or `local_ws`; bridge/TCP/TLS bytes are not substitutes. |
| `fixture` | `{url,ready_selector,counter_selector,click_name,private:{url,ready_selector,input_label}}` for the approved owned HTTPS fixture. Standard counter is `/ui`, `h1`, `#count`, `Increment task counter`; private page is `/private`, `h1`, `Synthetic private input`. |
| `site_classes` | `{url,ready_selector,asset_selector,asset_width,shadow_selector,shadow_text,css_color,frame_texts}`. Standard values: `/site-classes`, `#result`, `#authenticated-asset`, `40`, `#shadow`, `Open shadow content`, `rgb(17, 51, 85)`, `["Cross-origin child content","Nested child content"]`. |
| `sites` | Three to six explicitly approved `{label,kind,url,ready_selector}` records, including `static`, `dynamic`, `frame-media`. URLs must be pinned canonical final HTTPS URLs, because navigation waits for authoritative URL/new document epoch and the replay marker. Add `{fallback_selector:".browser-next-visual",minimum_fallback_images:3}` to the owned `/media` case. At least one record per class must be an approved representative site if those claims are to be made; using only the owned fixture does not prove broader fidelity. |
| `host_observer` | Mode/paths/hash/program pins below. Host fixtures and exact ordinary-account ledgers are prepared by the authorized host operator after observing actual PIDs; no permission changes or guessed process identities. |

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
Both maintained helper hashes are verified before observation. For the selected
CT106 `local` route, ledgers are now derived after native viewer/bootstrap and
before the first window of **each** condition; `ledger_a`/`ledger_ab` are retained
legacy external-ledger fields and are not used for local PID attribution. A
pre-create Voyage PID may suspend/change and cannot qualify this route. Add
gateway/supervisor leaf roots only when their exact identities and measurement
scope are authorized. Distinct roots must not overlap.

- `mode:"local"`: run the fixed helpers as the existing ordinary account on the
  executing host. Supply `programs` with exactly `node`, `worker`, `guardian`,
  `python` and `voyage`, each `{path,sha256}` for the installed qualified files.
  Paths must be canonical absolute paths, not wrapper scripts/symlinks. The
  ordinary launcher stamps its actual qualified Helm path/hash into private
  `native_helm_program` and captures both TUI PID/start identities; these are
  verified again by the initializer. This is suitable for a CT106 job launched through the authorized
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

Both samplers use three 10-second windows per condition. Both native TUI clients
are connected, but fixture B's browser is opened only when its actual one-use
viewer is loaded; its root is excluded from the first two fixture-A measurements. Each condition's exact scope
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
| `measure_arm` | Poll the already installed real CDP observers, choose a shared UTC start about 15 seconds ahead, and return `{started_at_ms}`. Root writes the exact private arm response before that time. This admits no website action. |
| `measure_window` | Observe only the supplied qualification Web tabs for the actual 10-second window. Per viewer return label, exact scope `actual CUA qualification Web tab renderer and public WSS application payload`, task seconds, heap-used bytes, sent/received application bytes and `selected_connection_observed:true`. If that observer is unavailable, return `{label,status:"unavailable"}`. Interaction continues but measurements cannot pass. Never substitute whole-browser/task-proxy metrics. |
| `observe_counter`, `observe_public_site`, `observe_site_classes` | Read replay only, no input. Verify exact counter/marker or CSS/shadow/authenticated image/cross+nested frames. Public screenshots may be retained here. |
| `nested_child_once_return` | Observe the native dynamic/child result, click the nested child's benign button exactly once in Web, observe its replay change, explicitly Continue agent. Preserve refusal/unknown and never retry. |
| `observe_private_exclusion` | No replay iframe, empty address and private/watching presentation in the other Web client. No screenshot or private content export. |
| Private reclaim | The owning Python/Native path issues one fixed TUI reopen, then the initiating Native principal explicitly reclaims, clears the private fixture and returns. CUA Web only observes `observe_private_exclusion` then `observe_counter`; it never receives a Web reclaim request. |
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
integer `sent_bytes`/positive `received_bytes`. The native source observer now
provides this layer, separately from browser CDP. Only fixed totals reach the
summary; no value is inferred from ciphertext or localhost bridge bytes.

### Source-owned native and supported CUA collectors

`helm/src/process_client/duplex_qualification.rs` is default-off. The launcher
sets `HELM_QUALIFICATION_WSS_OUTPUT` to a new private file per TUI and
`HELM_QUALIFICATION_WSS_LABEL` to its exact owned label. Only actual `wss` socket
activations enable it. It counts received Text payload bytes and Text bytes after
successful sink send, taking no frame content, credential, URL or command ID.
HTTP upgrade, TLS/TCP and Ping/Pong are excluded. Successful send is not peer
acknowledgement or website success.

The bounded writer holds a new `0600` inode in the invoking user's private
directory, publishes every 200 ms and expires at 600 seconds. One route ID plus
a multiple-route flag bounds attribution. PID/starttime, active socket,
handshake failures, disconnection/loss counters and source age prevent historical
connection counts from qualifying a retired socket. The harness pins inode and
process identity, rejects stale/unknown/expired data and transport changes within
a window, and records actual snapshot times. Identical read-only retries across
a write are bounded; effects are never repeated. This instrumented native-client
cost includes the observer's small file/thread overhead.

`voyage/tests/host_browser_cua_cost.mjs` implements the supported CUA capability.
Root loads `cuaBrowserCost` in its persistent CUA runtime, on each fixture tab:

```js
const observer = await cuaBrowserCost(fixtureTab, {
  label: 'a', tabId: EXACT_OWNED_TAB_ID,
  socketUrl: EXACT_PINNED_BROWSER_SOCKET,
  connectionId: EXACT_SAVED_CONNECTION_ID,
  sessionIds: [OWNED_SESSION_A, OWNED_SESSION_B],
});
// Root performs a normal same-origin reload, then opens Browser.
await observer.poll();
await observer.begin();
// Observe the actual coordinated ten-second window without repeating input.
const counts = await observer.finish();
// Put only counts in the exact private response; never emit raw CDP events.
await observer.stop();
```

Exact API calls are `tab.capabilities.get('cdp')`, scoped `cap.send` and
`cap.readEvents({afterSequence,limit:1000,methods,target,timeoutMs:0})`.
`Target.getTargetInfo` selects only that tab, without returning its URL. The
collector enables Network/Performance/Runtime and captures the current cursor
before Root's normal same-origin reload. The real Fleet socket then creates an
attributable event; an already-open socket missing `webSocketCreated` cannot
be called measured. No constructor/endpoint replacement or login export occurs.

Events are WebSocket created/closed/sent/received, fixed console diagnostics and
owned attached/detached child-target metadata. Both supplied tab/target IDs must
agree. Pagination drains `hasMore` through at most 20 pages; gaps, `truncated`,
wrong attribution and unacknowledged/closed sockets refuse. Child session IDs
come only from owned `Target.attachedToTarget`, never a browser-wide target list.
Only selected application-frame lengths survive reduction. Production fixed
`[Helm connection]` metadata proves connected/renewal acknowledgement; transient
selected-operation correlation reduces duplicate/refused/unknown/pending effects
to counts. Auth/private payloads and console objects are not emitted or fetched.

Performance facts are Task/Script/Layout duration, JS heap and node/document/
listener counts. Renderer sharing is possible: do not sum task time or heap
across tabs or present them as whole-browser OS CPU/RSS. The mailbox includes
heap/node deltas and actual start/duration; the harness checks Web/host alignment.
A late sample is incomplete. Root must coordinate actual CT/CUA windows rather
than filling numbers for an interval that already ended. These collectors and
regressions are prepared source, not executed native/cost proof.

The executable CUA path uses an explicit arm handshake for every measured Web
window. On `measure_arm`, Root polls each owned observer, chooses
`started_at_ms = Date.now() + 15000`, and returns that timestamp through the
approved host conduit. CT publishes `measure_window` immediately and waits for
that future start before running its native/host observers. Root then runs one
bounded CUA call:

```js
const viewers = await Promise.all(ownedObservers.map(
  observer => observer.measureAt(EXACT_ARMED_UTC_START, 10000)
));
// Return only {viewers} to this window's matching private response.
```

This waits for the real future interval within the active tool call; it assumes
no detached CDP/background timer survives a tool turn. Arm receipt must reach CT
with at least one second remaining, within 30 seconds, and the real sample must
align within one second. A delayed host conduit is incomplete, never a reason to
backdate metrics or increase effect timeouts. The overall 600-second bound stays.

### Exact Home Proxmox → CT106 preparation order

Serialize this with recovery/native jobs. Source `76c26aa` rejected the retained
legacy owner during preparation; certificate-valid `/up` and `/health` did not
prove admission. Continue only after the corrected qualified archive is deployed
and the same tenant/principal/Vessel is admitted. No uncertain update replay,
grant rewriting or loopback-discovery substitution for TLS.

1. **Read-only references.** As the existing ordinary execution account, resolve
   current binaries, credential references, authoritative session/capacity roots,
   Python/Node/Playwright and worker/guardian. Return versions/hashes/UID and
   permission/availability booleans only. Run current Helm
   `connect --access-file EXISTING_PRIVATE_REFERENCE --no-start list` with a
   30-second deadline. No secret contents, create/rename/Submit or browser starts.
2. **Dependencies before effects.** Preparation observed Debian 13.6, Node
   20.19.2, Playwright-core 1.63.0, missing Chromium, about 9.6 GiB disk, 2 GiB
   RAM/no swap. Maintained worker Node minimum is **20.20.0**. Read-only:
   `apt-cache policy chromium chromium-sandbox nodejs`,
   `apt-get -s --no-install-recommends install chromium chromium-sandbox`, and
   `df -Pk /usr /var/cache/apt/archives /tmp`. Record exact transaction, sizes,
   removals/upgrades and service effects. Cached candidates are not refreshed pins.
3. **Approved maintenance provisioning.** Refresh official apt metadata, resolve
   matching Chromium/sandbox versions, repeat the minimal simulation with exact
   pins, then install only that approved `--no-install-recommends` transaction.
   Verify normal package ownership/sandbox with `dpkg-query -L chromium-sandbox`
   and `stat`; no chmod repair, `--no-sandbox` or container/kernel policy change.
   Prefer supported official Debian Node; otherwise the host operator resolves
   an official nodejs.org supported LTS archive and published checksum, verifies
   and extracts it into the executing user's private runtime. No runtime/model
   fetch, unpinned latest resolution or global Node replacement in the journey.
4. **Actual Node override.** `Launch::discover()` checks `/usr/bin/node`, then
   `/usr/local/bin/node`, ignoring PATH. Private Node alone does not select it.
   Executing-host private LaunchConfig must set `config.host_browser_launch` to
   `{node:ABSOLUTE_SUPPORTED_NODE,worker:QUALIFIED_RELEASE/share/voyage/browser/worker.mjs,
   chromium:/usr/bin/chromium,config:{public_web:true,origins:[],width:1280,height:720}}`.
   This is not portable settings or an invented environment override. Required
   sandbox mode disables host-browser launch and remains a real refusal.
5. **Existing native TLS authority.** Current Connections UI imports but has no
   export action. Local supervisor discovery is separate loopback authority.
   The already-authorized host helper may reuse the exact retained encrypted
   `VesselConnection.credential` locally for the authenticated owner's selected
   tenant/connection/revision, with unchanged endpoint/Vessel/principal/grant.
   Supply the original native-format access file privately, owned `0600`, never
   stdout/chat; do not mint/mutate grants or invent missing identity fields.
   Recheck metadata before/after use; parser incompatibility is a real dependency.
6. **Two owned fixtures.** Use a normal dedicated synthetic named account if
   needed (closed loopback endpoint, synthetic credential/binding), preserving
   unrelated account/environment entries. Retain session/create UUIDs before:
   `CURRENT_HELM connect --access-file EXISTING_PRIVATE_REFERENCE --no-start new
   --id SESSION_UUID --command-id CREATE_UUID --workspace OWNED_WORKSPACE
   --config-path PRIVATE_LAUNCH_JSON`. `New` has no `--name`. Read `inspect`, then
   typed `request SESSION_UUID` Rename JSON with fresh command UUID, observed
   `expected_revision`, bounded `expires_at_ms`, and
   `name:qualification-333-RUN-a/b`. Require exact success/empty history/no run;
   never Submit/Goal/infer. Resolve lost creates by retained ID, not another create.
7. **Real site/operator/cost.** Approve only an owned temporary prefix on two
   existing certificate-valid HTTPS origins and pinned synthetic WebM. Start the
   bounded site separately. Prepare exact A/AB ledgers, helper hashes, private
   config/mailbox. Run `python3 -I voyage/tests/host_browser_production.py --config
   PRIVATE` as ordinary account with `web_mode:cua`, `host_observer.mode:local`,
   `client_ledger:automatic` and new `native_wire_evidence` path. Root alone owns
   two actual authenticated fixture tabs, CDP collection, normal reload/dock/
   actions. Preserve public native/browser WSS endpoints and strict TLS. Collect
   all nine aligned windows/site/privacy/renewal facts, explicit two browser
   closes and independent host `after` proof.
8. **Observed completion only.** Missing permission/heap/transport/cleanup data
   is unknown, never zero/pass. Dispose only owned clients/fixture. Retain unknown
   outcomes, slots/scratch/receipts until actual cleanup; no old-effect retry.
   Claims are only for the actual Linux host/sites. Source, installation,
   certificates and boolean operator responses are not a production journey pass.

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
| Native viewer payload | Passive CDP counters for that owned localhost `/operation` URL | UTF8 request and decoded response application bodies; encoded response transfer bytes and in-flight edges separate. This is not the Native–Vessel socket or total TCP/TLS cost. Native local/public socket bytes use the separately labelled passive native meter. Missing task-time/heap/body/encoding, empty selected source or in-flight requests cannot pass. |
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


### Current same-host ledger initialization

The local driver invokes only the pinned `host_browser_production_probe.py ledger`
after an actual native page reports its running browser binding, before arming a
cost window. It supplies the one/two selected owned worker roots, current browser
UUID/incarnation, both native TUI PID/start bootstraps and qualified program pins.
The read-only initializer verifies ordinary current UID, live process states,
exact executable inode/path/hash, worker and guardian argument shape, guardian
parent as the qualified Voyage, private root/worker-lock inode identity, scratch,
capacity slot and absence of any prior/in-progress cleanup marker. It refuses
foreign/stopped/suspended/replaced/unknown identities; it never wakes a runtime,
executes a command, changes permissions, signals a process or repairs a receipt.

Each selected Voyage is a leaf root, and its browser guardian is a descendant
root; the native client identities are excluded from those host roots. This
retains Voyage CPU/RSS while avoiding double-counting the browser tree. New
mode0600 `host-ledger-CONDITION-private.json` and matching proof files are written
in the newly owned output directory. The Node caller checks the same native
binding before/after preparation and uses that exact ledger for the three
windows. The sampler still verifies current root PID/start and bounded descendants
on every sample. All program/initialization hashing is outside measured windows;
its overhead is not silently presented as browser CPU. Native TUI/renderer cost
and coordinator overhead keep their distinct scope.

This automatic initializer is for `mode:"local"`, where the native clients and
Vessel browser share the CT106 PID namespace. SSH/mailbox retain the explicitly
externally prepared-ledger boundary and do not gain the ability to interpret a
client PID in a remote namespace. They are not the selected CT106 acceptance
route. No static/fictional PID or missing metadata can substitute for the local
initializer's observation. Python samplers/probe run with `-I`; the probe imports
its one maintained sibling by fixed importlib path. The main launcher admits only
its maintained sibling fixture directory after isolated Python startup.

### Existing native credential input dependency

An existing legitimate UID1000 private native credential must authorize the two
owned fixtures over the approved public HTTPS endpoint. It need not share the
Web principal: private reclaim remains with the initiating Native principal. Helm's workspace credential parser accepts exactly seven fields:
`schema_version` (1), `kind` (`workspace`), `endpoint`, `grant_id`, `principal_id`,
`vessel_id`, `token`. This is distinct from the qualification config's `schema:1`.
The production driver validates only its private file shape and delegates
validation/decryption to normal Helm. Do not print or paste its contents.

Keep an existing encrypted connection file at its legitimate original basename
and credential-key purpose; the storage purpose binds `helm-connection:NAME`.
If needed, provide only the existing supported `VOYAGE_CREDENTIAL_KEY_FILE` path
locally to the launcher, never its contents. Do not copy PHP33's encrypted Web
database credential into UID1000, extract a key through Root, export cookies,
mint a replacement grant, or rewrite a local discovery credential's endpoint.
`helm connect` has no endpoint override. Web Connections imports credentials;
it has no credential export control. If an existing authorized native public
file/key is unavailable, retain this as an actual input/authorization dependency
and ask only for its location. The disabled coordination form cannot create it.

Create/name the two owned empty fixtures through the existing public path only
after that input and original-owner admission are valid. CLI creation is
`helm connect --access-file EXISTING --no-start new --id SESSION_UUID
--command-id CREATE_UUID --workspace HOST_WORKSPACE --config-path PRIVATE_LAUNCH`.
The option is `--config-path`, not `--config`. The private version1 LaunchConfig
wrapper must retain the canonical workspace, reviewed config/explicit/selection/
confirmation and host browser launch settings. If naming by CLI, first inspect
its revision, then send the normal `request SESSION_UUID` JSON operation with
`op:"rename"`, an issued command UUID, observed revision, bounded expiry and
`name:"qualification-333-..."`; creation has no invented `--name` flag. Do not
Submit, start a Goal, use a provider or change grants while preparing this pass.

Prepared offline initializer contract cases are in
`voyage/tests/host_browser_production_probe_tests.py` (run with `python3 -I -B`
after the source-ready verification gate). They use mocked process metadata and
private file fixtures; they establish no native/production behavior.

The source delivery gate ran all eight offline probe contracts successfully,
`node --check` for the maintained production driver, and isolated AST parsing for
the changed Python files. The bounded unit used private umask, 1 GiB/no swap and
a five-minute cap; it completed in 151 ms at 23.6 MiB peak. No browser journey,
host resource measurement or native cleanup qualification was performed by this
gate. Rust source/Cargo inputs are unchanged; the previous truthful Rust coverage
measurement remains recorded. Live use still requires the qualified program pins,
exact same-host bootstrap/ownership proof and the production matrix above.


### Initiating Native private recovery

The Python parent already owns two actual TUI/PTY clients on their original
public access-file routes. `host_browser_native_reopen.py/.mjs` provides exactly
one source-owned fixture-A reopen, using a new private, inode-pinned mailbox.
The child can supply only issued UUID/digest, fixed action, fixture A label/
session, exact original TUI PID/start bootstrap and bounded expiry. It cannot
supply a command, endpoint, path, credentials, cookies or grants. The parent
verifies the original live process/executable/hash, the fixture title and exact
Voyage ID in the current Host browser panel, and writes an exclusive pending
receipt **before any key**. One Esc dismisses that panel, which otherwise ignores
Paste and Enter. The parent observes its dismissal with the fixture title and
owned client unchanged before typing `/browser detach` then Enter. It observes the
actual Handle.finish status `Viewer detached; host browser remains owned by
Voyage`, then sends actual F6 once and waits for a different private `open.html`
launcher produced by the normal TUI opener and the returned Host browser panel's
same title and Voyage ID. Every phase shares the original request's maximum
45-second expiry, fenced by both wall and monotonic clocks. Old launchers are one-use and are
never reloaded/reused. Any unknown key/cleanup outcome retains pending/unknown
response and fences another attempt; the action is not automatically retried.
The CUA path retains every Native page it creates separately from the current
fixture-A/B selection, including the original A after reopen and a fresh page
whose setup fails. Its final cleanup closes only those at most three pages and
records observed closure or an unresolved obligation; it never enumerates or
closes the human's CUA Web tabs. An unresolved Native page cannot pass.

Both direct and CUA routes load that fresh Native viewer and explicitly choose
Browse privately under the initiating Native process/credential. They require
the same browser/incarnation, a fresh attachment and advanced controller epoch.
Web stays excluded before and after the private page is cleared. Native then
explicitly Continues agent, sends one new benign counter click, and Web observes
that public result. Native also performs its own final browser close; a different
Web principal is never asked to reclaim or close a private controller. All old
and fresh native observation counters remain checked; the two original TUI
process identities, selected native socket byte windows and host cleanup proofs remain required.
This fixes a qualification dependency, not runtime authority. It creates no new
authority, wrapper, pairing or transport rewrite. Public mode retains its legitimate existing file/key input. Explicit Local mode requires the actual private directory/owner instead; it does not fabricate that file.

Prepared focused source contracts are `host_browser_native_reopen_tests.py`
(`python3 -I -B`) and `host_browser_native_reopen.test.mjs` (`node --test`). They
simulate the panel swallowing Paste/Enter until Esc, typed callbacks/private
files and test exact identity, one attempt, pre-key receipt, failed dismissal,
unknown result and refusal of arbitrary fields; they establish
no native/deployed browser behavior.


## Explicit NativeLocal qualification mode

[Requirements audit/source record](https://github.com/o-psi/helm.vessel.voyage/issues/333#issuecomment-5947487555)
adds a supported NativeLocal choice. #333 requires both real clients and deployed
TLS qualification; it does not prescribe a paired public Native credential for
this fixture. The original public route remains available and unchanged. Neither
choice changes the executing Voyage, runtime policy, site approvals or Web TLS.

Set `native_route` to `{mode:"local",directory,expected_vessel_id}` and omit
`access_file`. The directory must be the existing canonical private ordinary
Vessel namespace with its existing single-link private `process-http.json`.
Only normal Helm parses/uses the credential; qualification observes file identity,
never exports/parses its token or creates credentials. The launcher passes
`--directory --no-start` consistently for reads and the real TUI/F6. It checks the
validated greeting's actual Vessel/socket IDs and private meter identity before
F6. Discovery replacement or another physical Vessel refuses. Local metadata
queries are restricted to the two owned `directory/sessions/UUID` roots.

The default-off `HELM_QUALIFICATION_LOCAL_OUTPUT`/`_LABEL` enables the same private
bounded count-only native observer for actual local WS Text sends/receives.
Public mode retains `HELM_QUALIFICATION_WSS_OUTPUT`/`_LABEL`. Supplying both output
choices disables observation. Counts carry transport/authority and actual validated
Vessel/socket identity; a changed Vessel, nil greeting, stale stream, reconnect,
ambiguous route, file substitution or missed window cannot supply passing metrics.
They count application payload, excluding HTTP upgrade, control frames and TCP/TLS
overhead. Local counts never qualify Native public WSS or TLS.

Read-only `local-authority` proof binds each actual runtime's existing
`identity/actor.json` principal/installation to exact completed Attach/Control/Detach receipt claims observed from that Native
viewer. The independently checked nonsecret Vessel `identity/public.json` must match the
validated native greeting and remain byte/inode pinned. No signing key is read.
The unchanged operation digest includes its actual native socket and
runtime actor principal; mismatched sockets/bindings fail. Claims contain only
typed IDs/epochs/mode, never human input. IDs are attribution, not credentials. The
initial and explicit fresh private-reclaim proofs must retain the same actor and
physical Vessel. Browser binding/attachment/incarnation/controller fences and
observed same native client/socket remain mandatory; Web never borrows this actor.
The ordinary Linux route is not a native macOS/Windows or protected Root claim.

Native graphical metrics separately count UTF8 HTTP request bodies begun and
decoded response bodies completed inside each observation window. Encoded response
transfer bytes and requests crossing the window boundary are separate fields.
Missing bodies, nonboolean/missing response encoding, transport failure,
requests still in flight at window end, or a selected native source with no
observed requests/responses produce unknown metrics. Overflow cannot pass. No page/private text,
URL, header, cookie, command body or token is retained. These bridge and native
Vessel socket layers overlap and must **not** be summed as total bandwidth.
CPU/RSS/heap/latency and actual deployed Web public WSS scopes remain distinct.

This mode preserves the full site/media/private/control/reopen/renewal/multi-voyage
and cleanup programme and all strict Web TLS/CSRF/tenant/principal checks.
Prepared 2 Rust, 7 Python and 6 Node contract parents have not run. Source checks
are AST, Rust formatting, Node syntax and diff; Root owns the final focused/full
coverage/source publication/artifact gate and actual host qualification.


Prepared focused commands (run only inside the parent's coordinated bounded gate):

```sh
/usr/bin/python3 -I -B voyage/tests/host_browser_native_local_tests.py
node --test voyage/tests/host_browser_native_local_cost.test.mjs
```

Rust filter: `duplex::qualification::tests`. All current native and Web ordinary
transport tests remain required, then strict checks/full current workspace
coverage and actual hosted artifact qualification. No client/auth endpoint is
mocked or rewritten during production execution; pure event-source contracts
are explicitly separate evidence.
