# Helm Web: personal tenants and public Vessels

Issue [#290](https://github.com/o-psi/voyage/issues/290) replaces the original
single-operator design. Helm Web is a Laravel/Livewire/Flux browser client, not an
agent host and not Ratzilla.

## Ownership

An OAuth provider identity opens one user and one personal tenant. A tenant owns
up to 64 Vessel connections; new tenants start empty. There is no automatic import
of the old globally configured local Vessel. Provider identities are keyed by
provider and immutable subject, **not email**. Signing in through another provider
creates a separate personal tenant; account linking/team membership are not part
of this release. Google, X and GitHub are configurable providers. Only configured
providers are shown. OAuth tokens are used to retrieve identity then discarded.

Vessels run on their owners' hosts, own voyages and keep provider credentials.
The web host stores encrypted, explicitly supplied Vessel connection grants. It
is trusted to exercise those grants on behalf of their tenant. It does not run
Voyage agents or provision compute. Database tenant IDs are authorization scope,
not an OS execution sandbox.

## Flux presentation

All pages use the installed licensed Flux Pro v2 components: public header/navigation,
product headings/cards/actions, login, connection forms and console controls.
`resources/css/app.css` contains Tailwind/Flux imports, the documented class-based
dark variant and shared accent variables—not global element overrides or a
parallel component stylesheet. Layout spacing uses Tailwind utilities. System,
light and dark appearance follow Flux's appearance state.

The socket-driven conversation remains JavaScript-owned so Livewire does not
replace streaming content. Its interactive buttons, navigation items, question
inputs, cards and callouts are cloned from server-rendered Flux Blade templates;
JavaScript does not hand-build a second button/input system. Semantic Markdown,
code, tool JSON and disclosure content remain ordinary sanitized content, not
new UI components. The DOM journey test renders the actual Flux console view
before exercising the socket controls.

## Routes and interaction

- `/`: signed-in console; signed-out requests redirect to `/landing`.
- `/landing`: public product page with Sign in / Open console navigation.
- `/console/login`: configured OAuth provider choices; no operator password login.
- `/auth/{provider}` and `/auth/{provider}/callback`: OAuth flow.
- `/connections`: list, pair, import, replace and remove this tenant's Vessels.
- `/console/ticket`, `/console/socket`: tenant-authorized gateway transport.
- `/console/gateway/authorize`: loopback-only, authenticated internal redemption;
  never a public proxy or endpoint to expose in Cloudflare routing.

The old `/console` page redirect was removed; there is no legacy bookmark support.
Login returns to `/`, logout to `/landing`. Sessions last at most eight hours.
A real textarea, sanitized Markdown, history pages, full-message expansion,
provisional live output, approval/question cards and send/steer/cancel are
implemented. JavaScript owns the `wire:ignore` conversation region. Snapshot and
pending-decision reads are serialized at one-second intervals while visible;
catalogue refreshes every ten seconds. This is not per-token PHP rendering or
push-token subscription. The sidebar aggregates permitted voyages from all configured Vessels, with the Vessel name on each entry.

Browser intent records are tenant/connection/Vessel scoped and contain command
identities only, not prompts. They are persisted before dispatch. Reconnect reads
receipts only and never automatically resends uncertain mutations. An admission
receipt is not execution completion. Storage failure blocks sending; clearing
site storage loses local recovery evidence. Full-message expansion is capped at
4 MiB with explicit handoff to native Helm. New voyage creation and provider-account/model selection are available through scoped Vessel APIs. Uploads, private terminals, browser execution, account enrollment and the remaining native administration surfaces are still parity gaps.

## Connect your Vessels

Only publicly reachable HTTPS origins on port 443 are accepted. The authenticated
Vessel API must already be published at that origin. A Cloudflare Tunnel is one
way to expose an owner-operated Vessel; exposing the ordinary local supervisor's
private listener is **not** the public scoped API. See [Vessel connections](vessel-connections.md).

The gateway resolves all DNS answers, refuses private/reserved addresses and pins
an approved address for TLS connection with normal hostname/certificate checks.
It refuses redirects, URL credentials, alternate paths, loopback/private endpoints
and private DNS answers. This applies to both pairing and WebSocket connections,
including reconnect; a public hostname resolving to a private address is not a
workaround. The web app never makes a direct request to a tenant-provided URL.

On `/connections`, copy your tenant's pairing principal. On the Vessel host:

```sh
vessel pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal TENANT_PRINCIPAL_UUID \
  --workspace /your/workspace \
  --rights catalogue,observe,history,execute,steer,decide,cancel \
  --output /private/invitation.json
```

Paste the private invitation into the pairing form. It must target this tenant's
principal. Pending attempts retain the exact command identity and encrypted
invitation before dispatch. If delivery is uncertain, use **retry original
pairing**: Vessel's exact pairing deduplication returns the original credential,
not a replacement grant. This differs from ordinary conversation mutations,
which are reconciled through read-only receipts. A successful credential is
verified against the public pinned Vessel, encrypted at rest and never embedded
in the conversation page or gateway ticket.

Existing private credential JSON can also be imported. It must contain a public
`endpoint`, exact `vessel_id`, `grant_id` and `token`. Use a dedicated scoped grant.
Importing the same Vessel again replaces its credential inside this tenant and
increments connection revision; it cannot overwrite another tenant's connection.
Removing a connection stops new tickets and renewals. Open sockets retain a
maximum 60-second lease; immediate revocation is performed on the Vessel itself.

## Gateway and session boundary

The browser obtains an opaque single-use random ticket after CSRF and tenant
checks. Node redeems it through a fixed loopback Laravel endpoint using a shared
server credential. Laravel atomically consumes it, checks the current database
session, tenant/user ownership and connection revision, then supplies connection
credentials directly to Node—not to the browser. Each renewal requires a new
owned ticket; identity cannot switch on an existing socket. Tickets expire in
60 seconds; browser renewal is every 30 seconds. Logout invalidates sessions and
unredeemed tickets. Node applies its fixed operation allowlist and connection,
frame, pending-request, deadline and rate limits. It never handles reverse tool
execution or arbitrary upstream commands from HTTP.

Use database sessions (`SESSION_DRIVER=database`), the default JSON session serialization and
no Laravel session encryption (database protection remains a host responsibility).
`SESSION_SAME_SITE=lax` is required for OAuth's top-level callback; secure cookies
and HTTPS are required in deployment. Preserve `APP_KEY`: losing it loses encrypted
Vessel credentials. Back up SQLite consistently and keep backups private. Prevent
untrusted HTML/script injection on the same origin. OAuth callbacks/errors and
Vessel secrets must not be logged. Never enable debug output publicly.

## Deployment

Install the locked dependencies and build assets. Flux Pro requires Composer authentication for `composer.fluxui.dev`, following the [official installation instructions](https://fluxui.dev/docs/installation). Keep credentials in ignored, owner-private `web/auth.json` or deployment `COMPOSER_AUTH`; never commit license keys. The deployment must install the licensed package before serving its Blade components:

```sh
cd web
composer install --no-dev --optimize-autoloader
npm ci
npm run build
(cd gateway && npm ci --omit=dev --ignore-scripts)
php artisan migrate --force
php artisan optimize
```

Configure Laravel privately:

```dotenv
APP_ENV=production
APP_DEBUG=false
APP_URL=https://helm.vessel.voyage
SESSION_DRIVER=database
SESSION_SECURE_COOKIE=true
SESSION_SAME_SITE=lax
HELM_WEB_ENABLED=true
HELM_WEB_GATEWAY_SECRET=SHARED_RANDOM_SECRET_AT_LEAST_32_BYTES
HELM_WEB_GATEWAY_ADMIN_URL=http://127.0.0.1:8787
GOOGLE_CLIENT_ID=
GOOGLE_CLIENT_SECRET=
X_CLIENT_ID=
X_CLIENT_SECRET=
GITHUB_CLIENT_ID=
GITHUB_CLIENT_SECRET=
```

Register each OAuth application with the exact callback URL, for example
`https://helm.vessel.voyage/auth/google/callback` (substitute `x` or
`github`). Use web/confidential app credentials and provider console consent/test
user settings as appropriate. Do not paste secrets into chat. Only fully
configured providers are enabled; missing credentials show a setup-pending page,
not a bypass login. Changes require Laravel config cache refresh.

Node gateway environment:

```dotenv
HELM_WEB_GATEWAY_SECRET=THE_SAME_SHARED_RANDOM_SECRET
HELM_WEB_AUTH_URL=http://127.0.0.1/console/gateway/authorize
HELM_WEB_ORIGIN=https://helm.vessel.voyage
HELM_WEB_GATEWAY_HOST=127.0.0.1
HELM_WEB_GATEWAY_PORT=8787
```

No global `HELM_WEB_VESSELS_FILE`, loopback-upstream option or operator password
is used. Old single-operator sessions do not authenticate a tenant. Do not expose
the Node listener; Nginx proxies only the exact `/console/socket` endpoint to
`/socket`. Deny public access to `/console/gateway/authorize` and proxy its
loopback calls directly to Laravel. Existing [Nginx](../web/deploy/nginx.conf)
and [gateway service](../web/deploy/helm-web-gateway.service) templates need paths,
user and node executable matched to the host. Run `nginx -t` and
`systemd-analyze verify` before activation. The product origin is loopback behind
Cloudflare HTTPS; it is not a LAN HTTP login server.

The current local deployment lives at `/srv/helm-web/app` and uses separate
`helm-web-nginx`, `helm-web-gateway`, `php-fpm` and `helm-web-tunnel` services.
The old local Vessel adapter is not a connection for new tenants. Roll back source,
assets and database together from the pre-migration backup; do not re-enable the
old shared operator model for registered users. Setting `HELM_WEB_ENABLED=false`
and stopping the gateway disables console access without cancelling voyages.

## Verification and limits

Focused checks use real PHP HTTP sessions and separate tenant identities, plus
Node gateway tests with injected fixture transports. They cover tenant isolation,
foreign connection refusal, one-use tickets, connection deletion/session expiry,
OAuth state/provider identity, public endpoint validation, lease renewal and
uncertain-command recovery. Commands:

```sh
cd web
npm test
php tests/oauth.php
(cd gateway && npm test)
npm run build
find app config routes database/migrations -name '*.php' -exec php -l {} \;
php artisan route:list --except-vendor
php artisan view:cache
composer validate --no-check-publish
```

Delivery checks passed: 10 web tests, 80 offline OAuth assertions, 19 gateway
tests, production build, PHP lint/view/route checks, Composer validation,
Composer/npm audits (zero advisories), and documentation/diff checks. PHP iconv
must be enabled for the locked Composer dependencies.

Mock OAuth and DNS/socket fixtures do not establish live Google/X consent,
third-party Vessel compatibility, real-browser IME/accessibility or native
macOS/Windows gateway security. Live OAuth requires provisioned app credentials;
provider-backed voyage actions require a separate approved execution budget.
No Rust source changes or Rust coverage refresh belong to this web delivery.

## Native Flux console presentation

Console, sign-in and connection management use the installed licensed Flux Pro v2 Blade
components. Follow the official [sidebar](https://fluxui.dev/layouts/sidebar),
[modal](https://fluxui.dev/components/modal),
[callout](https://fluxui.dev/components/callout),
[text](https://fluxui.dev/components/text) and
[table](https://fluxui.dev/components/table) composition APIs. The responsive sidebar
uses Flux's mobile collapse/toggle and native item truncation; no wrapped labels
inside fixed-height navigation rows. Unchanged catalogue polls retain the controls.

Socket-driven controls clone server-rendered Flux templates. Do not create raw
interactive elements, copy vendor component markup/styles, or add another widget
library. Use documented component slots for callout headings, text and actions.
The message input uses the licensed native `flux:composer`, including its action slots, automatic height and Ctrl/Cmd+Enter submission. Account and model choices use native searchable Pro listboxes. Tool/attachment inspection uses a
native Flux modal with its built-in dismissal and focus behavior.

Semantic forms, layout containers and sanitized Markdown/code/list content remain
HTML. Markdown is sanitized before being composed into rendered Flux heading,
text, link, table, separator, card and callout templates. External content cannot
supply Flux/Alpine attributes or controls. Shared CSS remains Flux/Tailwind imports
and theme tokens; component appearance belongs to Flux, with utilities limited to
layout and content formatting. Empty output and obsolete selection notices are
hidden without changing receipt handling, canonical history or execution policy.

## Aggregated Vessel navigation

The console opens independent authenticated sockets to every configured Vessel,
merges their catalogues and identifies each voyage with a native Flux Vessel badge.
Search matches voyage titles, session IDs and Vessel names. Selection and in-memory
composer drafts use connection ID plus session ID, so identical voyage IDs on two
Vessels cannot share drafts or route commands through one another. The conversation
header also names the selected Vessel. Reloading clears unsent in-memory drafts.

Each connection owns its lease renewal, reconnect backoff, catalogue and command
journal. An unavailable Vessel retains its last known catalogue with offline status;
other Vessels remain usable. Read and mutation commands use only the selected
connection. Reconnection checks retained receipts and never replays an uncertain
mutation. Removed/unauthorized connections stop retrying automatically; refreshing
connections explicitly retries them. Reload after adding/removing connections in
another tab to update the configured set.

The ticket endpoint allows 256 requests per minute per user and 30 per connection,
covering initial connection and 30-second renewals for up to 64 Vessels. The gateway
retains its 64-socket global cap, permits up to 64 sockets per authenticated session,
and caps each session/tenant/connection at four sockets. Global capacity is shared
across users and tabs; a configured connection is not a reservation of capacity.
Credentials, grant identity validation and revocation are unchanged.

Browser verification used two synthetic Vessels with identical voyage names and
session IDs: combined labels, Vessel-name search, empty search, switching/draft
isolation, selected-Vessel submission, one-Vessel outage and recovery. The deployed
browser has one configured real Vessel; no second production connection or live
provider run was created for verification.

## New voyages and provider accounts

New voyage opens a native Flux modal for Vessel, authorized workspace, provider account, model, reasoning and service tier. Account metadata and models are fetched from that exact Vessel. Unavailable accounts remain labeled and cannot be selected. Explicit model/account changes reset reasoning and service options to provider defaults. Creating starts an empty independent voyage process; no inference is submitted until the user sends a message.

Creation needs `create` and `account_use` rights and explicit account UUIDs in the workspace connection grant. Pairing help documents `--accounts` alongside the rights. Existing idle voyages expose Account & model in the Pro Composer; changes require `account_use` and bind the reviewed session, incarnation and revision. Host-owner configuration and credentials are never tunneled through the browser.

Creation stores only its immutable request metadata under tenant/connection/Vessel identity before dispatch. An unconfirmed reply leaves a Check creation action which sends `resolve_start_account` for that exact request; it never repeats start or submits a prompt. Confirmed created/not-admitted outcomes settle the record. A pending creation blocks another creation on that connection. Connection replacement does not erase recovery records.

Full native-client parity remains open: account enrollment/usage, attachments, private terminals, browser sharing, lifecycle and other native administration are not implemented by these controls.
