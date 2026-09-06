# Provider architecture and Codex independence

Helm owns the agent loop, tools, policy, approvals, sessions, todos, terminals, and
subagents. A provider is only a model transport: it receives provider-neutral
messages and tool definitions and returns text and structured tool calls. Credentials
never enter saved sessions. A native provider may persist only a typed, versioned,
size-bounded continuation envelope; Helm clears that state when a model changes.

With planned [multi-Helm participation](voyages.md), this boundary applies separately to
each executing Helm. Provider configuration and credentials stay on that Helm;
Vessel relays control and events. The interface or coordinating role does not
select a shared provider account or transfer credentials to participants.

## Access and billing boundaries

| Configuration | Current transport | Credential and billing boundary | External Codex process |
| --- | --- | --- | --- |
| `provider = "openai-responses"` | Native Responses HTTP/SSE | API key named by `api_key_env`; API-account usage | Not used |
| `provider = "openai-chat"` | OpenAI-compatible Chat Completions HTTP/SSE | API key named by `api_key_env`; endpoint-defined billing | Not used |
| `provider = "chatgpt-oauth"` | Native ChatGPT Codex Responses HTTP/SSE | Helm-managed OAuth tokens; ChatGPT plan limits | Not used |
| `provider = "anthropic"` | Native Messages HTTP/SSE | API key named by `api_key_env`; Anthropic API-account usage | Not used |
| `provider = "codex-compatibility"` | Codex app-server compatibility bridge | Existing Codex sign-in and applicable plan limits | Required today |

`openai` and `openai-compatible` select Chat Completions
and serialize canonically as `openai-chat`. Select `openai-responses` explicitly for
the native Responses protocol. The `codex-subscription` name is accepted as
an alias for `codex-compatibility`.

ChatGPT/Codex subscription access and OpenAI API access are separate billing and
credential products. A ChatGPT subscription does not supply API credits or an API
key; native API requests are billed to the API account behind the configured key.
The compatibility bridge does not read `OPENAI_API_KEY`. Check current provider plan
terms and pricing before choosing a transport.

## Native architecture

Native adapters are Helm's independence boundary. They connect directly to provider
HTTP/SSE APIs and stream into Helm's own loop. Helm therefore works when no `codex`
executable is installed or discoverable, and it never launches Codex for a native
provider. An OpenAI-compatible service can be selected with `provider =
"openai-responses"` when it implements the Responses API, or `provider =
"openai-chat"` for the Chat Completions protocol. Set `base_url` for either
custom endpoint.

The native subscription architecture follows the same principle: `helm auth login`
uses ChatGPT OAuth and `provider = "chatgpt-oauth"` talks directly to the Codex
Responses backend, with credentials managed by Helm and no Codex child process.
`helm auth login --device` supports headless machines; `helm auth status`, `logout`,
and explicit `import-codex` credential import complete the credential lifecycle.
The explicitly named `codex-compatibility` provider remains available but is not the
native subscription architecture.

The ChatGPT Codex backend is an internal product endpoint rather than a documented
public OpenAI API contract. Direct subscription access is therefore explicitly an
experimental transport and may need maintenance when that backend changes. The
public `openai-responses` provider is the stable choice when API billing is acceptable.
The generic `base_url` never redirects subscription credentials; the separate
`chatgpt_base_url` override must be set deliberately and should normally remain unset.

## Compatibility bridge boundary

The current compatibility adapter starts `codex app-server --stdio`, uses its model
transport and experimental dynamic-tool protocol, forces the thread read-only with
elevation disabled, and rejects foreign tool requests. Helm still owns every tool
execution and approval. The bridge can require updates when the external app-server
protocol changes and is never selected implicitly by a native provider.

## Configure a native provider

1. For an existing ChatGPT subscription login, either run `helm auth login` (or
   `helm auth login --device` on a headless host), or explicitly import it with
   `helm auth import-codex`. The import reads the credential file; it never executes
   Codex. Then set `provider = "chatgpt-oauth"` and run `helm models`.
2. For separately billed API usage, obtain an API credential and keep it in an
   environment variable, never in Helm's TOML. Select a native provider and model:

   ```toml
   provider = "openai-responses"
   model = "YOUR_API_MODEL"
   api_key_env = "OPENAI_API_KEY"
   # base_url = "https://compatible.example/v1" # optional
   ```

   For Anthropic, select `provider = "anthropic"` and normally
   `api_key_env = "ANTHROPIC_API_KEY"`.
3. Run `helm config`, `helm doctor`, and `helm models`, then execute one small bounded
   task. These checks make the chosen transport and credential source explicit.
4. Resume or branch existing sessions as needed. Messages are provider-neutral, but
   model identifiers and behavior differ, so explicitly choose a supported model.
5. Codex may now be removed from `PATH`; `codex_command` is ignored by native
   providers.

To select the optional bridge, set `provider = "codex-compatibility"`, install a
compatible Codex CLI, authenticate it, and run `helm doctor`. A transport change
never expands filesystem, command, approval, or tool authority.

## Independence release gate

`tests/system/native_provider_no_codex.py` runs the real Helm binary against offline
API-key and subscription Responses fixtures. It exercises model discovery, a tool
loop, typed reasoning replay, session persistence, and cross-process resume while a
sentinel `codex` executable on `PATH` records and fails any invocation. CI and the
Linux release build run this without real credentials or external network access.

### Leading instructions in compatible Chat requests

The native `openai-chat` serializer combines consecutive leading plain system
messages into one system message, keeping their content in order with two newlines
between blocks. This supports templates that accept only one initial system block,
including runtime reconciliation guidance. It changes only the outgoing Chat
request: saved conversation history, role authority, tool calls, context preflight
and output limits remain unchanged. Other roles and later system messages retain
their positions; unusual tool metadata on a system message is preserved rather
than discarded. Responses and Anthropic serialization are unaffected.

This compatibility behavior does not validate model accounting or grant completion.
The runtime still decides whether a proposal is accepted. Offline native HTTP tests
cover ordinary and streaming requests; successful live reconciliation remains a
separate acceptance check under #83.
