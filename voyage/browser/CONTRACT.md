# Voyage server browser worker stdio v1

Launch `node voyage/browser/worker.mjs` with private stdin/stdout pipes; no HTTP
control endpoint, CDP port, local companion, or page-provided authority. stdout is
JSON lines only. Parent owns authentication, authorization, capacities, lifecycle
and filtering private human commands out of model history. Do not connect agent
arguments directly to arbitrary worker messages.

Each request `{id: UUID, op: string, ...}` has one reply
`{id, ok:true, result: ...}` or `{id, ok:false, error:{code}}`.
IDs identify exact canonical requests: changed reuse refuses; retries return the
retained receipt without reexecuting (observation payloads are not retained).
Durable journal fsync precedes lifecycle/agent effect dispatch. Input, offer, answer, status and receipt use a bounded 2048-entry volatile receipt LRU (no per-frame fsync). Input and signaling additionally require independent consecutive sequence numbers per viewer admission; eviction never authorizes old sequences. Reconnect creates a new admission, while epochs fence old commands. Status/receipt are harmless repeatable observations. `control` and `disconnect` fence synchronously before asynchronous quiescence, and agent results crossing any fence are withheld. Already-dispatched site effects cannot be undone. A dispatched receipt after crash means unknown,
never automatic replay. `receipt` with `request_id` queries it. No event stream:
SDP uses complete bounded ICE gathering, no trickle. Ordinary operations serialized; control/disconnect and dialog input use an interrupt lane;
input queue and line size bounded. Parent must not pipeline unbounded writes.

First request MUST be `init` with `config` from **trusted startup only**:
`{root:absolute_private_state_dir, executable:absolute_Chromium_path,
public_web:true, origins:[{origin:"https://example.com",private_network:false}],
ice_servers:[], relay_only:false, width:1280,height:720}`.
Public HTTP(S) is authorized by trusted `public_web:true`, without a manual origin ceremony; false limits browsing to the supplied origins. Private networks always require an explicit trusted origin entry. No default STUN. Up to four TURN entries, each
`{urls:["turns:host:5349?transport=tcp"],username,credential}`. No STUN URLs.
`relay_only:true` requires TURN. Browser is launched only on `open`.
Root must be dedicated, owned, mode 0700; never user profile. Existing live lock
refuses; stale locks need owner reconciliation, not blind removal/replay.

`status` returns incarnation `browser` UUID, `epochs` containing `tab`,
`document`, `viewport`, `control`, `capture`, current `mode` (`agent`, `human`,
`private`), controller ID, tab IDs, and viewer IDs (no DOM/URL/title).
`open` starts browser. `policy` with `public_web` and `origins` replaces trusted network policy, closes current proxy connections and revokes current capture/control epoch; parent only, never agent supplied. `close` awaits managed browser cleanup. `shutdown` closes
and exits. `receipt` and `status` do not open it. Every command below needs the
exact `browser` and `epochs` returned by current status; stale values refuse.

Viewer/control operations (parent-authenticated human only):
- `join`, `viewer`: create viewer ID, maximum four; private admission only controller.
- `disconnect`, `viewer`: close peer/remove viewer, relinquish controller; private
  mode remains private (no implicit return). Parent must call on socket loss.
- `control`, `viewer`, `mode`: `human`/`private` acquires sole controller or changes
  its mode; `agent` explicitly returns, only current controller. Every transition
  revokes transports/capture and advances epochs. Private reconnect requires same
  controller identity retained by parent; private disconnect retains controller ID.
- `offer`, `viewer`, `signal_seq`: returns `{type:"offer",sdp}` and current epochs. Each viewer
  gets independent sender transport. `signal_seq` starts at 1 and is consecutive across offer/answer for that viewer; sequences are consumed even on dispatched failure. `answer`, `viewer`, `description:{type:"answer",sdp}`.
- `input`, `viewer`, `seq` (start at 1, strictly consecutive), `action`:
  `{kind:"pointer",type:"move"|"down"|"up",x,y,button:"left"|"middle"|"right"}`,
  `{kind:"key",type:"down"|"up",key}`, `{kind:"text",text}`,
  `{kind:"scroll",x,y}`, `{kind:"resize",width,height}`,
  `{kind:"dialog",accept:boolean,text?:string}`, `{kind:"navigate",url}`,
  `{kind:"tabs",operation:"new"|"select"|"close",tab?:UUID}`. Only controller in human/private.

Agent operations: `agent`, `action` with `kind`:
- `inspect`: bounded text plus actionable `elements:[{ref,tag,text}]` (fresh opaque refs).
- `navigate`, `url` (allowed origin only).
- `click`, `ref`; `fill`, `ref`, `text`; `scroll`, `x`, `y`.
- `tabs`, `operation:"list"|"new"|"select"|"close"`, `tab` for select/close.
- `screenshot`: `{mime_type:"image/jpeg",data_base64}`.
- `upload`, `ref`, `name`, `mime_type`, `data_base64` (<=2 MiB).
- `download`, `download_id`: bounded bytes; inspect exposes completed downloads.
All agent actions/observations refused outside agent mode. References are valid
only for current full epochs. Parent must authorize upload/disclosure under Voyage
policy; worker does not prompt locally per click. Human private data never enters
journal (only request hash and outcome code), agent response, or other peers.

Replies to effects include current `status` alongside `value`. On duplicate IDs,
`result:{receipt,content_withheld:true}` instead of cached secret observations.
Failure after dispatch returns `outcome_unknown`, not safe-to-retry evidence.
Receipt states: `dispatched`, `completed`, `refused`, `unknown`.

Security boundary: Chromium sandbox is mandatory, private Playwright CDP pipe;
separate task and trusted encoder Chromium processes/profiles. Origin/DNS-pinned
proxy and request interception are defense-in-depth, **not an OS egress sandbox**.
Production supervisor must enforce network isolation against Chromium compromise
and resource/disk quotas; the worker does not claim Chromium flags alone provide
that. RTC intentionally makes network connections to authenticated viewers/TURN;
parent must authorize recipients and network deployment. Native platform and
remote/TURN qualification remain separate from synthetic loopback decode tests.

## Integration changes from draft / protocol adapter

Worker JSON is a private parent adapter protocol, not `HostBrowserOperation`
serde. Parent maps attachment ID to `viewer`, browser_id to `browser`, selected
UUID to `status.tab`, and the four wire epochs to worker document/viewport/control/
capture epochs; worker additionally has a numeric tab epoch. Parent retains the
Voyage incarnation separately. Rust `RequestOffer`/`Answer` become worker
`offer`/`answer` with parent-owned `signal_seq`. No trickle ICE support: reject
wire `Ice` explicitly rather than silently accepting it. Translate wire Pointer
pressed/button to move/down/up; Scroll delta_x/delta_y to x/y; Tab to tabs.
Never use untrusted `config`, `policy`, executable or filesystem roots.

Public browsing is dynamic by default only when trusted init sets public_web=true.
Private/localhost requires an explicit exact origin grant; there is no implicit
localhost bypass. WebSockets use Chromium's authenticated CONNECT proxy; service workers remain
blocked. CONNECT is authorized as an HTTPS origin, even for Chromium's `ws://`
tunnels. A private `ws://host:port` therefore also needs the explicit trusted
`https://host:port` grant; an HTTP grant alone is insufficient. This intentionally
does not broaden private-network permission or infer grants from page origins.
Public WebSockets require public_web or an explicit HTTPS origin grant. Proxy
CONNECT cannot inspect encrypted paths/messages: origin/address containment, not
application-message filtering. Policy replacement destroys existing tunnels. Task-page WebRTC is disabled by init script plus proxy UDP policy;
this is defense in depth, not protection from a compromised Chromium. Encoder
has no task content and uses real host ICE candidates (mDNS obfuscation disabled
only in the separate encoder process). Parent must authorize candidate disclosure;
TURN credentials never belong in agent-visible config or logs.

An offer is needed again after control, tab switch or policy changes because all
old peers are closed. Private disconnect keeps the controller identity and private
mode. No automatic return. Control acknowledges only after encoder reset, capture
stop and draining current bounded JPEG delivery. It interrupts pending agent
observations immediately; previously sent website effects remain uncertain.
Input/dialog uses its own interrupt lane and cannot sit behind a blocked action.
Human input sequence counters survive control transitions within an admission.
All references are invalidated by epochs; navigation caused by an action may
advance document epoch before its reply. Use reply status, never predict epochs.

Downloads are capped at 2 MiB for returned data, at most eight retained in memory,
and are withheld/deleted on privacy transitions. Chromium can write a larger
transient download before completion: supervisor disk quotas remain required.
No arbitrary page evaluate action is exposed. CDP uses Playwright private pipes.
Browser profiles are Playwright temporary profiles, never user profiles.
Cleanup failure retains the root worker.lock and resource handles; owner must
reconcile, never delete that lock to replay uncertain effects.

Follow-up fencing: old agent/input effects remain tracked until actual settlement;
private control acknowledgement waits for them, not only their raced replies.
An effect that fails to settle within five seconds quarantines/closes the task
browser and refuses the transition acknowledgement. EOF/SIGTERM synchronously
blocks new admission/execution and fences before waiting for lanes; cleanup has an
eight-second process deadline. A deadline exit is unresolved cleanup, not proof
of descendant termination; supervisor process-tree containment remains required.
Queued shutdown arriving with EOF may return parent_disconnected rather than a
success receipt; orderly callers wait for the shutdown reply before closing stdin.

Element preflight refusal responses carry `error.state = refused` and a bounded
code; only stale references, hidden/disabled controls and noneditable fill targets
are recoverable by the parent without closing the worker. The worker records the
refusal before returning it. Other errors remain unknown, including failures after
a click or text effect. Inspect returns visible controls with labels/type metadata.

Join/offer/answer/disconnect allow document/viewport changes on the same tab and
capture/controller epochs; page input still checks every epoch. The encoder
refreshes current pixels for connected receivers, including static pages, and
clears them during privacy reset. Public IPv6 classification excludes IANA
2001::/23 protocol assignments and 2001:db8::/32 documentation space rather than
the entire 2001::/16. Mixed public/private DNS answers remain refused. See the
[IANA special-purpose registry](https://www.iana.org/assignments/iana-ipv6-special-registry).

IPv4 special-use checks restrict protocol-assignment and documentation space to
`192.0.0.0/24` and `192.0.2.0/24`, allowing public destinations such as IANA at
`192.0.43.8`. Both halves of benchmark range `198.18.0.0/15` remain denied. See
[IANA's IPv4 registry](https://www.iana.org/assignments/iana-ipv4-special-registry/).

Inspection marks controls obscured at their interaction point. Click/fill checks
that point before dispatch and returns a recoverable `element_obscured` refusal
when another element intercepts it. Dismissing overlays or scrolling is a separate
authorized interaction. Screenshots try bounded JPEG qualities to fit the current
Voyage output budget; they never increase that budget, and ingestion still refuses
images too large even at the lowest quality.

The encoder marks webpage content as detailed video and prefers preserving
resolution over frame rate under bandwidth pressure. Pointer mapping still uses
the authoritative CSS viewport and remains correct if a receiver adapts frames.
## Read-only observation failures

`inspect` includes `document.url` (without URL credentials) and `document.title`
before its larger control list in JSON previews. Inspection and screenshot errors with unchanged agent authority return
`observation_unavailable` / `refused`, withholding partial output and discarding
references while preserving the browser. This class does not apply to navigation,
clicks, fills, uploads, downloads or control/privacy fencing. Callers may issue a
new observation; they must not replay a prior external effect to recover a read.
