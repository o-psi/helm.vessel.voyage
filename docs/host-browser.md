# Voyage-owned browser (#333)

Status: implementation in progress for v1.0.2; not a statement of delivered behavior.
[Issue #333](https://github.com/o-psi/helm.vessel.voyage/issues/333) owns implementation,
verification, publication and follow-up audit. Existing local-browser behavior is
recorded in [current state](current-state.md) and [local browser](local-browser.md).

## Product contract

Helm includes both Helm TUI and Helm Web. A voyage opens its own browser on the
Vessel host when needed. Both clients attach to that same browser. Switching a
view never changes browser ownership; disconnecting Helm is not cancellation.
No separate local companion, per-click local consent ceremony, or manual origin
configuration is the default workflow. Existing execution policy still applies.
Private login, policy-required consequential approvals and actual capacity failures
are meaningful interactions, not reasons to expose transport setup machinery.

Voyage owns the browser worker and cleanup. Vessel supervises/routes authorized
commands and accounts for capacity; it does not run an embedded browser agent.
Each voyage has isolated browser state. Multiple authorized viewers may watch;
only one human connection or the agent controls the browser at a time.

## Transport and authority

The worker controls Chromium over a private pipe. Raw CDP is never a public API.
A structured page description serves agent actions and text presentation; Helm
renders video from the real page, never executes a copy of website scripts.
WebRTC carries media; the authenticated Helm–Vessel socket carries typed signaling
and controls. WebRTC does not automatically traverse a WSS reverse proxy.
Direct, TURN-relayed and fallback paths need actual qualification.

The server supplies socket provenance. User identity alone cannot distinguish two
windows belonging to the same user. Viewer bindings include the principal, socket,
voyage incarnation, browser instance, tab/document/viewport, and control/capture
epochs. Delayed input is refused, never retargeted. Authority revocation fences
existing media as well as future commands. Private input excludes the agent and
other viewers before acknowledgment; losing the private controller never silently
returns the page to the agent. Already delivered frames cannot be recalled.

Browser action identity is recorded before effects. Timeouts preserve uncertainty;
reconnect must not repeat a website action. High-frequency input/heartbeat handling
must not exhaust a permanent lifetime command budget. Process closure, website
action outcome and unresolved cleanup are distinct states.

## Deployment and resources

Browser state, login cookies and downloads reside on the execution host. Private
input is hidden from model history, not from the host administrator. Existing
personal-browser cookies are not imported. Localhost means the Vessel host.
Chromium sandboxing, private storage, bounded tabs/transfers/media, explicit network
policy and process/resource accounting remain necessary. Stop video when nobody
watches; do not stop unrelated browser work. Retiring resources count until cleanup
is observed. A running page is not promised to survive process or host failure.

## Evidence required before completion

- Actual decoded WebRTC video, readable text and measured interaction performance.
- Both Helm client journeys: navigation, typing/IME, scrolling, resize, tabs including
  last-tab recovery, website dialogs, uploads/downloads, private handoff and cleanup.
- Multiple voyages and viewers, stale input, socket loss/revocation, private-view
  exclusion, slow viewers, process crashes and capacity exhaustion.
- Direct and relay deployment evidence; fallback behavior and resource measurements.
- Focused local tests, applicable workspace Rust coverage, packaged distribution,
  hosted build follow-through and independent follow-up audit.

The early local Chromium/JPEG/canvas/VP8 experiment delivered decoded frames to two
synthetic viewers. It only establishes feasibility: latency tails, production
networking, controller privacy and both-client integration were not qualified.
No stable release or native-platform certification follows from that experiment.

## TUI presentation and comparative UX

The user explicitly does not require interactive browser video inside the terminal.
Helm TUI presents status/activity and opens the shared authenticated graphical
viewer in the ordinary browser. Helm Web embeds that viewer alongside conversation.
Both attach to the same Voyage resource; no separate browser/session or manual
connection details are introduced by opening a view.

Read-only T3 Code source research at commit
`7445aa733ada33e45289e5aa5055f79142556513` informed the integrated task-panel direction:
[preview tools](https://github.com/pingdotgg/t3code/blob/7445aa733ada33e45289e5aa5055f79142556513/apps/server/src/mcp/toolkits/preview/tools.ts)
and [browser surface](https://github.com/pingdotgg/t3code/blob/7445aa733ada33e45289e5aa5055f79142556513/apps/web/src/components/preview/addBrowserSurface.ts).
This is not hands-on validation. Its inspected preview is desktop-only; its brief
human-input interruption does not establish private capture suspension. Borrow
integrated navigation and activity presentation, not those implementation limits.

## Executing-host prerequisites and development configuration

The worker distribution is installed beside the executable at
`share/voyage/browser/worker.mjs` with its pinned Playwright dependency. The default
lookup uses host Node and Chromium executables. Helm viewer machines need neither.
A host may override paths and network/media settings through its private launch
configuration, never through model tool arguments or portable start settings:

```json
{
  "host_browser_launch": {
    "node": "/usr/bin/node",
    "worker": "/absolute/release/share/voyage/browser/worker.mjs",
    "chromium": "/usr/bin/chromium",
    "config": {
      "public_web": true,
      "origins": [],
      "ice_servers": [],
      "relay_only": false,
      "width": 1280,
      "height": 720
    }
  }
}
```

Private-network access is a host policy setting, not a per-click companion prompt.
For a permitted development origin, add an exact origin entry with
`private_network: true`. No TURN service is assumed; credentials belong only in
private host configuration and authorized signaling, never diagnostic output.
The existing `sandbox.mode = "required"` currently refuses this worker rather
than silently launching outside the required OS sandbox. This is a real remaining
integration limit, distinct from Chromium's own mandatory sandbox.

Worker profile temporary paths are short and private because Chromium's Unix
socket pathname limit cannot accommodate long Voyage journal paths. Their cleanup
is observed; uncertain descendant cleanup retains capacity/evidence rather than
claiming the browser closed. Browser startup does not install system packages.

Private mode is an **agent/history/other-viewer exclusion boundary**, not a ban on
the trusted host's media processing: private pixels must pass through capture and
encoding to reach the private controller. They are memory-only media, not model
tool results or recordings. Task JavaScript is not executed inside the encoder;
task-page pixels necessarily are processed there. Closing old peers and clearing
queued frames precede private acknowledgment. No claim is made that host operators
cannot access private input or that already delivered frames can be recalled.

If a deployment needs receiver-side TURN, an explicit
`config.viewer_rtc_configuration` uses the browser API shape (`iceServers` with
TURN `urls`, `username`, `credential`, plus `iceTransportPolicy`). It is sent only
in the authorized offer response, not status/history; use credentials specifically
intended for viewers, never an administrator TURN secret. Encoder `ice_servers`
are not copied implicitly. Direct ICE discloses host candidates to authorized
viewers; relay-only deployments should configure both policies deliberately.
Public NAT/TLS deployment qualification remains distinct from local relay tests.

### Crash qualification finding

The explicit crash fixture found that Node SIGKILL left private profiles and its
lock; hanging Chromium before EOF/SIGTERM left live crashpad descendants across
multiple process groups. Parent-process-group kill is insufficient. A separate
Linux subreaper guardian now owns this cleanup; see the verification below. Failed crash tests are retained as failures; normal EOF success
does not supersede them.

The Linux worker is now launched under a separate Python subreaper guardian.
It observes/reaps Chromium descendants across process groups after Node failure
or loss of Voyage stdin, removes only its owned short private temporary tree, and
writes metadata-only cleanup evidence. Runtime resource release requires that
positive evidence; website action receipts remain unknown after interruption.
A killed guardian or unavailable proof is still unresolved, never guessed cleanup.

Final guardian adverse checks (Linux synthetic) now pass for Node SIGKILL, hung
Chromium with EOF, hung Chromium with TERM, and normal explicit shutdown. Each
fixture verifies zero live descendants/zombies/sockets/profiles and preserves an
unrelated process. The original failed crash evidence is retained; the passing
result applies to the new subreaper path, not the removed process-group assumption.
The integrated guardian process journey also passes native and Web suspended-owner
preparation and cleanup. SIGKILL of the guardian itself remains unresolved without
external guardian evidence, not a successful cleanup claim.

The React preview introduced concurrently in #336 now wraps the same viewer and
adapter, with attachment cleanup on task/socket changes. Its focused lifecycle
suite and production build pass; the full process journey exercises the shared
viewer/adapter and native launchers, not every React layout interaction.

Rust workspace coverage after final Rust edits: 1,734 passed, zero failed, two
ignored. Corrected current-artifact report records lines 75.2771%, functions
72.0288%, regions 71.9199% (previous 76.1854/72.8644/72.8350). The approximately
0.9 percentage-point drop reflects new runtime paths; no measured scope was
narrowed. JS/Python/browser journeys are separate evidence, not Rust coverage.
Mixed shared-target report was retained; only all 16 current Cargo executables
were used for the published summary, with no LLVM export diagnostics.

Linux distribution verification used stripped **development** binaries (not an
optimized release claim), packaged under ignored target with worker/dependency
hash inventory. Namespace-isolated installer checks passed install, upgrade,
status, executable hashes and worker/guardian/Playwright sidecar preservation;
host services were untouched. The unstripped development package initially exceeded
bootstrap size bounds and was refused; limits were not weakened. Hosted nightly
optimized artifact inspection remains a separate delivery gate.
