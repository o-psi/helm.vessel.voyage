# Helm website

Laravel 13, Livewire 4 and free Flux UI 2 product landing page for
https://helm.vessel.voyage. Foleybridge.Software presents Helm, Vessel, and Voyage
as a product suite for running coding agents across machines the user controls.
Product copy leads with customer outcomes while accurately distinguishing current
Linux behavior from planned web, mobile, and cross-voyage collaboration. The
interface selector uses actual Livewire requests. This app does not connect to
Vessels, execute agents, enroll users or collect email addresses.

## Development

Requires PHP 8.3+ with Laravel extensions, Composer, and a Node version supported
by the locked Vite release (Node 20.19+ or 22.12+).

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
migrations. No authentication routes are enabled.

## Deployment

The dedicated unprivileged Debian 13 CT is named `helm-web`: 2 cores, 2 GiB RAM,
512 MiB swap and a 16 GiB root disk. DHCP attaches to the existing bridge. Container
nesting is enabled for Debian systemd mount compatibility; no host devices are
passed through. It starts at host boot.

Application: `/srv/helm/app`, owned by the unprivileged `helm` deployment account.
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

Generate the application key once, retain it across upgrades, then run migrations
with `--force`, `npm ci`, `npm run build` and `php artisan optimize`. Run Composer
and build commands as `helm`. Reapply shared directory permissions after caching.

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
Helm title, and its CSS, Flux and Livewire scripts must return 200. Exercise all
three interface choices and confirm the planned labels for web and mobile.
`/.env` must return 403; unknown pages must return 404. Debug output stays disabled.

If public resolution lags, inspect authoritative/public DNS independently; do not
mistake a resolver cache for a failed tunnel. Tunnel registration alone does not
establish working application routing. The connector's disabled ICMP proxy is
unneeded for this HTTP-only origin.

Before upgrades, back up the application `.env`, database (using an SQLite-aware
backup), and tunnel token to operator-controlled private storage, or take a stopped
container backup. No off-host backup schedule is provisioned by this change.
Rollback application source and its matching assets together; database rollback
requires the matching backup. Do not regenerate keys during recovery.

Initial checks: production build and Laravel optimization, migrations, Composer
validation/audit, origin HTTP/health, public HTTPS/assets, actual Livewire POSTs
for terminal/web/mobile, environment-file denial and service health. These are
HTTP/runtime checks; no browser visual QA or agent execution was performed.
