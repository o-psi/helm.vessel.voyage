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
requests do not follow redirects. Local compatible endpoints and keyless access
require explicit configuration; inspect `helm local-provider --help` for discovery,
probe and config-generation commands before changing endpoints.

```sh
vessel auth login
vessel auth login --device
vessel auth status
vessel auth import-codex
vessel auth logout
```

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
to recover a saved first send and press F4 to check its outcome before retrying.
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
