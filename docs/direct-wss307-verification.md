# Direct browser WSS and live conversation verification (#307)

## Implemented contract

- Laravel authenticates the user, checks tenant/session/connection state before
  and after minting, and performs public-only, DNS-pinned HTTPS bootstrap.
- Vessel mints single-use, memory-only 120-second credentials bound to the exact
  canonical HTTPS Helm Web Origin and existing grant authority. The first WSS
  frame authenticates before Hello or command dispatch. Native header-based
  sockets retain their original boundary.
- The browser connects directly to `/v1/vessel/browser-socket` using
  `voyage.vessel.v1`. A replacement connection opens before expiry; existing
  requests drain without replay. Stored pairing tokens never reach JavaScript.
- The web conversation subscribes to native public observation events, the same
  invalidation/projection mechanism used by the TUI. Assistant output, generating
  tool arguments, and provider-disclosed thinking/summary previews update from
  bounded projections. Canonical tool calls replace provisional previews.
  Gaps, malformed cursors, incarnation changes and reconnect trigger resync.
- Logout/removal prevents new minting, not already-admitted work. Existing
  credentials expire within 120 seconds; socket close can take up to 3 additional
  seconds for bounded writes/authorization. Removing a web connection does not
  revoke grants used by unrelated clients.

## Observed automated evidence (Linux, 2026-09-16)

| Check | Result and boundary |
|---|---|
| `npm test` in `web/` | 26 tests passed, including real PHP HTTP sessions/CSRF/tenant isolation, direct fleet routing/renewal, history/actions/decisions/reconnect receipts, live-output/tool/reasoning DOM updates, and malformed/gapped observations |
| `php tests/direct-bootstrap.php` | 67 checks passed: bootstrap contract, tenant/session races, origin/credential validation, removal/logout and preservation of long-term grants |
| `php tests/direct-http.php` | 14 checks passed: injected cURL option/body/header bounds, TLS verification, DNS pinning, proxy/redirect refusal; not live public TLS |
| `php tests/oauth.php` | 80 offline OAuth checks passed; not live consent/login |
| `cargo test -p vessel --locked -j 8 process_http::browser` | 14 tests passed, including production HTTP/mint/auth/duplex paths against a simulated supervisor, native socket regression, pre-auth timeout/refusal, Origin mismatch, credential reuse, revocation/authority change and expiry |
| Workspace Rust coverage | 1,663 passed, 0 failed, 1 ignored; compact measurement in `coverage/latest.json` |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy -p vessel --locked --all-targets -j 8 -- -D warnings` | Passed |
| Production Vite build | Passed; manifest entries resolve to emitted assets |
| PHP lint / route listing / Composer validation | Passed |
| Documentation links / whitespace | Changed local Markdown links resolve; `git diff --check` passed |

Raw logs, HTML and detailed coverage are retained under ignored
`target/direct-wss-307` and `target/coverage-report/direct-wss-307`.
The default shared-target coverage report contains historical-object diagnostics;
its mixed totals are **not** published. The corrected export selects all 16 current
Cargo workspace executables (including all executed tests). Reused unchanged
installer/protocol/storage source paths are validated byte-for-byte against the
previous measurement's source commit. Standard test-source exclusions apply to
both current and reused source roots, rather than accidentally counting old-root
tests. No current executable or production source is omitted to improve coverage.
Line coverage is 75.40285% (76,647/101,650), up from 75.14596%; functions
72.05747% (7,071/9,813), regions 71.99466% (119,784/166,379).

## Measurements

[The reproducible synthetic comparison](direct-wss307-benchmark.md) uses five
Vessels per user, 1/4/10 concurrent users and three repeats per transport. It
validated 45,000 replies. At ten users the legacy relay alone used median
1,183 ms CPU, 142 MiB sampled peak RSS, and approximately 100.5 MB in and out.
Direct-mode conversation traffic bypassed the mock hosted bootstrap process.
The hosted bootstrap fixture is **not PHP**: these numbers do not establish actual
Helm Web deployment savings, production capacity, or watts per user.

## Still required before closing the full issue

The local shared-browser tool refused access because no browser was shared. No
live-provider calls, live OAuth consent, production service restarts or gateway
shutdown were performed by this delivery. A real controlled deployment still needs:

1. Upgrade its Vessel and Laravel/assets, retain existing pairing credentials,
   and connect from a browser with trusted TLS. Inspect WSS Origin/authentication
   and confirm no credentials in URLs and no cross-origin HTTP bootstrap.
2. Exercise catalogue, create/open, account/model selection, history, output,
   submit/steer/decisions/cancel and live tool/assistant updates. Use an approved
   provider/budget only where real model execution is required.
3. Exercise malformed/expired credentials, disallowed Origin, insufficient
   underlying grant scope, revocation, removal/logout deadlines, offline Vessel,
   renewal and reconnect receipt recovery with uncertain effects never replayed.
4. Stop the legacy Node conversation gateway in that controlled deployment and
   repeat workflows; capture browser network evidence that conversation traffic
   goes directly to the Vessel.
5. Measure actual hosted PHP/web-server CPU, RSS and ingress/egress before/after
   the same workload, including five Vessels per user and multiple users. Keep
   fixture observations separate from real deployment counters.

These are explicit remaining acceptance checks, not passing or silently deferred
claims. The implementation can be published without claiming the whole issue's
live-deployment acceptance is complete. See [migration steps](helm-web.md#upgrade-and-retire-the-conversation-gateway).
