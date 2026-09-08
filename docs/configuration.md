# Configuring the current implementation

Execution configuration and policy are resolved in the voyage process on the
executing host. Helm submits configuration references through Vessel; remote routes
never copy provider credentials. See [Architecture](architecture.md) and
[Operations](operations.md).

## Runtime configuration

By default a launched owner uses that host's `helm/config.toml`. Explicit
`helm connect new --config-path /absolute/host/config.toml --workspace /absolute/project`
selects an owned private configuration file. Normal local `helm --config`, `--set`,
provider and policy options capture a private launch envelope on the same executing
machine. This preserves explicit policy selection while revalidating current host
ceilings. Provider environment credentials must be available to the supervisor
before it starts the child. OAuth tokens stay with the executing account.

A voyage is bound to one workspace and session identity. `/model NAME` selects the
next-turn model. `/configure /absolute/host/file` replaces settings only while idle
with required cleanup observed. Persisted settings take precedence on restart;
removed or revoked authority is never silently revived. On ordinary resume, only
explicit configuration/model overrides request a change; reconnect alone does not
mutate active work. Configuration drafts remain distinct from session history.

The Linux installer service uses that user's configuration; see its
[service guide](../installer/README.md). Custom workflow directories are host paths
and require owner authority. Participant endpoint configuration names a private
scoped credential file, receiver identity and accepted binding revision; see
[process access](process-access.md).

## Native Vessel coordination

The `vessel` tool is enabled by default. Supervised voyages receive their own
session identity and supervisor directory at bootstrap; these runtime-only values
cannot be supplied through saved conversation or configuration. Its bundled
coordination skill is included only when the tool is in the actual registry.

```toml
[vessel]
enabled = true
# Optional discovery directory for an embedding without supervised context:
# local_directory = "/absolute/private/vessel"

[vessel.remotes]
# Named routes use private AccessCredential files, not inline tokens or URLs:
# build = "/absolute/private/build-vessel-access.json"
```

`local` denotes the supervising Vessel; other tool targets are configured aliases.
The route uses the credential's real grant rights: a session-scoped grant does not
acquire host-level creation authority. No route is enrolled or supervisor started
implicitly. Set `vessel.enabled = false` to omit the tool and bundled guidance.
See [Vessel coordination](vessel-coordination.md) for action and receipt semantics.

## Configuration sources

Use [helm/config.example.toml](../helm/config.example.toml) as the field reference.
The default file is the platform config directory's `helm/config.toml`, normally
`~/.config/helm/config.toml` on Linux. `--config /absolute/config.toml` selects
another file. Current loading falls back to defaults when the selected file does
not exist, so verify the resolved configuration rather than assuming a path loaded.
Malformed or unreadable existing files report an error.

```sh
helm --config /absolute/config.toml config
helm --config /absolute/config.toml doctor
helm --config /absolute/config.toml --set command_timeout_secs=60 config
```

`--set KEY=VALUE` applies invocation overrides; explicit `--provider`, `--model`
and `--access` flags are applied afterward. Chat can remember selected preferences,
and resumed sessions retain their own model/workspace where no explicit override
applies. `helm config` conceals secret bindings. `helm doctor` checks configuration
and dependencies without contacting a model. `helm models` queries the configured
provider/account and therefore requires the relevant credentials and network access.

## Next-turn inference controls

The composer exposes model, thinking, and service selectors. `/model`, `/thinking`,
and `/service` open the corresponding picker; a value selects it directly:

```text
/model MODEL_ID
/thinking high
/service priority
/thinking inherit
/service inherit
/service default
```

`inherit` clears the explicit override. Voyage selects an advertised catalog
value when one exists; otherwise the request is provider-managed and the actual
default is unknown. `/thinking default` remains a compatibility alias for
`/thinking inherit`. **`/service default` is now an explicit choice, not a clear
operation.** On the OpenAI API it sends `service_tier = "default"` (standard
service). On ChatGPT OAuth it disables catalog tier selection and omits the wire
field, matching that backend's client convention; this does not claim a delivered
tier. Use `inherit`, not `default`, to restore catalog selection.

Explicit launch fields are `reasoning_effort` and `service_tier`:

```toml
reasoning_effort = "high"
service_tier = "priority"
```

Omitting a launch field enables catalog/provider inheritance. Catalog defaults
are actually encoded when selected, not merely displayed as guesses about backend
omission. **A catalog-selected service tier may affect cost or allowance.** Use an
explicit service choice to override it. No rejection causes an automatic downgrade,
tier change, or support-probing inference request.

Native ChatGPT discovery uses the executing account's authenticated catalog:
`supported_reasoning_levels`, `default_reasoning_level`, `service_tiers`, and
`default_service_tier`. OpenAI's public/compatible model-list endpoint does not
establish reasoning defaults, project service defaults or account entitlements.
Those remain unknown rather than being inferred from model names or plan labels.
OpenAI-compatible endpoints are not assumed to share OpenAI's backend defaults.
Anthropic and the optional Codex compatibility bridge still reject these generic
explicit overrides; their different thinking/tier configuration is not silently
translated. Compatibility catalog metadata cannot broaden adapter support.

The shared resolution distinguishes unknown, unsupported, and model-advertised
choices; explicit requests; advertised defaults; resolved wire values; source and
observation time; and account support (unknown unless established). Model-advertised
support is not a promise of account entitlement, availability, pricing or acceptance.
Priority may cost more. Actual response service metadata, when present, is separate
from the requested tier. The active-turn picker shows the last completed response's
reported service; this is a live observation, not a durable billing ledger or a claim
about every request in that turn. Absent/malformed response metadata is not guessed.
Automatic title requests use a separate utility model and its provider defaults.

Runtime catalog observations expire after five minutes and are bound to provider,
endpoint and executing credential/account context. Logout/account/endpoint changes
cannot reuse another context's catalog. Failed automatic discovery backs off for
30 seconds; reopening a picker explicitly retries. Background snapshots do not wait
for network discovery. Local drafts discover through temporary supervised Voyages
and check their context every 30 seconds while selected; their metadata is advisory,
and sending always invokes executing-host validation. Suspended/restarted runtimes
may show unknown metadata until discovery runs again; explicit settings remain saved.
Catalog errors never turn unknown support into known unsupported. Catalog fields
are bounded and screened for unsafe terminal text and credential disclosure.

Defaults and validation are resolved again before provider dispatch and frozen for
the admitted turn, including retries and tool continuations. Mid-turn model/catalog
changes only affect future turns. The provider can still reject an encodable request.

Selections on an unsent new-voyage draft stay local until creation. For an existing
voyage, Helm submits one atomic `set_inference` replacement of model and both
optional overrides, using the observed session revision, an immutable command ID,
and a deadline. A concurrent checkpoint or another client can cause a revision
conflict. Refresh and make an explicit new selection, not a blind stale retry.

Vessel durably binds the intent before forwarding it; Voyage validates and journals
the effective configuration and the applied or rejected outcome. Only an applied
Voyage receipt means the update took effect. An uncertain delivery is resolved by
its existing command identity, never automatically replayed as new work. Vessel's
intent record is not an offline delivery queue or a second source of effective
settings. Settings mutation requires executing-account owner authority.

A running turn keeps its original settings for the entire tool loop. Accepted
changes govern the next admitted turn, including after suspension or restart.
Model switches do not modify an active turn's provider replay; the replay reset
happens atomically at the next admission. Existing child agents retain their
construction-time settings. Snapshots distinguish next-turn requested settings
from the active turn's frozen settings. Rejections preserve effective settings.

## Providers and authentication

| Provider value | Transport and credentials |
| --- | --- |
| `openai-responses` | Native Responses; API key from `api_key_env`. |
| `openai-chat` | Native compatible Chat Completions; endpoint-defined API credentials. |
| `chatgpt-oauth` | Native experimental subscription transport; Vessel-managed OAuth tokens. |
| `anthropic` | Native Messages; normally `ANTHROPIC_API_KEY`. |
| `codex-compatibility` | Optional external `codex app-server` bridge using its sign-in. |

Native providers do not require a Codex executable. Config aliases `openai` and
`openai-compatible` select `openai-chat`; `codex-subscription` selects the optional
bridge. Provider aliases are not additional CLI `--provider` choices.
API access and subscription access use separate credentials and billing; a
subscription does not supply API credits. This is a transport-selection boundary,
not a statement of current provider pricing or model availability.

```toml
provider = "openai-responses"
model = "YOUR_API_MODEL"
api_key_env = "OPENAI_API_KEY"
```

Set the named environment variable privately before invoking Helm. TOML names the
variable, not its secret value. `base_url` selects a deliberate API endpoint for
native compatible transports. Subscription credentials are not redirected by that
field; `chatgpt_base_url` is separate and should normally remain unset. Native HTTP
requests do not follow redirects. Native provider endpoints require HTTPS except
literal-loopback HTTP for local/development use. Userinfo and fragments are
rejected, and loopback requests bypass environment proxies. Local compatible endpoints and keyless access
require explicit configuration; inspect `helm local-provider --help` for discovery,
probe and config-generation commands before changing endpoints.

```sh
vessel auth login
vessel auth login --device
vessel auth status
vessel auth import-codex
vessel auth logout
```

The native OAuth cache and import source must be private owned regular files under
private immediate parent directories, without symlinks or hardlinks. Reads are
bounded to 64 KiB; unsafe existing storage is refused rather than silently changing
permissions. Logout atomically replaces the cache with a private logged-out
record; it does not revoke upstream tokens or remove the path. These storage
changes have Linux verification; native platform claims require separate evidence.


Choose browser login or device login, not both for one authentication attempt.
Import explicitly reads an existing Codex credential file; it does not run Codex.
Use `import-codex --path /absolute/auth.json` for another source and `--force` only
to replace existing local ChatGPT credentials. OAuth tokens live in the platform local-data
directory's `helm/chatgpt-oauth.json`. This legacy path is intentionally retained
so existing logins remain usable without copying or re-importing credentials.
Run `vessel auth` on the execution host as the account running Voyage; it is a
local command and does not authenticate a remote Vessel from Helm. For a headless
host, use `vessel auth login --device`. Provider credentials are separate from
Vessel attachment/enrollment credentials. Helm only exposes read-only local
credential diagnostics; Voyage loads and refreshes tokens on the executing host.
`helm auth` is no longer supported; update scripts to call `vessel auth`.
Status reports cached credentials, not a live provider validation. Logout removes
the cache but does not revoke tokens already cached by running Voyages; stop those
processes when changing accounts or logging out.

Subscription transport uses an internal
product endpoint and remains experimental. Never copy tokens into sessions,
project configuration, command arguments or Vessel enrollment.

## Execution policy and limits

`access` accepts `read-only`, `approval` or `unrestricted`. Read-only denies
mutations even when a write root is listed; approval requires review for actions
that need it. Unrestricted remains subject to configured roots, denials and other
limits. `unattended_approval = "deny"` refuses approval-required actions when there
is no interactive approver. `"allow"` is an explicit unattended grant.
These settings are application policy, not an OS sandbox.

`allow_read` and `allow_write` add existing roots; the active workspace is included
by the current policy adapter. `deny_commands` names denied command basenames.
Children receive an empty environment populated only from `inherit_env` and `[env]`;
secret-like inherited names are refused. Keep credentials out of child environment
values too. `redact_values` supplies additional literal output redactions.
See [Security](security.md) for enforcement and cleanup limits.

### Optional Linux process isolation

`[sandbox]` selects an additional OS boundary on the executing Voyage host.
The default `mode = "off"` retains application policy alone. `mode = "required"`
uses the system `/usr/bin/bwrap` on Linux x86_64 and refuses launch when setup
cannot succeed; approvals and unrestricted access do not disable it. Install
bubblewrap on that host and use a kernel that permits unprivileged user namespaces.
`helm doctor` reports local adapter readiness separately from provider readiness;
its bounded `/bin/true` probe checks setup, not every security property or a remote
Vessel's environment.

```toml
[sandbox]
mode = "required"
network = "denied"
address_space_bytes = 4294967296
cpu_seconds = 600
file_size_bytes = 1073741824
open_files = 1024
uid_processes = 4096
temporary_bytes = 268435456
```

The adapter mounts explicitly allowed roots plus a small read-only system runtime,
with private process/device views and temporary storage. Broad host roots and
reserved runtime mounts are refused. `network = "host"` is an explicit grant of
host IPv4/IPv6 connectivity, including local endpoints; it is not an endpoint
allowlist. Unix socket creation remains denied. Subprocess environments still use
the configured allowlist.

Address space, CPU, file size and descriptor limits apply per process. The process
count limit covers the same real UID, including processes outside this sandbox.
The temporary storage limit covers its private tmpfs. These are not aggregate
descendant memory or CPU budgets.

The optional Codex compatibility bridge has separate `bridge_read`,
`bridge_inherit_env` and `bridge_network` grants inside `[sandbox]`. They do not add
tool roots. Required mode does not automatically expose provider login stores or
grant bridge network access. Native HTTP providers remain outside tool subprocess
isolation and keep credentials on the executing host. Required isolation on other
platforms is unsupported and fails closed.

Named profiles are explicit, private choices. Create/inspect a candidate, preview
its exact revision and digest for the actual workspace, then select it at launch:

```sh
helm policy create review --preset restricted
helm policy inspect review
helm --workspace /absolute/project policy preview review --revision REVISION --digest DIGEST
helm --workspace /absolute/project --policy-profile review \
  --policy-revision REVISION --policy-digest DIGEST chat
```

A permissive transition also requires the fresh preview's `--policy-confirm` digest.
Profiles normally live under the config directory's `helm/profiles`; use an
explicit absolute `--policy-directory` to select another private store. Imported
rules are inert until selected. Saved session text cannot select a profile.
Optional `helm policy defaults` stores pinned private global/project candidates;
inspect its `init`, `enable`, `set`, `preview` and `activate` help before adoption.
Enabling writes a separately chosen config file; it does not overwrite the source.

Resolution precedence is explicit launch selection, private project preference,
private global candidate, then Config. Explicit policy overrides follow selection.
On Linux, `/etc/helm/policy-ceiling.toml` supplies the final administrator maximum;
invalid or untrusted existing ceiling state refuses execution. Profile/default
changes are checked again before new execution. They do not instantly reverse
already dispatched effects. Linux ceiling enforcement is not claimed for other OSes.

`command_timeout_secs`, `max_output_bytes`, terminal count/unread limits and
subagent concurrency bound different resources. `max_tokens = 0` and
`context_window = 0` add no operator token cap; provider-specific requirements still
apply. `provider_retry_attempts`, `provider_retry_initial_ms` and
`provider_retry_max_ms` bound retries before streamed content arrives. Optional
`helm inference` limits count dispatch attempts, not money or complete token cost.

## Storage and diagnostics

Helm stores private new-voyage drafts in `helm-new-drafts/` beside its default
Vessel directory (normally `~/.local/state/voyage/helm-new-drafts` on Linux).
These files contain local launch settings, composer text and any pending first-send
identities; they are not canonical sessions. Draft locks keep concurrent Helm
windows independent. Untouched blank drafts are not saved. Open interactive Helm
to recover a saved first send; Helm automatically continues setup and checks its outcome without replaying an attempted turn.
Keep these files private alongside the existing `helm-views/` interface drafts.

The platform local-data directory's `helm` child is the ordinary data root,
normally `~/.local/share/helm` on Linux. Legacy JSON import sources are under `sessions/`;
TUI diagnostics append to `logs/helm.log`. Non-TUI diagnostics go to stderr.
`--log-format json` changes diagnostic formatting, and `--verbose` enables more
logging; neither is a transcript-export command. Protect logs and backups as
private operator data. No automatic log-retention guarantee is implied.

Current owners store canonical data in `VESSEL_DIR/sessions/SESSION_UUID/journal/`
with private identity and resource directories beside it. Managed installations use
`STORE/vessel/sessions/SESSION_UUID/journal/`; their old `STORE/journal/journal.sqlite3`
is a legacy import/recovery source (journal schema 8). Enrollment defaults
to the data root's `attachment/`; its identities differ from provider credentials.
Remote workers use a dedicated installation and fixed enrollment binding.
Vessel's database and private attachment authority directory are separate again.
Back up private stores consistently while their owners are stopped, and follow
[Operations](operations.md) for explicit upgrade and recovery. Do not delete state
or restore an older grant to bypass an unresolved operation.
