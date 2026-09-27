# Helm Web: React console, personal tenants and public Vessels

React 19 / TypeScript is the production console at `/`. Laravel supplies sessions,
OAuth, tenant-isolated connection storage and short-lived credential bootstrap.
Helm remains a viewer/controller; Vessel-supervised Voyage processes own execution.

The old Livewire/Flux console is archived in
[`archive/helm-web-flux`](../archive/helm-web-flux/README.md). It has no application
route or Vite entry. Shared public and sign-in pages retain Flux; that does not
make them a second console. Implement all new console behavior in
`web/resources/react`, with shared protocol/browser helpers in `web/resources/js`.

## Routes and interaction

- `/`: authenticated React console; signed-out requests redirect to `/landing`.
- `/react`: compatibility redirect to `/`, preserving `manage-vessels=1`.
- `/landing`, `/helm`, `/vessel`, `/voyage`: public product pages.
- `/console/login`, `/auth/{provider}`, `/auth/{provider}/callback`: existing login.
- `/connections`: opens the React connection manager; mutation routes retain tenant
  checks and CSRF. Successful changes and errors return to `/?manage-vessels=1`.
- `/console/ticket`: temporary tenant-authorized Vessel credential bootstrap.

React retains each open voyage's in-memory draft, renders canonical history and
live observations, and shows Starting, Working and Waiting for you in the
conversation pane. Successful admission is not execution completion. Unknown
commands retain their exact receipt identities and are never automatically replayed.

New voyage settings choose Vessel, workspace and profile; **Create voyage** creates
an independent voyage without inference. Sending a message starts a run. Profile
management, account enrollment, reasoning/service choices, attachments, typed
approval/question dialogs, advanced voyage actions and the shared browser remain
available. Reloading loses unsent text/pictures, not server-side conversations.

Profile setup uses a compact overview with separate searchable profile, account and
model screens. Each profile row has a three-dot menu for edit, duplicate, default
and delete; the fixed header keeps Create profile (+) available even in empty or
filtered lists. Successful lists have no manual reload control; a failed load or
uncertain change offers a read-only Check status recovery action. Header/back navigation and footer actions stay visible while the
current screen scrolls. Unsaved profile edits survive picker navigation and Vessel connection renewal.
A changed catalogue revision requires review before saving; reasoning
and service tier have their own step. Profile deletion requires confirmation.
Expired ChatGPT accounts offer an explicit exact-binding sign-in refresh followed
by catalogue reload; an uncertain refresh is not replayed, and a mismatched reply
requires explicit reload before use. Account usage is loaded only on request.

**Vessel updates** under Location uses the existing reviewed updater and saved operation
journal. It checks capabilities before profile discovery, reviews an exact release,
applies only on approval and verifies the running release after reconnect. A Vessel
predating the updater still needs one administrator bootstrap. See
[remote updates](remote-updates.md).

## Ownership

An OAuth provider identity opens one user and one personal tenant. A tenant owns
up to 64 Vessel connections; new tenants start empty. There is no automatic import
of the old globally configured local Vessel. Provider identities are keyed by
provider and immutable subject, **not email**. Signing in through another provider
creates a separate personal tenant; account linking/team membership are not part
of this release. Google, X and GitHub are configurable providers. Only configured
providers are shown. OAuth tokens are used to retrieve identity then discarded.

Vessels run on their owners' hosts, own voyages and keep provider credentials.
The web host stores encrypted, explicitly supplied Vessel connection credentials.
The owner’s full-access pairing trusts it to operate that Vessel on behalf of its tenant. It does not run
Voyage agents or provision compute. Database tenant IDs are authorization scope,
not an OS execution sandbox.

## Connect your Vessels

Only publicly reachable HTTPS origins on port 443 are accepted. The authenticated
Vessel API must already be published at that origin. A Cloudflare Tunnel is one
way to expose an owner-operated Vessel; exposing the ordinary local supervisor's
private listener is **not** the public scoped API. See [Vessel connections](vessel-connections.md).

Laravel resolves all DNS answers for server-side pairing, verification and temporary
credential bootstrap. It refuses private/reserved addresses, pins an approved
address for TLS with normal hostname/certificate checks, and refuses redirects,
URL credentials and alternate paths. The browser then opens WSS directly to that
public Vessel. Browser DNS/TLS/network policy is independent: server-side pinning
does not pin the browser's DNS resolution. There is no private-network relay or
fallback. Use a publicly resolvable hostname and a browser-trusted TLS certificate.
No cross-origin HTTP bootstrap is needed: browser bootstrap is same-origin to
Laravel; only the authenticated WebSocket crosses origins.

On `/connections`, copy your tenant's pairing principal. On the Vessel host:

```sh
vessel pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal TENANT_PRINCIPAL_UUID \
  --full-access \
  --output /private/invitation.json
```

Paste the private invitation into the pairing form. It must target this tenant's
principal. Pending attempts retain the exact command identity and encrypted
invitation before dispatch. If delivery is uncertain, use **retry original
pairing**: Vessel's exact pairing deduplication returns the original credential,
not a replacement connection. This differs from ordinary conversation mutations,
which are reconciled through read-only receipts. A successful credential is
verified against the public pinned Vessel, encrypted at rest and never embedded
in the conversation page or browser bootstrap response.

Existing private credential JSON can also be imported. It must contain a public
`endpoint`, exact `vessel_id`, `grant_id` and `token`. Use a dedicated connection credential.
Importing the same Vessel again replaces its credential inside this tenant and
increments connection revision; it cannot overwrite another tenant's connection.
Removing a connection stops new tickets and renewals. Open sockets retain a
maximum 120-second credential lifetime; revoke the underlying grant on the Vessel
only when all clients using that grant should lose access.

## Direct transport and session boundary

The browser obtains a temporary Vessel-issued credential through the authenticated,
CSRF-protected `/console/ticket` endpoint. Laravel checks tenant ownership and
calls the selected Vessel's `/v1/vessel/browser-credentials` using its encrypted
stored pairing credential. The browser receives only a random temporary token,
expiry, expected Vessel identity and public socket URL. It opens
`wss://VESSEL/v1/vessel/browser-socket` with subprotocol `voyage.vessel.v1`, sends
`{type:"authenticate",token:…}` as the first frame, then waits for the identity-bound
`hello`. Credentials never appear in URLs or WebSocket subprotocols. Do not log
bootstrap bodies or authentication frames.

Conversation commands, snapshots, history, live output and subscriptions travel
between browser and Vessel, not through PHP or Node. Vessel applies the existing
connection grant scope plus the web-operation subset; owner pairing remains
explicit full access, scoped pairing remains scoped. Shared-local-browser execution
and private terminals are not enabled by this transport. Provider credentials
stay on the executing host.

Temporary credentials last at most 120 seconds. The browser opens a replacement
30 seconds before expiry, switches new requests to it and lets old requests drain
for at most 30 seconds. Renewal and reconnection never replay effects. Each new
socket uses the same existing receipt journal. Failed bootstrap retries with bounded
backoff; sign-out, removed connections and permission refusal stop retries.
Credentials are memory-only in Vessel and browser: Vessel restart invalidates them.

Logout/removal prevents future bootstrap; it does **not** claim immediate remote
revocation of an already-open socket. An already-issued credential reaches its
hard deadline within 120 seconds. The Vessel checks current grant authority and
expiry on operations and observation delivery; a hard deadline closes the socket,
with at most 3 additional seconds for bounded in-progress authorization/writes. Revoking the underlying grant on Vessel affects
all clients using that grant; removing a web connection deliberately does not
revoke unrelated clients. In-flight admitted commands can finish and voyages are
not cancelled by closing the browser or signing out.

Use database sessions (`SESSION_DRIVER=database`), the default JSON session serialization and
no Laravel session encryption (database protection remains a host responsibility).
`SESSION_SAME_SITE=lax` is required for OAuth's top-level callback; secure cookies
and HTTPS are required in deployment. Preserve `APP_KEY`: losing it loses encrypted
Vessel credentials. Back up SQLite consistently and keep backups private. Prevent
untrusted HTML/script injection on the same origin. OAuth callbacks/errors and
Vessel secrets must not be logged. Never enable debug output publicly.

## Deployment

Install the locked dependencies and build assets. PHP needs cURL, OpenSSL, DNS support,
a matching CLI executable and permission to launch the bounded DNS-only PHP child
(5-second DNS deadline; HTTPS has a separate 5-second deadline/3-second connect timeout). Flux Pro requires Composer authentication for `composer.fluxui.dev`, following the [official installation instructions](https://fluxui.dev/docs/installation). Keep credentials in ignored, owner-private `web/auth.json` or deployment `COMPOSER_AUTH`; never commit license keys. The shared login and public pages still require the licensed package; the React console does not load Flux. Install it before serving those Blade components:

```sh
cd web
composer install --no-dev --optimize-autoloader
npm ci
npm run build
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

### Upgrade and retire the conversation gateway

Upgrade each Vessel first so its public API exposes the two browser endpoints.
Then deploy matching Laravel code and built assets. Keep `APP_KEY`, tenant rows,
pairings and encrypted connection credentials: existing users do not need to pair
again. Set `APP_URL` to the exact public HTTPS Helm Web origin. That origin is
bound into temporary credentials; a different site cannot use them.

Deploy the [Nginx template](../web/deploy/nginx.conf), which no longer proxies
`/console/socket`. Stop/disable `helm-web-gateway` only after checking other users
of that deployment and confirming upgraded clients work directly. The retained
`web/gateway` code/service template is legacy migration/test material, not required
by the new console. Retire its shared secret only when no legacy consumer needs it.
Do not delete unrelated Vessel grants or provider credentials. Old pages should
be reloaded after upgrade; no silent gateway fallback is provided.

Run `nginx -t` before activation. Back up source/assets/database consistently and
preserve the previous gateway configuration for deliberate rollback. Rollback
requires matching old browser assets and gateway, not just PHP. Setting
`HELM_WEB_ENABLED=false` disables new console access; temporary credentials already
issued expire within their bounded lifetime, without cancelling voyages.

## Verification and limits

From `web/`: `npm run typecheck`, `npm test`, `npm run build`, then
`node tests/browser-layout-browser.mjs`. The active suite includes shared transport,
tenant/CSRF/authentication checks and React lifecycle/receipt/update tests. Archived
Flux presentation fixtures are retained as history outside active test discovery.
A successful build alone does not establish live deployment or provider inference.
Verify the authenticated production root, the `/react` redirect, existing voyage
history, connection manager, settings and updater in the shared browser after
publication. Native/provider checks retain their separate host/budget requirements.

Deployment, backups and rollback are described in [the Web README](../web/README.md).
The former Flux presentation details are historical in the archive; they are not
requirements for the production React console.
