# OAuth foundation verification and integration contract (#290)

Run from `web/` after `composer install`:

```sh
php tests/oauth.php
```

On the development host PHP has iconv installed but disabled by default; use
`php -d extension=iconv tests/oauth.php` and the same prefix for Composer.
No system PHP configuration change is required. The check boots Laravel with
an in-memory SQLite database, array sessions/cache, and synthetic credentials.
Do not run with a production cached configuration: clear the **test checkout's**
config cache first. The fixture installs `MockProviders` in the container;
its `driver()` calls the real Socialite driver and replaces only Guzzle's
transport with `MockHandler`. Tests invoke `redirect()`/`callback()` with an
attached Laravel session and mock token/profile/email HTTP responses. This
exercises real state consumption and PKCE exchange without contacting providers.
No production fake-login route or bypass exists. These are controller/service
checks, not a live provider or full HTTP routing journey.

Main integration owns:

- GET `/auth/{provider}` and GET `/auth/{provider}/callback` to
  `OAuthController::redirect` / `callback`, with web session and existing console
  enablement/HTTPS/security headers. Apply login rate limiting.
- Login links from static `OAuthController::providers()` (`id => label`), displaying
  the generic `oauth` validation error. Unknown/incomplete providers are unavailable.
- `Auth` plus assigned `user.tenant_id` enforcement; remove password tenant access.
  Logout must log out Auth and invalidate the session. OAuth success regenerates
  the session and sets `helm_operator_until` (Unix seconds, +8h) and fresh UUID
  `helm_auth_id` (ticket-session revocation identity).
- `SESSION_SAME_SITE=lax`, secure cookies, trusted reverse-proxy configuration,
  and HTTPS `APP_URL`. Callback URLs use `config('app.url')`, never request Host.

Provider applications require `GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET`,
`X_CLIENT_ID/X_CLIENT_SECRET`, or `GITHUB_CLIENT_ID/GITHUB_CLIENT_SECRET`.
Each provider enables independently only when both values are nonblank.
Register exactly `${APP_URL}/auth/google/callback`, `/x/callback`, or
`/github/callback` under the same `/auth/` prefix. X must be configured
as a confidential Web App OAuth2 client, not OAuth1. Socialite 5.31's native X
provider uses X endpoints, state, S256 PKCE and HTTP Basic token-client auth.
All three drivers enable S256 PKCE. No offline/refresh-token scope is requested;
no provider token/profile is persisted. Do not enable HTTP debug logging, request
query logging on callbacks, or external exception capture of provider responses.
No provider credentials were supplied and live sign-in is not verified.

## Additive deployment migration

Back up the database; install the committed Composer lock and run normal
`php artisan migrate --force` before enabling OAuth/routes. Existing password
users and hashes remain intact but have **no tenant**. There is no email allowlist,
email linking, legacy tenant backfill, or shared connection import. Email becomes
nullable/nonunique display data; password becomes nullable. Any old password reset
or email-based authentication must not be used for tenant login.

`tenants.id` and unique `tenants.principal_id` are generated UUIDs; `name` defaults
to `Personal tenant`. Explicit `id`, `principal_id`, and `name` are fillable for
trusted provisioning/test callers. `users.tenant_id`
is a nullable unique FK (one owner per personal tenant). `oauth_identities` has
`id`, `provider`, `subject`, `user_id`, timestamps, and unique `(provider, subject)`.
The Eloquent model explicitly uses `oauth_identities`, avoiding acronym inflection.
Opaque subjects are case-sensitive (explicit binary collation on MySQL/MariaDB).
`User::tenant()`, `User::oauthIdentities()`, `Tenant::users()`, and
`OAuthIdentity::user()` provide relationships. Tenant creation and user/identity
insertion are one transaction; duplicate identity races roll back the losing
allocation before looking up the winner. New tenants contain no connections.

Schema rollback intentionally refuses: reconstructing non-null passwords and
unique emails would be destructive. Roll back application code only, keeping the
additive schema, or develop a separately reviewed data-preserving migration.
SQLite fresh install and legacy upgrade were executed locally; native MySQL and
PostgreSQL migrations/concurrent login races still require deployment validation.
