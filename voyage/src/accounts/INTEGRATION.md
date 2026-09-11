# Account backend integration (#213, sequential backend phase)

This is a host backend, not a completed Helm/runtime/remote-account feature. No
commit or publication is part of this phase. The new protocol module is
`voyage_protocol::accounts`; runtime types are in `voyage::accounts` (imported as
`voyage_runtime::accounts` by Vessel).

## Registry and admitted bindings

The principal integration signatures are:

```rust
Registry::new(root: PathBuf) -> Registry
Registry::default_host() -> anyhow::Result<Registry>
Registry::list(&self, allowed: impl Fn(&AccountDescriptor) -> bool)
    -> anyhow::Result<(u64, Vec<AccountDescriptor>)>
Registry::connection(&self, id: Uuid) -> anyhow::Result<ConnectionDescriptor>
Registry::freeze(&self, id: Uuid, transport: Transport)
    -> anyhow::Result<AccountBinding>
Registry::validate_binding(&self, binding: &AccountBinding)
    -> anyhow::Result<AccountDescriptor>
Registry::resolve_api_key(&self, binding: &AccountBinding) -> anyhow::Result<String>
Registry::token_store(&self, binding: &AccountBinding)
    -> anyhow::Result<ChatGptTokenStore>
Registry::oauth_provider(&self, binding: &AccountBinding)
    -> anyhow::Result<ChatGptOauthProvider>
Registry::enrollment_actor(&self, account: Uuid)
    -> anyhow::Result<Option<EnrollmentActor>>
```

`AccountBinding` contains `account_id`, `connection_id`, `identity_generation`,
`connection_revision`, and `transport`. Persist exactly this binding in trusted
launch/configuration and freeze it at run admission, including subagents, title
work, discovery and retries. Do not persist a label as identity or silently call
`freeze` on a stale saved selection to refresh its generation. A rejected binding
requires explicit review, not fallback.

`validate_binding` checks identity, lifecycle and connection compatibility. It
returns an observed descriptor, including local credential availability; it does
not establish provider entitlement. `resolve_api_key` is synchronous for
`Config::api_key()`. **Native API adapters must resolve again for every outgoing
request/retry/discovery**, not keep the string from provider construction. Validate
that the configured endpoint equals the binding's approved connection endpoint;
transport compatibility alone never authorizes a new endpoint. The native OAuth
factory `oauth_provider` performs that endpoint selection safely. Low-level
`token_store` callers must use the matching approved OAuth endpoints.

`AccountDescriptor` exposes UUIDs, safe ASCII alias/label, `metadata_revision`,
`identity_generation`, `credential_revision`, `capability_revision`, lifecycle
`state` and observed `availability`. Scope catalog/continuation state to the
binding and `capability_revision`; OAuth refresh increments credential revision
but not capability revision. Explicit reauthentication/API rotation invalidates
permission-relevant observations. No provider identifier or credential is listed.

The registry does not grant authority: filter before returning descriptors and
recheck current account-use allowlists/workspace authority at mutation and run
admission on **both** Vessel and Voyage. `connections()` is an owner-local admin
list, not an unfiltered remote catalogue. Enrollment provenance is host-internal
input to grant filtering, not account-use permission by itself.

## Shared host device service

```rust
type Authorize = Arc<dyn Fn(&EnrollmentActor, Uuid) -> bool + Send + Sync>;
DeviceService::new(registry: Registry, authorize: Authorize) -> DeviceService
DeviceService::start(&self, request: EnrollmentRequest)
    -> impl Future<Output = anyhow::Result<EnrollmentStatus>>
DeviceService::status(&self, id: Uuid, actor: &EnrollmentActor)
    -> anyhow::Result<PrivateEnrollmentStatus>
DeviceService::cancel(&self, command_id: Uuid, id: Uuid, actor: &EnrollmentActor)
    -> anyhow::Result<EnrollmentStatus>
DeviceService::drive(&self, id: Uuid, actor: &EnrollmentActor)
    -> impl Future<Output = anyhow::Result<EnrollmentStatus>>
DeviceService::run(&self, id: Uuid, actor: &EnrollmentActor)
    -> impl Future<Output = anyhow::Result<EnrollmentStatus>>
DeviceService::resume_candidates(&self)
    -> anyhow::Result<Vec<(Uuid, EnrollmentActor)>>
```

These types live in `voyage::accounts::device`. `EnrollmentRequest` has distinct
`command_id`, `enrollment_id`, `connection_id`, `alias`, `label`, and
`actor: EnrollmentActor { principal, workspace }`. Construct the actor from the
**authenticated** identity/workspace; do not trust an actor supplied in a remote
JSON body. The callback must consult current enrollment permission for that actor,
workspace and connection, including revocation. It runs again before effects and
inside final publication. It must be fast, synchronous and must not reenter the
registry (the publication lock is held).

Reserve and retain the request IDs before starting. `start` durably deduplicates
exact requests and never repeats an unresolved start. `drive` is one bounded tick;
`run` owns a bounded polling loop. Vessel must supervise the worker independently
of an HTTP request or Helm view. On supervisor recovery, `resume_candidates`
provides host-internal work identities; it is **not** a remote enumeration API.
Starting/exchanging states after a crash are not replayed: they become uncertain
at expiry. Pending polls can resume. Dropping a worker during a network operation
may leave a durable uncertain operation; never call a fresh start as a retry.

Only `PrivateEnrollmentStatus` exposes temporary `user_code`/`verification_uri`,
and only while pending after matching the actor and checking current authority.
It intentionally has no `Debug`. Never put this response into generic command
receipts, snapshots, SSE, tool output, conversation, or diagnostics. Ordinary
`EnrollmentStatus` has only ID/state/account ID/expiry and
`effects_may_have_occurred`. Private display material is cleared on terminal
states. Provider tokens/device polling secrets stay in the private host checkpoint.

Cancellation and final account publication share one transaction. Cancel winning
prevents publication; publication winning returns `Succeeded`, not a fictitious
undo. Cancellation/expiry do not attest upstream revocation. Pending, slow-down,
denied, expiry, bounded response errors and uncertain exchange responses have
separate handling. The production service permits only native ChatGPT device
authorization and its exact verification URL, never caller-selected endpoints.

## Host-local administration and migration

Other public registry operations are `add_connection`, `add_api`, `add_oauth`,
`rotate_api`, `reauthenticate_api`, `reauthenticate_oauth`, `rename`, and `logout`.
Their complete signatures are in `accounts.rs`. `ApiKeyInput` is either
`Stored(String)` or `Environment(String)` and deliberately has no `Debug` or wire
serialization. Use `accounts::private_input::api_key()` only in an attached private
human terminal. Environment bindings hash the admitted value privately and fail
closed if it changes. Same-identity API rotation needs an explicit owner
attestation; otherwise identity generation advances. This is not provider identity
verification. OAuth reauthentication compares the retained provider identity;
different identities require explicit replacement. Removed UUIDs cannot be revived.

`logout(id, remove)` advances identity generation and tombstones credentials in one
checkpoint. Already dispatched requests cannot be recalled and upstream tokens
are not revoked. Old refresh commits cannot resurrect a logged-out account.

```rust
Registry::migrate_legacy_oauth(&self, old_writers_stopped: bool)
    -> anyhow::Result<Option<Uuid>>
Registry::migrate_legacy_api(&self, endpoint: String, transport: Transport,
                           environment: String)
    -> anyhow::Result<AccountBinding>
```

OAuth migration is explicit and idempotent, including absent/null caches. The
owner must stop old binaries first: no marker can fence binaries that do not read
it. The original cache is retained as upgrade data, not deleted. After migration,
new default-path `TokenStore`s resolve the stable registry UUID; pre-migration
instances refuse further legacy access. Registry logout never causes re-import.
Legacy native environment configuration can be materialized once via
`migrate_legacy_api`; subsequent calls return the original binding, not a new
account or generation. Parent integration must persist the resulting binding in
old trusted configs/drafts and retain the existing transport/billing choice.

The named CLI is `vessel auth accounts`:

- `connections`, `connect --label ... --endpoint ... --transports ...`
- `list`, `status --account UUID`
- `add --connection UUID --account ALIAS [--env ENVIRONMENT_NAME]`
- `login --connection UUID --account ALIAS` (private-terminal device flow)
- `rename --account UUID --alias ... --label ...`
- `logout --account UUID [--remove]`
- `import --connection UUID --account ALIAS --path PRIVATE_FILE`
- `reauthenticate --account UUID --generation N --path PRIVATE_FILE [--replace-identity]`
- `rotate-api --account UUID --generation N [--env NAME] [--attest-same-identity]`
- `migrate-legacy --old-writers-stopped`
- `enrollment-status --enrollment UUID`, `cancel-enrollment --enrollment UUID`

Existing `vessel auth login/status/logout/import-codex` shapes remain compatible.
Their legacy browser/device login path is retained; **named** device enrollment
uses the new shared service. Consolidating the old CLI spelling into the new
private enrollment lifecycle is not claimed in this backend phase.

## Bounds, storage and remaining integration

One atomic **private** checkpoint contains separate credential and metadata
structures; only safe projections leave storage. It uses existing native
owner-private, symlink/hardlink-safe directory primitives. Registry lock waits
are bounded to 750 ms; credential readers wait up to three seconds for another
process's refresh. Refresh intent is per account and durable before network IO;
no network exchange holds the registry lock. The legacy file store has an
analogous durable refresh fence. OAuth requests have bounded timeouts and response
sizes. Refresh failure/crash stays fenced until explicit reauthentication.

Limits are 64 KiB per private checkpoint, 64 profile/tombstone records, 32
connections, 64 retained enrollment records, 8 active attempts, 16 cancel IDs per
attempt, and a maximum ten-minute enrollment lifetime. Byte capacity may be
reached before record counts. Capacity/storage failures refuse publication; a
lost/failed post-exchange publication stays unresolved rather than reissuing the
exchange. Retained command identities are not pruned automatically. Expanding or
compacting this bounded store is not part of this phase.

Parent still owns account-aware Config/launch persistence, endpoint matching and
per-request API credential resolution, scoped remote grants/endpoints, private
Helm presentation, atomic next-run selection, and active-run/cache integration.
This phase does not claim remote/UI end-to-end completion, native macOS/Windows
security evidence, live provider validation, or new organization/project headers.
No provider bridge, paid inference, or real login was used.
