# Helm Web: private Livewire console

Issue [#289](https://github.com/o-psi/voyage/issues/289). This is a browser-native
client, **not Ratzilla** and not another execution runtime.

The console lives at `/`. Signed-out visitors are redirected to `/landing`, the
public product page with a Sign in link. Successful login returns to `/`; logout
returns to `/landing`. Legacy `/console` bookmarks redirect to `/`. Authentication,
ticket and socket endpoints retain their `/console/` paths.

## Implemented scope

- Laravel 13 / Livewire 4 / Flux shell, password login, configured Vessel selector,
  voyage search and responsive conversation layout alongside the public site.
- JavaScript owns the `wire:ignore` conversation region: sanitized Markdown,
  selectable text/code, expandable tool/attachment metadata, previous-history
  pages, full-message expansion, ordinary textarea with IME-aware Ctrl/Cmd+Enter.
- Existing voyages: snapshot/history, submit when idle, steer when running,
  approval allow/deny, offered/custom question answers, cancel the observed run.
- Canonical history stays separate from provisional live output. Snapshot and
  decision reads run at most once a second while visible, serialized with actions;
  catalogue refresh is every ten seconds. These are bounded requests over a
  persistent WebSocket, **not token push subscriptions or Livewire per-token updates**.
- Full message expansion is capped at 4 MiB with an explicit native-client handoff.
  Large run output uses server UTF-8 byte continuations and a Load more button.
- A fresh authenticated connection, current snapshot and no unresolved command
  intent are required to act. Revisions/incarnations/run IDs fence mutations.
  Browser localStorage holds one identity-only record per intent **before** send.
  Storage failure disables dispatch. Separate keys avoid cross-tab array overwrite.
  Reconnect reads receipts only: no mutation retries, replacement IDs or inferred
  success from process liveness. Acknowledgement is not execution completion.

This first scope operates **existing** voyages. Create a voyage in native Helm.
No web uploads, private terminal, shared-browser execution, account enrollment,
provider settings, archive/delete or administrative/grant editor is exposed.
No React, Tauri, PHP agent loop, extra canonical history store or provider proxy.
There is no automatic gateway startup or grant enrollment.

## Trust and authentication

The web host is trusted with configured Vessel grant tokens. A compromise of this
host can exercise those grants: provision least privilege, not unrestricted admin
credentials. Provider credentials remain on each execution host. Vessel/Voyage
retain authoritative scope checks, local policy, approvals and execution ownership.
Browser logout, backgrounding and disconnect **do not cancel voyages**.

Console login is one operator password hash, not public registration or multi-user
authorization. Five failed attempts per source IP lock login for five minutes.
CSRF applies to login, logout and ticket requests; session ID rotates at login,
logout invalidates it, and authentication has a fixed eight-hour maximum. Use a
server-side session driver. All console responses are no-store and deny framing.
Livewire persists the console middleware on subsequent updates and the component
checks authentication during boot. Debug output must remain disabled.

Laravel signs single-use, audience-bound, 60-second gateway tickets. The browser
sends a ticket in its first WebSocket frame, never a URL. Node pins the exact web
Origin, configured alias, Vessel UUID and public subprotocol, and injects private
Vessel headers server-side. Fresh ticket renewal occurs every 30 seconds. After
logout an already admitted socket may remain usable for **up to 60 seconds**;
no renewal is possible with the invalidated session. Revoke the Vessel grant for
immediate authority revocation. Rotate the HMAC secret and restart the gateway to
invalidate all web connections. Restart introduces a 60-second admission blackout
to prevent ticket replay across process restarts; do not overlap gateway instances.

Node accepts only the read/submit/steer/cancel/respond public command subset.
Reverse execution, arbitrary URLs/headers, uploads, terminals, browser work,
notifications and grant management are refused. Limits and detailed wire contract
are in [the gateway README](../web/gateway/README.md).

Conversation text is untrusted: Markdown is sanitized; raw controls, images,
frames and script execution are not rendered. Tool/attachment metadata uses text
nodes. External links require a user click and use no opener/referrer. Approval
cards show the actual action/target/reason. Question answers are model-visible;
passwords and secrets must never be entered there. Use a dedicated browser profile
on shared machines. Command identity records persist in localStorage across logout
so unconfirmed work is not forgotten; conversation and prompt text are not stored
there. Clearing browser site storage loses that local recovery record: inspect
remote receipts before repeating any uncertain work.

## Provisioning (not performed automatically)

The existing product site uses `/srv/helm/app` as the deployed **web/** directory.
Keep console access disabled until private configuration and gateway service are
ready. The checked-in default is `HELM_WEB_ENABLED=false`.

1. Install the locked PHP/frontend dependencies and Node >=22 gateway dependencies:

   ```sh
   cd web
   composer install --no-dev --optimize-autoloader
   npm ci
   npm run build
   cd gateway
   npm ci --omit=dev --ignore-scripts
   ```

2. In an operator-private terminal, generate a `password_hash()` result and a
   cryptographically random shared HMAC secret (at least 32 bytes). Do not put real
   passwords/tokens into chat, command arguments, Git, public files or screenshots.
   Configure the deployment's private Laravel `.env` (quote a password hash so `$`
   is preserved):

   ```dotenv
   APP_ENV=production
   APP_DEBUG=false
   SESSION_DRIVER=database
   SESSION_SECURE_COOKIE=true
   SESSION_SAME_SITE=strict
   HELM_WEB_ENABLED=true
   HELM_WEB_PASSWORD_HASH='OPERATOR_PASSWORD_HASH'
   HELM_WEB_GATEWAY_SECRET='SHARED_RANDOM_SECRET'
   HELM_WEB_VESSELS=primary
   ```

   Retain the existing `APP_KEY`, run migrations and rebuild configuration/view
   caches. Never use the example placeholders as real credentials. The console
   refuses disabled/incomplete configuration and non-HTTPS remote requests.

3. Provision an unprivileged `helm-gateway` system user. Store its private upstream
   JSON outside the web root, owned by this user and mode 0600, following the
   [gateway configuration](../web/gateway/README.md). Use an existing explicitly
   authorized Vessel connection grant, its exact grant UUID and Vessel UUID.
   Server-side aliases must match `HELM_WEB_VESSELS`; no tokens appear in that list.
   Configure `/etc/helm-web/gateway.env` (root:helm-gateway, 0640), for example:

   ```dotenv
   HELM_WEB_GATEWAY_SECRET=SHARED_RANDOM_SECRET
   HELM_WEB_ORIGIN=https://helm.vessel.voyage
   HELM_WEB_VESSELS_FILE=/etc/helm-web/vessels.json
   HELM_WEB_GATEWAY_HOST=127.0.0.1
   HELM_WEB_GATEWAY_PORT=8787
   ```

   The two services must receive the same literal secret. The gateway requires WSS
   upstream; literal loopback WS is allowed only with explicit test/local opt-in.
   No upstream redirects, URL credentials or TLS verification bypasses are allowed.

4. Install [the service unit](../web/deploy/helm-web-gateway.service); verify the
   Node executable and deployment paths on the actual machine. It restricts writes
   and runs without web-server privileges. Do not enable overlapping instances.
   Install the updated [Nginx configuration](../web/deploy/nginx.conf): its exact
   `/console/socket` location rewrites to the gateway's `/socket`, preserves Origin
   and Upgrade, and disables access logging for that endpoint. Run `nginx -t` and
   `systemd-analyze verify` before activation. The existing loopback origin assumes
   HTTPS termination by Cloudflare; do not expose it to the LAN unchanged.

5. Before exposing the console, verify login/CSRF, logout/lease expiry, allowed and
   refused grants, exact Vessel identity, session reads and reconnect in the real
   deployment. Provider-backed sends require operator-approved budget. Verify that
   closing the tab leaves its voyage running. Disable the console again on failure.

Rollback: set `HELM_WEB_ENABLED=false`, refresh Laravel configuration cache and
stop the gateway (immediate socket closure). Public product pages remain available.
Keep application assets matched to source. Preserve remote receipts and local
intent records; never use rollback as a reason to repeat uncertain commands.

## Verification

From the repository root, with the existing web dependencies installed:

```sh
cd web
npm ci
(cd gateway && npm ci --ignore-scripts && npm test)
npm test
npm run build
find app config routes -name '*.php' -exec php -l {} \;
php artisan route:list --path=console
php artisan view:cache
composer validate --no-check-publish
npm audit
```

Focused evidence for this delivery:

- Node gateway: 12 tests, real local WebSocket fixture, pinned hello/headers,
  exact request allowlist, lease renewal/expiry, replay refusal, wrong Origin,
  missing auth, reverse execution refusal, limits, timeout/heartbeat, private file
  restrictions and startup replay blackout.
- Web: 10 tests covering journal ordering/storage failure/recovery and wire fences;
  sanitized Markdown and DOM conversation journey through send, approval, question,
  cancel and uncertain disconnect/reconnect without replay; actual PHP HTTP
  login/session/CSRF/rate-limit/disabled-route tests. A real Laravel-issued ticket
  crosses the actual gateway into a synthetic upstream and receives a reply.
- Production asset build, PHP syntax, route/view cache, Composer validation,
  dependency audits (zero reported vulnerabilities), manifest/link checks and
  `systemd-analyze verify` passed locally. Nginx is not installed in this checkout
  environment; `nginx -t` remains a deployment gate.

The DOM journey uses jsdom, not a real browser. No real-provider send, production
console deployment, native mobile browser/IME/accessibility certification, or
macOS/Windows gateway security verification is claimed. An authenticated shared
browser was not supplied for an end-to-end real-browser journey. Rust is unchanged
by this delivery; the last Rust coverage measurement is intentionally preserved.
