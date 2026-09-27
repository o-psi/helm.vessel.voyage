> The console now connects browsers directly to public Vessel WSS endpoints (#307).
> Laravel handles login, tenant connections and temporary credential bootstrap.
> Node is needed to build assets, not to relay conversations. See
> [Helm Web deployment and gateway migration](../docs/helm-web.md#upgrade-and-retire-the-conversation-gateway).

# Helm website

## Production console

React 19 / TypeScript is the authenticated console at `/`. `/react` redirects to
`/` and retains the connection-management query. Laravel owns authentication,
tenant-scoped connection bootstrap and the shared public/login pages. The console
loads no Livewire or Flux runtime. The retired console is preserved in
[`archive/helm-web-flux`](../archive/helm-web-flux/README.md), outside route,
Composer, Vite and default test discovery. Do not implement console changes there.

From `web/`, use `npm ci`, `npm run typecheck`, `npm test`, and `npm run build`.
The supported Node range is in `package.json`. Production assets contain the
React console plus shared public/login assets; a build alone does not deploy them.

React owns voyage navigation, drafts, history, run status, settings, decisions,
attachments and connection management. Shared transport/intent journals retain
exact command identities across the cutover. Browser viewing, account enrollment,
advanced voyage actions and the reviewed updater use explicitly disposed adapters.
No unknown command is automatically replayed. Provider credentials remain on Vessels.

Create voyage explicitly prepares an independent voyage before its first message.
Unsent text and pictures live in page memory and survive switching conversations,
but not reloading. Profile setup carries over the archived console’s compact
overview, separate searchable pickers, back navigation, fixed actions, deletion
confirmation and expired-sign-in refresh. React owns the screens and draft state;
no Flux console code is loaded. Reasoning/service and account usage have dedicated
steps. Vessel updates under Location exposes exact prepare/review/apply/status and capability-gates older
hosts. Existing private keys, tenants, connections and conversation journals need
no migration for this interface cutover.

helm.vessel.voyage. Foleybridge.Software presents Helm, Vessel, and Voyage
as a product suite for running coding agents across machines the user controls.
Product copy presents Helm Web and the Linux terminal client as two interfaces
to operator-controlled Vessels and independent Voyages. Web use requires Vessel
and Voyage installed on a machine with an authenticated public HTTPS/WSS endpoint;
it does not include hosted agent compute. The
public landing page announces v1.0.0 with pinned Linux x86-64 archive/checksum
links, glibc 2.39+ requirements and review-first installation instructions. It links to focused `/helm`, `/vessel`, and
`/voyage` product pages. The public pages do not execute agents, enroll users or collect email addresses.
An opt-in authenticated **Helm Web console** at `/` gives each OAuth identity a
personal tenant with its own publicly reachable Vessel connections over a direct
authenticated WebSocket after same-origin credential bootstrap. See
[console setup and limits](../docs/helm-web.md).
Signed-out visitors go to `/landing`. Google, X and GitHub sign-in are shown only
when their application credentials are configured.
The console never executes agents on the web host.

## Development

Requires PHP 8.3+ with Laravel extensions, Composer, and a Node version allowed
by `package.json`'s `engines`. Flux Pro also requires private Composer
authentication for composer.fluxui.dev; keep auth.json out of Git (it is ignored).

```sh
cd web
composer install
cp .env.example .env
php artisan key:generate
php artisan migrate
npm ci
npm run build
php artisan serve
```

Keep `.env`, SQLite databases, logs, `vendor`, `node_modules` and compiled assets
out of Git. The lockfiles preserve deployed dependency versions. Source uses the
Laravel application skeleton, with its original framework configuration and
migrations. Console authentication routes fail closed (404) until explicitly
configured. OAuth sign-in creates a personal tenant with no inherited connections.

## Deployment

The public Helm Web origin runs in Proxmox CT 106, the dedicated unprivileged
Debian 13 CT named `helm-web`. The laptop remains a Vessel host and serves its
public Vessel API; browser console traffic connects directly to that API after
Helm Web login. The CT has 2 cores, 2 GiB RAM, 512 MiB swap and a 16 GiB root
disk. DHCP attaches to the existing bridge. Container nesting is enabled for
Debian systemd mount compatibility; no host devices are passed through. It
starts at host boot.

Application: `/srv/helm/app`, owned by the unprivileged `helm` deployment account.
Private runtime state is under `/srv/helm/runtime`, outside the application source;
preserve it and the production `.env` across source updates. The initial CT source
was copied from the laptop's live working tree, including uncommitted changes.
Future changes to `web/` require an explicit deployment to CT 106 and a private
backup before replacing that source; a local edit or Git push does not update the
running website. Deploy matching PHP source and built assets together.

Nginx serves only `public/`, on `127.0.0.1:80`. PHP-FPM runs as `www-data`.
The deployment account's home must permit traversal (`chmod 755 /srv/helm`).
Only `storage`, `bootstrap/cache` and the SQLite database directory need group
write access for `www-data`; use setgid directories to preserve that group.
The production `.env` is `helm:www-data` mode 0640. Set:

```dotenv
APP_NAME=Helm
APP_ENV=production
APP_DEBUG=false
APP_URL=https://helm.vessel.voyage
SESSION_SECURE_COOKIE=true
LOG_LEVEL=warning
```

Retain the migrated `APP_KEY` exactly across CT upgrades; do not regenerate it.
Generate a key only for a fresh installation with no existing runtime state. Run
`php artisan migrate --force` on the CT as `helm`. Keep the configuration cache
cleared: the private `LARAVEL_STORAGE_PATH` comes from `.env`, but
`php artisan optimize` currently caches paths under `/srv/helm/app/storage`. If
`optimize` was run, follow it with `php artisan config:clear` before serving
traffic. Let `www-data` compile views in
`/srv/helm/runtime/storage/framework/views`; a
view cache generated as `helm` can fail when Laravel later updates a compiled
file's timestamp as `www-data`. Verify both `/up` and an authenticated console
request after changing caches. See [compiled-view deployment notes](deploy/compiled-views.md).

The CT currently has Node 20.19.2, below `package.json`'s supported range. The
initial deployment used matching assets built on the laptop. For later updates,
run `npm ci` and `npm run build` on a host with supported Node, then copy the
matching built assets to the CT; alternatively, upgrade the CT's Node before
building there. Run Composer as `helm` and reapply shared directory permissions
after caching.

`deploy/nginx.conf` is installed in `/etc/nginx/sites-available/helm` and linked
under `sites-enabled`. It assumes all requests arrive through the local HTTPS
Cloudflare connector and sets FastCGI HTTPS accordingly. Do not expose this origin
to the LAN without revisiting the HTTPS assumption. Run `nginx -t` before restarting
Nginx; changing from a wildcard listener to loopback requires a restart.

`deploy/cloudflared.service` runs the connector as its own system user, reading
`/etc/cloudflared/tunnel-token` (root:cloudflared, 0640). Never place the credential
in source, command arguments or issue comments. The remotely managed tunnel maps
`helm.vessel.voyage` to `http://localhost:80`, with a final 404 catch-all. Nginx,
PHP 8.4 FPM and cloudflared are enabled at boot. The connector binary is installed
at `/usr/local/bin/cloudflared`; explicit operator upgrades are required.

## Operations and verification

Use the Proxmox console or `pct exec` for administration. Check `systemctl is-active
nginx php8.4-fpm cloudflared`, `systemctl --failed`, and `nginx -t`. The internal
`http://127.0.0.1/up` endpoint should return 200. The public page must contain the
Helm title, and all production React assets must return 200. The console must not
load the archived console bundle or Flux/Livewire scripts. Shared public/login
pages still load their own Flux assets. Verify desktop and mobile layouts.
`/.env` must return 403; unknown pages must return 404. Debug output stays disabled.

If public resolution lags, inspect authoritative/public DNS independently; do not
mistake a resolver cache for a failed tunnel. Tunnel registration alone does not
establish working application routing. The connector's disabled ICMP proxy is
unneeded for this HTTP-only origin.

Before upgrades, back up the application `.env`, private runtime state and database
(using an SQLite-aware backup), and tunnel token to operator-controlled private
storage, or take a stopped container backup. Keep the deployed source and its
matching assets available for rollback. No off-host backup schedule is provisioned
by this change.

Rollback application source and its matching assets together; database rollback
requires the matching backup. Do not regenerate keys during recovery.

Initial checks: production build and Laravel optimization, migrations, Composer
validation/audit, origin HTTP/health, public HTTPS/assets, route-specific titles,
descriptions and links, environment-file denial and service health. These are
HTTP/runtime checks; no browser visual QA or agent execution was performed.

### Console verification

Run `npm test` for the active shared transport/auth and React suites. The HTTP
fixture checks canonical root rendering, `/react` redirect, tenant isolation,
CSRF, connection-management feedback and logout. Archived Flux presentation
fixtures are historical and are excluded from the active suite. Use
`node tests/browser-layout-browser.mjs` after building for desktop/mobile Chromium
checks, then verify the authenticated production root in the shared browser.

## Laravel Boost (local AI development)

[Laravel Boost](https://laravel.com/docs/boost) is a development-only Composer
dependency (`laravel/boost`); install with `composer install` from `web/`.
The committed `boost.json`, `AGENTS.md`, and `.agents/skills/` contain the generated
Laravel/Livewire/Flux guidance. They do not replace the root repository instructions.
The Codex generator is used for its portable AGENTS.md and skill format; it does
not imply that Helm uses Codex or that a client connection has been activated.

To refresh the generated guidance and skills from `web/`:

```sh
APP_ENV=local LOG_CHANNEL=stderr php artisan boost:update --no-interaction
```

For Helm, merge [helm-boost.toml](helm-boost.toml) into the **executing host's**
Helm configuration (normally `~/.config/helm/config.toml`), retaining existing
settings. Its relative `web/artisan` path assumes a repository-root workspace;
use the absolute path to `web/artisan` for other workspaces. Start a new voyage
and check `/tools` for the discovered Boost tools. Helm does not automatically
load this snippet or Codex's `.codex/config.toml`. Existing voyages do not gain
new MCP tools in place. See [MCP configuration](../docs/configuration.md#mcp-tools-and-artifacts).

For other MCP clients, configure a stdio server launching `php` with arguments
`["/absolute/path/to/web/artisan", "boost:mcp"]` and environment variables
`APP_ENV=local` and `LOG_CHANNEL=stderr`. These overrides apply only to the
Boost process: the checkout's existing environment may disable development
commands, and its configured log directory may not be writable locally.
Do not change production environment/debug settings to enable Boost.

Boost can inspect application data and logs and exposes database tools; only
connect trusted local development agents, with an appropriate local database
configuration. Enabling the local environment does not replace credentials or
isolate the database. No database query is needed for the initialization/tools
listing smoke check. Do not expose this server publicly. Production deployments
should continue using `composer install --no-dev`; no automatic `boost:update`
Composer hook is installed, so production updates do not depend on a dev package.
