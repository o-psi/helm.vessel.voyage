# Helm local browser adapter

This directory is the **local** half of issue #234. It contains no Vessel client,
provider client, agent loop, remote browser endpoint, or remote pairing mechanism.
The parent Helm process routes typed browser requests over its existing authenticated
full-duplex Vessel connection and launches this helper over **private stdio**.
Do not expose the stdio protocol or loopback companion to a network listener.

## Dependencies and setup

Runtime assets: `helper.mjs`, `security.mjs`, `index.html`, `app.js`, `app.css`,
`package.json`, and `package-lock.json`. `probe.mjs` and `cleanup-probe.mjs` are scoped synthetic probes,
not runtime assets or a replacement general test suite.

- Node.js **24 or newer** (Linux evidence: Node 26.8.2).
- `playwright-core` **1.63.0**, pinned in both manifests. The npm registry version
  and package integrity were checked during implementation.
- Locally installed Chromium, default `/usr/bin/chromium` (Linux evidence:
  Chromium 152.0.7977.82). Playwright does **not** install a browser here.
- Chromium's sandbox must work. This helper explicitly enables it and never
  silently substitutes `--no-sandbox`. A broken sandbox is a setup failure.

Install only through an explicit local operator setup step, in the adapter's
private installed asset directory:

```sh
npm ci --ignore-scripts --no-audit --no-fund
node helper.mjs
```

The second command waits for private JSONL input; it is not a public server CLI.
Helm's manager supplies the profile path and opens the returned companion URL.
The helper itself never installs dependencies or launches a shell. Native macOS
and Windows execution/security have **not** been established by these Linux checks.

## Private stdio contract

One JSON object per line, maximum 3 MiB. Requests have `id` (ASCII letters,
digits, `_` or `-`, 1–128 bytes) and `op`. Replies are:

```json
{"id":"request-id","ok":true,"result":{}}
```

or `{"id":"request-id","ok":false,"error":{"code":"stable_code","message":"sanitized message"}}`.
No page/process exception text is emitted. The only stderr diagnostics are fixed
internal-failure notices. Stdout is exclusively private protocol output.

Unsolicited local-manager events:

- `{event:"control",browser_id,epoch,capture_epoch,mode,shared,ready,pending,connected}`
- `{event:"approval",request_id,pending:true|false}`

| Operation | Fields and result |
| --- | --- |
| `init` | `session_dir` absolute, private, owned directory; optional `browser_id`, `executable_path`, `label`, `binding`, `heartbeat_ms`, `limits`. Returns status, limits and **local-secret** `companion_url`. |
| `status` | Returns `browser_id,epoch,capture_epoch,mode,shared,ready,pending,connected`. Never contains private page content. |
| `heartbeat` | Optional current `binding`. Refreshes liveness, not authority. A changed owner/executor identity fences sharing. |
| `control` | `mode:"private"` or `"human"`. Immediately advances both epochs and fences queued actions and capture. `agent` is refused over stdio. |
| `action` | `id` is the request UUID; `epoch,capture_epoch,action_sha256,action`, plus `binding` when initialized with one; optional `expires_at_ms`. Takes the exact typed protocol `BrowserAction`. Returns `BrowserResult` below. |
| `receipt` | `request_id`; returns `{receipt:null|receipt}`. No content replay. |
| `cancel` | `request_id`; returns `{receipt,cancellation_requested}`. A requested cancellation is not proof that a dispatched effect did not happen. |
| `shutdown` | Fences, closes Chromium/proxy/HTTP, removes session staging, retains profile/receipts, replies `{closed:true}` and exits after stdout flush. |

`BrowserBinding` carries session/incarnation/run/browser/resource/executor identity,
controller/capture epochs and lease expiry. The parent may heartbeat an idle binding
with `run_id:null`; the first admitted action pins its non-null run. A different
run fences and requires new local sharing. An owner/incarnation/resource/executor
change cannot renew old actions. The parent must heartbeat **only while its pinned
full-duplex connection and grant remain live**, typically every second. The default
miss threshold is five seconds. The manager must consume each `control` event and
publish the helper's new controller/capture epochs through the existing duplex
binding handshake before admitting another action. In particular:

1. Initialize with the selected owner binding and `run_id:null` while idle.
2. Local Share advances the epochs; publish those epochs, then send the first
   actual action with `run_id:Some(UUID)` (a UUID string in JSON).
3. Idle `run_id:null` heartbeats do not erase the pinned run. A later non-null run
   fences; notify the local human, require explicit Share again, and publish the
   resulting epochs. Do not retry an old effect under a new request identity.

The local companion displays the actual bound session/run IDs alongside the
manager label. This helper probe establishes the binding semantics, **not** the
parent's end-to-end duplex publication/reshare handshake. EOF and signals fence and clean up, not reconnect.

`BrowserAction` uses the `action` discriminator:

- `inspect {page_id:null|UUID}`
- `navigate {target,url}`
- `click {target,element}`; `fill {target,element,text}`
- `scroll {target,delta_x,delta_y}`; `screenshot {target}`
- `tabs {operation:{operation:"list"|"open"|"select"|"close",...}}`
  (`open` has `url`; select/close have `target`)
- `upload_prepare {transfer_id,name,mime_type,data_base64}`
- `upload {target,element,grant_id}`; `download {target,download_id}`

A target is `{page_id,observation_id}`. Inspection yields bounded untrusted page
text plus explicit fresh element refs (`element`/`ref`). References expire after
30 seconds and are invalidated by navigation, effects and authority changes.
Element identity and its descriptive signature are checked again before effects.
There is no remote JavaScript, CDP, browser launch flag, shell, or local path action.

`BrowserResult` is `{request_id,action_sha256,state,text,page_id,observation_id,image,file}`;
nullable fields are explicit. States are `completed`, `refused`, `cancelled`, or
`unresolved`. `image` is `{mime_type:"image/jpeg",data_base64}` and `file` is
`{name,mime_type,data_base64}`. Raster bytes are actual Chromium JPEG output, never
an invented image or a filesystem reference, capped at 2 MiB. Text is bounded
observation JSON or a content-free error code. Duplicate requests return their
state and `duplicate_content_withheld`, never redo the effect or replay content.

## Local consent, privacy and network policy

The companion uses the **same persistent Chromium context** as agent actions.
Its viewport is a live, transient raster stream with pointer down/up/move, wheel,
keyboard and text/IME forwarding (128 pending input events maximum), not a screenshot-only viewer. Local controls
include tabs, navigation, takeover, explicit sharing, origin grants, one-effect
approval prompts, upload picker, downloads, clipboard paste and page dialogs.
Human input is refused in agent mode. Focus preserves viewport coordinates; held
keys/buttons are released on authority changes and local-window blur. Pending dispatched automation must settle
before new local input is accepted; takeover immediately fences captures/queues,
not a claim that an already-dispatched browser/network effect was undone.

Private and human modes never send DOM, page titles/URLs, frames, dialog text,
console/network data or human input over stdout. Frames stay in transient local
memory. No tracing, video or screen recordings are enabled. Returning to agent
mode requires explicit local consent and fresh inspection; old grants/refs do not
inherit a new epoch. The share dialog warns that the **entire shared browser**,
including signed-in visible content, can become remote model/history input.

Each local approval shows current/destination origin, bounded untrusted element
description, non-password fill value, or file name/size/MIME as applicable. These
are rendered as text, not HTML; page content cannot approve permissions. Approval
stdout events contain only request identity and pending state, never that content.
All agent effects require one independently confirmed local prompt, even when
remote policy already approved them. Prompts expire (default 30 seconds). No
controller means refusal, not an unattended indefinite approval wait. The local
controller has a five-second lease; other tabs cannot steal it while live.

The companion listens only on ephemeral `127.0.0.1`. The local launch secret is
in a fragment, consumed into an HttpOnly/SameSite=Strict cookie, and removed from
the address bar. **Never forward its init URL to Vessel, model, conversation,
logs, argv or network pairing UI.** Reopening with a consumed fragment requires
the existing same-origin cookie. JSON endpoints require exact Host and Origin,
a same-origin CSRF header, and JSON content type. Query strings are refused.
Responses use no-store, no-referrer, nosniff, restrictive CSP, frame-ancestors
none, and same-origin resource/opener policy. No request logs are emitted.

The helper starts with **zero origin grants**. Only the companion can add exact
HTTP(S) origins, including a separate explicit private-network grant. Browser
routing checks every resource/redirect; a private authenticated loopback proxy
resolves DNS, classifies every returned address and connects to that exact IP.
It does not merely validate DNS and let Chromium resolve the host again. Private,
loopback, link-local, reserved and conservatively classified IPv6 addresses are
refused unless the exact origin has a locally reviewed private-network grant.
Revocation and permission downgrades tear down proxy connections, including HTTP
upstream sockets; pending DNS resolution rechecks that the grant still exists. Service workers, WebSockets, QUIC and
non-proxied WebRTC are disabled. These choices deliberately break sites needing
those facilities; enabling them without equivalent network enforcement is not a
supported workaround.

## Files, resources and receipts

The root, profile, receipts and staging directories must be private and owned.
The dedicated profile has one exclusive executor lock. A clean shutdown permits
reuse. A crash lock is **not automatically removed based on PID or age**; an
operator must first establish the old executor/browser is gone. Retained queued
receipts become cancelled-before-dispatch and dispatched receipts become unknown;
neither is replayed. There is no claim that a dead process survived restart.

Cleanup retains one close promise, including rejection: overlapping/repeated
shutdowns cannot report success while the first close is pending or failed.
`closed:true` is emitted only after Chromium context close and owned helper
resources are observed closed, and staging/lock cleanup succeeds. A rejected
Chromium close leaves staging and `executor.lock` intact and returns
`cleanup_unresolved`. **Helper process exit alone is not positive browser cleanup
evidence**, including an exit after EOF/signal or a forced kill; the parent must
not turn a missing/failed graceful reply into a cleanup attestation.

Receipts are atomically replaced, file/directory fsynced before effects. They
retain the exact canonical helper-request SHA-256, remote action digest, ID,
epoch, timestamps and state/code—not URLs, fill text, names, DOM, images, secrets
or transfer bytes. Unknown effects stay unknown. There is no automatic receipt
pruning to make unresolved work disappear. Limits refuse new work instead.

Defaults: 8 tabs (maximum 16), 16 pending agent requests, 10,000 retained receipts,
8 staged uploads and 8 downloads, 2 MiB per disclosed file/image, 64 exact origins,
15-second browser-operation timeout, 30-second prompt timeout, 1280×720 viewport.
One raster capture runs at a time. Each controller is capped at ten frame requests
per second, and HTTP connections/headers/bodies are bounded. A bounded overall
action deadline fences and closes Chromium rather than letting a stuck page hold
execution indefinitely. Parent shutdown remains responsible for process-level
cleanup supervision when the browser itself cannot exit.

Profile/cache accounting includes the entire session root, defaults to 256 MiB
(configurable 32 MiB–1 GiB), rejects an over-budget start, and checks every second.
Chromium disk/media caches are each configured to 16 MiB. A detected disk/entry
budget violation fences and closes Chromium. Download staging is monitored every
100 ms, cancelled on aggregate over-budget/30-second timeout, and rejected above
2 MiB before retention or disclosure. **These are application-level budget checks,
not hard filesystem quotas:** in-flight browser writes may overshoot between
checks. An OS/filesystem quota is required for an exact host disk ceiling. This
limitation must not be described as sandboxed hard disk enforcement.

Uploads come from the human picker, or from `upload_prepare` bytes after local
approval; remote paths are never accepted. Each staged upload additionally needs
an explicit companion grant for the current sharing epoch and a confirmed upload
effect. Safe filename and MIME metadata are preserved in the website file payload;
local filesystem paths are not. Direct local file-chooser uploads consume their
staging slot after delivery. Taking local control also enables discarding staged
uploads/downloads, so the bounded pools need not remain full until shutdown. Inspection lists only explicitly granted IDs. Downloads remain private;
local save and remote disclosure are separate controls. Disclosure requires both
a local file grant and an action confirmation. Files are never auto-opened or
executed. The current Voyage broker requires nonempty disclosed file bytes; empty
downloads can be saved locally but return `empty_download_local_save_only` for
remote disclosure (rather than sending an invalid broker result). Names are stripped
of control/path characters and bounded to 128 UTF-8 bytes. Local saves use a browser download dialog and the safe default filename
`download.bin`; the chosen destination is not revealed to the agent.

## Actual verification and remaining limitations

From a configured checkout:

```sh
node --check helper.mjs
node --check security.mjs
node --check app.js
node --check cleanup-probe.mjs
HELM_BROWSER_PROBE_ROOT=/absolute/private/evidence/root node probe.mjs
HELM_BROWSER_PROBE_ROOT=/absolute/private/evidence/root node cleanup-probe.mjs
```

The scoped probe uses synthetic loopback content and real sandboxed Chromium,
without Cargo, providers or Vessel. It checks local network permission, HTTP
security headers and rejections, private refusal, controller exclusion, real
companion frame rendering/input transport, typed inspection and raster output,
local deny/allow and prompt expiry, durable duplicate/conflict handling, stale
references, actual human-picked and remotely staged upload bytes reaching a website,
local save versus separately approved download disclosure, keyboard input reaching
the viewed page while absent from private stdout, concurrent screenshot/control
fencing, takeover after a navigation is durably dispatched, idle/first/later run
binding and explicit reshare, heartbeat loss without renewal/replay, clean restart
receipt recovery, refusal of an untouched unexpected executor lock, private file
modes, and live disk-budget shutdown. Synthetic dispatched/queued receipts are
explicit fixtures for restart-state checks, not claims of recovered live effects.
The cleanup probe instruments only a private copy, never production assets: it
checks overlapping shutdown and a rejected close promise, including failure
retention after helper exit. Its rejection is synthesized after actually closing
Chromium, avoiding a deliberately orphaned browser while testing the exact
failure boundary. Positive fixture quiescence is not a claim that arbitrary
production close failures are harmless. Its retained profile/receipts
are private verification evidence, not publishable artifacts.

This is not native macOS/Windows verification, a live website compatibility
certification, a remote transport end-to-end check, or a guarantee against a
malicious local same-user process/compromised Chromium. Complex canvas sites,
accessibility trees, touch gestures, OS-native dialogs, drag/drop and all desktop
IME variants are not certified. The viewport is a bounded approximately 10-fps
JPEG interactive view, not audio/video streaming. Clipboard export from the page
to the host is not implemented; paste requires local confirmation and the host
browser's clipboard permission. The agent cannot accept page dialogs; a local
human must take over. Application policy is not an OS sandbox or a claim that
external effects can be rolled back after dispatch.
