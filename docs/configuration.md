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
Anthropic rejects these generic explicit overrides; its different thinking/tier
configuration is not silently translated. Catalog metadata cannot broaden adapter
support.

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

All providers are native and do not require a Codex executable. Config aliases
`openai` and `openai-compatible` select `openai-chat`. Provider aliases are not
additional CLI `--provider` choices.

The external Codex bridge has been removed. Old provider values
`codex-compatibility` and `codex-subscription` are rejected, not silently migrated.
Choose a native provider explicitly and configure its credentials. Remove the
obsolete `codex_command` setting and any `[sandbox]` keys `bridge_read`,
`bridge_inherit_env`, and `bridge_network`; the latter are no longer accepted.
Existing native OAuth logins and explicit `vessel auth import-codex` credential-file
import are unchanged; neither launches Codex.

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
Vessel connection and scoped session credentials. Helm only exposes read-only local
credential diagnostics; Voyage loads and refreshes tokens on the executing host.
`helm auth` is no longer supported; update scripts to call `vessel auth`.
Status reports cached credentials, not a live provider validation. Logout removes
the cache but does not revoke tokens already cached by running Voyages; stop those
processes when changing accounts or logging out.

Subscription transport uses an internal
product endpoint and remains experimental. Never copy tokens into sessions,
project configuration, command arguments or Vessel connection credentials.

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

### Subagent worktree roots

Unrestricted mode does not delegate arbitrary worktree paths. Isolated subagents
require the planned workspace to be covered by **both** parent `allow_read` and
`allow_write`, including canonical-path/symlink checks and inherited ceilings.
Voyage plans worktrees under its session resource directory:
`resources/worktrees/<workspace-hash>/<name>-<uuid>`. A refused launch reports the
planned destination before creating the worktree.

An operator may provision the narrowly scoped worktree parent directory and add
that existing path to both lists in an executing-host private configuration,
preserving other settings. Apply configuration with idle `/configure` after
cleanup is observed, or select it at launch. Access-mode changes alone do not
change these roots. Do not grant the entire session storage directory merely to
make worktrees available, and never relocate work to evade a refusal.

Repository discovery supports ordinary clones, linked worktrees and this
checkout's `.local-git/worktree.git` metadata. Missing repositories and malformed
metadata have distinct diagnostics. Archived subagent IDs remain readable with
`status`/`wait`; they are not restartable. Use `spawn` for genuinely new work, not
an automatic replay of uncertain effects.

See [tool validation and outcomes](runtime-contract.md#tool-validation-and-outcomes) for command parsing,
action schemas and result classifications.

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

Native HTTP providers remain outside tool subprocess isolation and keep credentials
on the executing host. Required isolation on other platforms is unsupported and
fails closed.

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
`provider_retry_max_ms` bound retries before streamed content arrives. Exponential
waits use half-to-full equal jitter. Explicit `Retry-After` seconds or HTTP dates
are honored for throttling and transient service errors; a server delay above the
configured maximum returns a failure instead of retrying early. Cancellation
interrupts the wait, and exhausted accounts are not retried. Missing usage remains
unknown in inference history. See the [runtime contract](runtime-contract.md#provider-outcomes-retries-and-accounting)
for completion signals and replay boundaries. Optional
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
is a legacy import/recovery source (journal schema 8). Vessel connection and
scoped grant credentials are separate from provider credentials.
Back up private stores consistently while their owners are stopped, and follow
[Operations](operations.md) for explicit upgrade and recovery. Do not delete state
or restore an older grant to bypass an unresolved operation.

## MCP tools and artifacts

Voyage owns MCP connections, discovered tools, argument validation and cleanup.
Vessel supervises the voyage process and authorizes observation/download requests;
Helm presents its results. MCP supports stdio and Streamable HTTP using protocol
version `2025-06-18`. Provider HTTP and the public Vessel HTTP API are separate
transports.

Configure either a command or a URL for each named server:

```toml
[mcp_servers.files]
command = "mcp-server-filesystem"
args = ["/approved/workspace"]

[mcp_servers.remote]
url = "https://tools.example.com/mcp"
bearer_token_env = "MCP_REMOTE_TOKEN"
```

`bearer_token_env` explicitly grants the HTTP transport access to one executing-host
environment variable. It is resolved directly on that machine and is not added
to subprocess environments or persisted configuration. Do not put this variable
in `inherit_env`; generic inherited tool environments refuse credential names.
Credentials stay on that machine; Helm does not forward them. Endpoints require HTTPS; literal loopback IP HTTP is supported for
local servers. Userinfo, query strings, fragments and redirects are refused.
Hostnames over HTTP, legacy separate SSE endpoints, ambient HTTP proxies and
OAuth discovery are not supported by this configured transport. Use an explicitly
provisioned bearer credential where required. Configured stdio entries remain
compatible; HTTP entries must not also specify a command or arguments.

Read-only startup does not connect or launch MCP servers. Approval mode requires
approval per MCP call; unrestricted mode still respects execution authority and
configured limits. A server's annotations never grant permission or turn an MCP
call into a trusted read-only tool. Stdio runs under the applicable subprocess
policy; HTTP runs in Voyage's transport and is not confined by subprocess mounts.
Required sandbox mode with denied network also refuses MCP HTTP startup.

### Discovery and validation contract

The live registry is the capability manifest: stable namespaced tool name,
description, input schema, optional output schema and optional MCP annotations
(title and read-only/destructive/idempotent/open-world hints). Registry entries
are snapshotted at registration, names are collision checked, and declarations
cannot authorize new capabilities. Declarative skill-package manifests and the
extension SDK are separate contracts. Discovery supports bounded pagination,
up to 4,096 tools and 128 pages, with repeated cursors refused. A new runtime
rediscovers tools; unsolicited list-change notifications do not mutate a running
registry.

Voyage compiles input and output schemas locally at registration. Arguments are
validated before approval and dispatch. Successful results with an output schema
must contain matching `structuredContent`; schema failure after a call is a
reported result failure and never triggers a retry. Schema constants and instance
values are excluded from validation diagnostics. Known secrets in executable
metadata refuse provider dispatch; natural descriptions receive a separate redacted
projection.

Supported JSON Schema drafts are 4, 6, 7, 2019-09 and 2020-12, within bounded
local validation: 256 KiB per schema, 1 MiB per instance, 4,096 nodes and depth 48.
Nonrecursive local JSON-pointer references are supported. Remote/file references,
recursive/dynamic references, nested schema identifiers, unsupported required
vocabularies and regex features
requiring backtracking are refused explicitly. Formats remain annotations.
Invalid or unsupported discovered schemas fail runtime construction rather than
silently weakening validation.

### Concurrency and failure contract

Each foreground agent executes its returned tool calls sequentially. Each MCP
connection has one in-flight RPC; queue wait counts toward the configured tool
`command_timeout_secs`. Subagents construct their own registries and connections,
and separate voyages execute independently. These limits do not promise a global
machine-wide MCP scheduler or a configurable per-tool parallelism setting.

Queued cancellation sends nothing. After possible dispatch, transport failure, timeout or
dropped callers retire the connection; ordinary calls may send bounded best-effort
cancellation. Initialization is never cancelled with an MCP notification. No
transport reconnects or automatically resends effects. Late responses cannot
revive a retired connection. Stdio cleanup observes owned local processes; HTTP
cleanup retires the local connection and attempts bounded session DELETE when a
session ID exists. Remote cancellation or DELETE never proves an external effect
stopped. HTTP session expiry requires explicit new runtime initialization.

JSON frames are limited to 1 MiB; SSE streams have bounded lines, event data,
aggregate bytes and unrelated-message count. Nonmatching HTTP responses, malformed
JSON/SSE, unsupported initialization and incomplete streams fail closed. Only ping
is answered as a server request; servers cannot execute Voyage tools through the
client connection.

### Ordered results and downloads

Tool results retain ordered text, images, audio, embedded text/binary resources,
resource links, `structuredContent` and the MCP error flag. Unsupported block types
are explicit failures. Optional presentation metadata such as icons and content
annotations is not part of this result contract. Resource URIs are references and
are never fetched automatically.

Binary content is stored privately under its owning session. Canonical history
contains immutable UUID/SHA-256/size/MIME references, never base64 payloads or local
paths. Native provider tool outputs use a deterministic text projection containing
all text, resource metadata and structured output; binary bytes are not automatically
sent for model vision/audio interpretation. The complete typed result remains in
history and authorized operators can download the original bytes. Helm shows
attachment metadata beside tool activity.

```sh
helm connect artifact SESSION_UUID ARTIFACT_UUID ./result.bin
```

Use the normal `connect --access-file ...` selection for a remote Vessel. Download
requires session-history authority, works after idle suspension without waking an
agent, checks immutable metadata, size and SHA-256, and publishes to a new local
file without overwriting it. Content is never automatically opened. The public
`read_artifact` operation supports bounded 64 KiB chunks and uses opaque IDs scoped
to the selected session.

The private store permits up to 256 artifacts, 4 MiB per blob and a 64 MiB database;
transport limits can impose smaller effective results. Image blocks additionally
require actual PNG/JPEG/WebP validation under the existing 2 MiB image limit.
Audio and other resource MIME types are server declarations, not executable-content
trust. A result has at most 128 blocks. Configured known secrets in binary bytes,
identifiers or structured output cause refusal; natural text is redacted before
publication. This is known-value redaction, not arbitrary encoded-secret detection.

Artifacts survive resume and local branching with their immutable IDs. Clear and
failed calls can retain bounded unreferenced blobs until explicit session deletion;
there is no automatic eviction of referenced data. Delete purges the artifact
store. Cross-Vessel owner transfer and legacy JSON import of binary-bearing histories
fail explicitly, as image-bearing transfers already do, rather than create dangling
references. Typed-result persistence upgrades journal schema 8/9/10 to 11 under the
runtime ownership fence; older runtimes refuse the upgraded journal.

These storage and process behaviors require actual native verification. Current
acceptance evidence is Linux with synthetic local peers; it does not establish
native macOS/Windows private-storage behavior or live-provider results.
