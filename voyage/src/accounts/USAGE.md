# Private account usage and default contract (#263)

The native account-use human path supports `AccountUsage { workspace, account,
refresh }`. `refresh: false` is a private registry read only; true attempts one
read-only native ChatGPT backend usage GET, bounded to 15 seconds including token
refresh and at most 64 KiB JSON. Concurrent refreshes are refused without queuing.
No inference, credit redemption, purchase, automatic polling or Codex bridge exists.

The response is `AccountUsageObservation`: exact account binding,
`capability_revision` matching the account descriptor, optional last successful
snapshot, latest `refresh_status`, and optional `attempted_at`. Snapshot times and
reset times are UTC Unix seconds. Only primary/secondary window numeric percentage,
optional actual window seconds and optional absolute reset time are retained.
Missing windows are not zero; no valid windows is invalid/unknown. Reset expiry
never refills cached allowance. Additional categories, plan/credits and identities
are deliberately excluded. These are Codex allowance observations, not all ChatGPT
limits, remaining token/message counts, API cost or billing balances (#71).

Credentials and cache remain in private executing-host storage. Existing account-use
permission, pinned host routing, binding and capability checks apply before fetch
and before publication/disclosure. Session/participant grants cannot read usage;
owner and human connection scopes can. Errors retain no raw provider response or
identity. Last successful snapshots survive failed refreshes; logout/identity or
capability changes invalidate them. Helm must also fence responses by authenticated
host, account identity and capability revision and label age/stale/error honestly.

`Accounts` adds `default_account`, `default_revision`, and `can_set_default`.
`AccountSetDefault { command_id, workspace, account, expected_revision }` is an
owner-only compare-and-set. Exact command receipts are retained (bounded to 256,
then refuse new commands); conflicting IDs or stale revisions refuse. An old receipt
is historical evidence, not the current default: reread Accounts after mutation.
The setting is host-wide, not per-project or a change to model/reasoning/service.
A hidden unauthorized default is returned as null, never leaked to a scoped reader.

`AccountDefaults` returns `{ "code": "default_account_required", "account": null }`
when no default is initialized, so Helm can still show onboarding with Accounts.
No first account is selected implicitly. New independent voyages require a default;
when no explicit account was supplied they select that exact binding. Explicit
account overrides remain explicit. Frozen existing voyages are not rewritten.
Logout leaves the default selected (unavailable, no fallback); removal requires an
explicit replacement first. An account reauthenticated to a different identity
requires explicitly setting the new default binding, not silent generation updates.

Upstream source evidence, not live compatibility evidence: OpenAI Codex commit
`ee6814bfa4889fe9b2b3dcc9cc8bdd91effa8ab8`,
`codex-rs/backend-client/src/client/rate_limit_resets.rs` and generated
`rate_limit_window_snapshot.rs`. No authenticated provider requests were used for
this delivery's verification. Native macOS/Windows storage assurance remains #200.
