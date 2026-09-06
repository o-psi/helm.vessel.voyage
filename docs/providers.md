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

Browser login returns to `http://localhost:1455/auth/callback` (port 1457 if
1455 is busy). Helm listens only on IPv4 loopback. Keep Helm running and complete
sign-in in a browser on the same machine; use `helm auth login --device` for a
headless host. If authorization is rejected, cancel the attempt and generate a
fresh link after updating Helm. Do not edit the callback URL: the authorization
request and token exchange must use the same redirect URI. Check completion with
`helm auth status`.

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

## HTTP redirect boundary

Native Anthropic, OpenAI-compatible, and ChatGPT OAuth requests do not follow HTTP
redirects, including same-origin redirects. This applies to model discovery,
completion, streaming, and OAuth token/device exchanges. A redirect returns a
request error without displaying its body or destination. Configure the intended
final endpoint explicitly after checking its authority; Helm does not forward
credentials or replay request bodies at a redirect destination. Browser navigation
during interactive OAuth authorization remains part of the browser login flow.


## Retries and tool-call identity

Helm retries retryable provider failures only before any streamed text or tool-call
fragment has been received. Attempts are bounded by `provider_retry_attempts`.
Without a server delay, each wait uses equal jitter between half of the current
exponential delay and that delay, capped by `provider_retry_max_ms`. Cancellation
interrupts the wait. A provider's explicit Retry-After delay is honored without
jitter; if it exceeds the configured maximum wait, Helm returns the original
provider failure instead of retrying earlier. Reported usage remains known usage;
unreported work on failed requests is not treated as a measured zero cost.

Within one completed provider response, exact duplicate tool-call IDs with the
same tool name and JSON arguments produce one tool invocation and one canonical
result, in first-occurrence order. Helm validates the entire response before any
tool effects: conflicting calls sharing an ID fail the response. Empty, oversized
or control-containing IDs also fail validation. OpenAI Responses continuation
preserves opaque reasoning items while emitting the retained call only once.

A call ID is a response-local correlation label, not a durable effect identity.
The same ID in a later request or a resumed run is a fresh call and remains subject
to current local policy. Helm does not automatically retry tools or promise exactly
once effects across requests or crashes. Interrupted managed calls without durable
results continue to block execution until explicit [unknown-outcome reconciliation](local-tool-reconciliation.md).
Reconciliation records uncertainty and never repeats the effect or claims rollback.

Ordinary chat can continue after an interrupted tool batch, including after resume.
For historical calls without a saved result, the outgoing provider request includes
a failed result explaining that the outcome is unknown and the operation may have
taken effect. This request projection preserves the canonical transcript and all
saved results; it never re-executes historical calls. Inspect current state before
retrying an operation whose outcome is unknown. Managed-session admission still
requires the explicit reconciliation described above.

Local [inference allowances](inference-budgets.md) count dispatch permits across
root, child, retry and title requests. Their private attempt ledger distinguishes
reported token values from unavailable usage. These optional limits are not token
or monetary caps, and do not infer complete billing from saved token sums.
