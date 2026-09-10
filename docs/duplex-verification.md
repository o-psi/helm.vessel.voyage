# Focused duplex verification — 2026-09-10

Scope: #235, isolated `feature/browser-duplex-transport`, based on `69343e1`,
including `628d0ba`, `e945e05`, `34ca74c`, `324834d` and the subsequent socket
lifecycle/resource-bound fixes. These are targeted Linux observations, not a
restored general test suite or a deployed-browser acceptance claim. Browser #234
is integrated and verified by its owning voyage; #198 retains deployment gates.

## Build and source checks

- `cargo build -p helm -p vessel -p voyage --bins --locked`: passed.
- `cargo clippy -p helm -p vessel -p voyage-protocol --lib --bins --locked --no-deps -- -D warnings`:
  passed for affected packages.
- Dependency-inclusive strict Clippy was blocked by the existing
  `voyage/src/provider/mod.rs` `ProviderStreamEvent` large-enum warning. It was not
  reported as passing and unrelated provider code was not changed here.
- Owned Rust formatting, manifest metadata, local documentation links and Git
  whitespace checks were checked separately. No release build or paid provider
  was used. A stale shared-target protocol artifact required refreshing the
  checkout's protocol source mtime; no cache cleanup or new target tree was used.

## Actual local and scoped socket journeys

A private isolated HOME/XDG fixture ran the built Helm, Vessel and independent
voyage binaries, an authenticated loopback gateway and a local synthetic streaming
provider. No operator provider configuration or credentials were used. New fixture
identities were allocated for each run; uncertain prior effects were not replayed.

Observed passing checks:

- Local upgrade and two simultaneous clients with distinct socket identities;
  bad bearer and browser Origin refusal.
- Two independent session owners; correlated ordinary replies continue while an
  ordinary long-poll observation is pending. Events share the command socket.
- A running submit's exact durable ID/payload returns its original run; changing
  its payload is refused. Closing clients leaves the run alive. A fresh socket
  observes the receipt and no second provider dispatch occurs.
- Actual human PTY attach, write and snapshot travel through the socket. A unique
  synthetic private input marker appears in the private terminal snapshot but not
  in canonical history.
- Session-scoped and workspace-paired sockets, actual Helm CLI with each credential
  format, denied cross-session access, wrong identity pin refusal and required
  workspace pin. Pairing remains explicit HTTP bootstrap, not a browser relay.
- Idle grant expiry and revocation terminate sockets; expired/revoked credentials
  cannot reconnect. Event replay and explicit run cancellation remain distinct
  from disconnect.
- Malformed/private-forwarding/oversized frames close the offending socket while
  another client continues. Actual `helm connect ... run` streams its answer and
  finishes with exactly one synthetic provider request.
- Cleanup is separately awaited: each newly created fixture session publishes
  `stopped.json` with `cleanup_observed: true`. Stop admission alone was not treated
  as cleanup. This does not attest to resources retained from an earlier interrupted
  operator run.

The final full journey log and summary are retained locally at
`.local/duplex-verify.log` and `.local/duplex-verification-summary.json`; its fixture
was `/tmp/duplex-235-u7wkhtk1`. Earlier `/tmp` evidence disappeared across the host
restart; the old pass log is not evidence of present cleanup or surviving processes.

## Targeted transport probes

Temporary, uncommitted probes exercised the real Helm Client against a controlled
socket peer and the real Vessel duplex engine against a controlled Backend:

- Client clones and separately constructed clients with the same route/activation
  share one socket. A single reverse consumer accepts typed notifications; missing
  consumers refuse; stale reverse replies cannot migrate after disconnect.
- Ordinary commands continue while reverse acknowledgement is pending. Failed
  effect replies remain unknown and are never resent. Monotonic loss generation
  survives coalesced reconnect notifications.
- An unconsumed event queue closes the socket rather than silently losing events.
  Explicit disconnect retires all activation clones; a new activation can connect.
- The probe exposed an immediate reconnect race: the loss notification could arrive
  before the old actor stopped accepting work. The fix retires its handle before
  publishing loss. Ten repeated client probe runs passed after the fix.
- The actual server reverse hook delivers `BrowserWork`, receives its correlated
  acknowledgement and invokes disconnect notification. A silent raw WebSocket peer
  expires under heartbeat bounds; another client's control requests progress while
  an event consumer stalls.
- HTTP 200 without upgrade, redirects and HTTP 503 fail closed in actual Helm;
  no POST command or SSE fallback and no redirect following occurred.

Local probe sources/logs and hardlinked verification binaries remain under `.local/`;
no deleted scripts/tests were recreated or staged. The server Backend probe is not
an authentication substitute: actual authentication/scoping was exercised in the
separate local/scoped gateway journey above.

## Remaining boundaries

Loopback WS does not establish deployed WSS certificate/proxy behavior, native
macOS/Windows operation, external security certification or live-provider quality.
The browser owner is responsible for combined browser/runtime consent and effect
verification and coordinated main publication. Transport commits are not permission
to overwrite concurrent account/UI work or to attest retained unknown cleanup.
