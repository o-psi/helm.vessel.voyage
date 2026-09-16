# Helm Web: personal tenants and public Vessels

Issue [#290](https://github.com/o-psi/voyage/issues/290) introduced personal tenants;
[#307](https://github.com/o-psi/voyage/issues/307) connects browsers directly to public Vessels. Helm Web is a Laravel/Livewire/Flux browser client, not an
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
The web host stores encrypted, explicitly supplied Vessel connection credentials.
The owner’s full-access pairing trusts it to operate that Vessel on behalf of its tenant. It does not run
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
- `/console/ticket`: tenant-authorized temporary Vessel credential bootstrap.
- `/console/socket` and `/console/gateway/authorize`: retired legacy endpoints;
  never a public proxy or endpoint to expose in Cloudflare routing.

The old `/console` page redirect was removed; there is no legacy bookmark support.
Login returns to `/`, logout to `/landing`. Sessions last at most eight hours.
A real textarea, sanitized Markdown, history pages, full-message expansion,
provisional live output, approval/question cards and send/steer/cancel are
implemented. JavaScript owns the `wire:ignore` conversation region. Snapshot and
pending-decision reads are driven by the native Vessel observation subscription,
with a 30-second fallback refresh; catalogue refreshes every ten seconds. Live
assistant output, generating tool arguments and provider-disclosed reasoning
update from the same bounded projections used by the TUI. Native events are
invalidations, not append-only token payloads: bursts coalesce into fresh reads.
Gaps, owner changes and reconnect seed a new snapshot/subscription. Canonical
messages replace provisional tool previews without executing preview content.
The conversation follows new messages, live output and delayed content resizing while
the reader remains at the bottom. Scrolling upward pauses following; returning to
the bottom or choosing **Jump to latest** resumes it. Loading earlier history
preserves the reading position, and selecting a voyage starts at its latest output.
This is not per-token PHP rendering. The sidebar aggregates permitted voyages from all configured Vessels, with the Vessel name on each entry.

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
(5-second DNS deadline; HTTPS has a separate 5-second deadline/3-second connect timeout). Flux Pro requires Composer authentication for `composer.fluxui.dev`, following the [official installation instructions](https://fluxui.dev/docs/installation). Keep credentials in ignored, owner-private `web/auth.json` or deployment `COMPOSER_AUTH`; never commit license keys. The deployment must install the licensed package before serving its Blade components:

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

Focused checks cover separate HTTP tenant sessions, direct PHP bootstrap and
HTTPS options, native browser socket authentication, direct fleet renewal and
uncertain-command recovery. Fixtures are not live OAuth, TLS browser certification
or paid-provider evidence. Legacy gateway fixtures remain for comparison. Commands:

```sh
cd web
npm test
php tests/oauth.php
php tests/direct-bootstrap.php
php tests/direct-http.php
(cd gateway && npm test)
npm run build
find app config routes database/migrations -name '*.php' -exec php -l {} \;
php artisan route:list --except-vendor
# Use an isolated VIEW_COMPILED_PATH for CLI/rendering checks; see deploy/compiled-views.md.
composer validate --no-check-publish
```

Current delivery evidence and remaining deployed-browser limitations are recorded
in [the #307 verification report](direct-wss307-verification.md). Historical #290 counts do not establish the new direct transport.
See [transport measurement](direct-wss307-benchmark.md) for the synthetic
before/after workload, measured resource use, traffic and explicit limitations.


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
Search matches voyage titles, session IDs and Vessel names. Client selection and
local recovery are connection-scoped; durable draft identities belong to the
selected Vessel. Identical voyage IDs on two Vessels cannot share drafts or route
commands through one another. The conversation
header also names the selected Vessel. Unsent composition is autosaved on the selected Vessel; see
[shared drafts](shared-drafts.md) for cross-device discovery, conflict handling
and picture staging. Local pending-command records remain admission recovery,
not a second authoritative draft store.

Each connection owns its lease renewal, reconnect backoff, catalogue and command
journal. An unavailable Vessel retains its last known catalogue with offline status;
other Vessels remain usable. Read and mutation commands use only the selected
connection. Reconnection checks retained receipts and never replays an uncertain
mutation. Removed/unauthorized connections stop retrying automatically; refreshing
connections explicitly retries them. Reload after adding/removing connections in
another tab to update the configured set.

The ticket endpoint allows 256 requests per minute per user and 30 per connection,
covering initial connections and approximately 90-second renewals for up to 64 Vessels.
Each Vessel bounds its own sockets and temporary credentials. Renewal briefly needs
an extra socket; configured connections are not capacity reservations.

Browser verification used two synthetic Vessels with identical voyage names and
session IDs: combined labels, Vessel-name search, empty search, switching/draft
isolation, selected-Vessel submission, one-Vessel outage and recovery. The deployed
browser has one configured real Vessel; no second production connection or live
provider run was created for verification.

## New voyages and provider accounts

New voyage opens a native Flux modal for Vessel, workspace folder, provider account, model, reasoning and service tier. Account metadata and models are fetched from that exact Vessel. Unavailable accounts remain labeled and cannot be selected. Explicit model/account changes reset reasoning and service options to provider defaults. Creating starts an empty independent voyage process; no inference is submitted until the user sends a message.

Owner pairing uses `--full-access`: no rights, workspace or account allowlists are required. It covers all ordinary voyages, canonical workspace folders and provider accounts, including resources added after pairing. The workspace picker offers known folders and **Another folder…** for an existing absolute path on the Vessel. Authentication, expiry, revocation and each voyage’s local execution policy still apply. Provider credentials remain on the Vessel. Full access does not add web UI features that are listed above as parity gaps.

Older scoped connections are not upgraded automatically. Pair a new full-access invitation to replace one. Scoped invitations remain supported for other clients. Existing idle voyages expose account/model controls, binding the reviewed session, incarnation and revision.

Creation stores only its immutable request metadata under tenant/connection/Vessel identity before dispatch. An unconfirmed reply leaves a Check creation action which sends `resolve_start_account` for that exact request; it never repeats start or submits a prompt. Confirmed created/not-admitted outcomes settle the record. A pending creation blocks another creation on that connection. Connection replacement does not erase recovery records.

Full native-client parity remains open: account enrollment/usage, private terminals, browser sharing, lifecycle and other native administration are not implemented by these controls. Picture attachments use the shared draft staging and existing multimodal submission paths; they do not add terminal or browser execution authority.

## Composer keyboard and access controls

The Flux composer sends (or steers an active run) on Enter; Shift+Enter inserts a newline. IME composition Enter does not send. The access selector shows the selected voyage’s observed read-only, approval or unrestricted mode. Changing it sends `set_access` through the same revision-bound command journal as other actions; only a refreshed owner snapshot confirms the mode. Stale, disconnected or unconfirmed-command state disables changes. Executing-host policy and connection grants remain authoritative; unrestricted is not an OS sandbox bypass.

The composer uses a compact model/reasoning settings trigger, access selector, and icon-only send/cancel controls with accessible labels. Full access maps to `unrestricted`; host limits still apply. Cancel is hidden without an active run. Attachments and @/slash suggestions are not advertised because these web workflows are not implemented.

When the checkout is also the served application, CLI-generated Blade views must remain readable by the PHP-FPM service user, which also needs write access to runtime view/cache/session/log directories. Use narrowly scoped service-user ACLs with directory inheritance, not world-writable permissions; preserve application keys and credentials. A passing CLI render does not prove the service user can render the authenticated console.

Existing-voyage settings open in click-triggered Flux popovers above the composer: account/model with provider discovery, a separate reasoning selector, and access with mode explanations. New voyage retains its modal. Popovers are viewport-width constrained; model options scroll. Inference changes bind the reviewed voyage identity and revision and are refused after switching or starting a run. Popover field Enter does not submit the message draft.

Reasoning uses a stepped Flux slider: the first stop is provider default, followed by the provider-advertised levels in their returned order. The label shows the canonical level, not an invented numeric effort. Default-only models disable the slider; existing explicit values remain visible. Service tiers use Flux radio choices. These controls are shared by creation and account/model editing; account and model lists remain searchable.

The model popover now contains only model selection. Account has its own picker and private usage observation: cached status, observation time, used percentages and reset times when exposed by the provider. Refresh is explicit and uses the existing scoped `account_usage` read, never inference; unavailable/unsupported data is not zero usage. Switching accounts selects the account’s default model and resets provider options. Service radios have their own popover. Usage replies are checked against the selected account and connection before display.

## Conversation presentation

Conversation chrome is moved to the sidebar; mobile keeps a navigation toggle. User messages are compact right-aligned surfaces; assistant responses are open, readable text rather than repeated bordered cards. Consecutive tool request/result entries collapse into one activity disclosure, with bodies rendered only when opened. Complete-message expansion and structured details remain available inside it; expanded groups survive history refreshes. Tool content is never discarded or interpreted as trusted HTML. Streaming output remains explicitly provisional, and approvals/errors stay outside collapsed history.

Composer triggers display observed settings: model, account label, service tier, reasoning and access. Account labels load from the selected Vessel’s scoped catalogue, checked against the full account binding and voyage incarnation; late replies cannot replace another voyage’s label. Missing metadata is shown as unavailable, never guessed from another account. Null service tier is displayed as Default tier.

Conversation refinement: assistant replies now use left-aligned bubbles without an attribution row. Message times use durable `created_at`, with the full local date/time on hover and an explicit unknown state for legacy entries. Tool activity lists every request/result entry by name; each opens independently, or Expand all reveals them together. Results correlate names via tool-call IDs, and completion/failure is shown only when the saved success flag supplies it. Entry expansion is retained across history refreshes.

Tool activity follows the TUI’s compact action pattern: pair calls/results by ID, retain the newest three actions, and reveal the older prefix separately from per-action details. Summaries use saved outcome/duration facts and descriptive file/shell actions; missing timestamps are omitted. Web bubbles are presentation-only. Specialist TUI descriptions and Vessel-specific outcome interpretation are not yet fully ported.

## Connection deadlines

Direct browser requests have a 30-second reply deadline. Timeout or disconnect
leaves mutations in the receipt journal; neither renewal nor reconnect resends
them. Connection diagnostics (`[Helm connection]` in the browser console) contain
fixed metadata only, never token, prompt or result bodies. The legacy Node gateway
is no longer a source of diagnostics for the current conversation path.
