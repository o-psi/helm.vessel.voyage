# Security, approvals, and operations

## Access modes

Set `access = "approval"` in configuration or pass
`--access read-only|approval|unrestricted` for a single invocation:

- `read-only` allows file, directory, search, todo-list, and terminal-output inspection. It
  rejects shell commands, file writes, persistent-terminal mutation, mutating todo actions,
  worktree changes, and unknown external MCP tools.
- `approval` allows ordinary inspection without prompts, asks before every file write and before
  commands classified as consequential, and keeps explicit denials in force.
- `unrestricted` does not ask for tool approval. Workspace read/write roots and `deny_commands`
  remain enforced; this mode is not an OS sandbox bypass.

Subagents inherit the same Helm configuration and therefore cannot use commands or writes when
the parent is read-only. Unattended execution cannot display an approval prompt, so actions
that require one follow `unattended_approval`. Vessel connectivity is currently
[unavailable](vessel-connectivity-retirement.md).

## Execution boundary

Helm confines filesystem tools to canonical readable and writable roots. Existing
symlinks are resolved before authorization. New paths are anchored to their nearest
existing canonical ancestor. Command denial scans every parsed word, preventing a
denied executable from being hidden behind a command chain. Unparseable commands are
denied. These controls complement rather than replace an OS sandbox.

Tool subprocesses start with an empty environment. `inherit_env` is an explicit
allowlist; names resembling keys, tokens, secrets, passwords, or credentials are
rejected. Values in `[env]`, MCP environment values, provider credentials, and
`redact_values` are removed from tool results, errors, displayed tool arguments, and
approval targets. Values shorter than four characters are not treated as secrets to
avoid corrupting ordinary output. Do not place secrets directly in prompts.

## Attended and unattended approvals

Interactive chat and local `run` are attended. An `always` or `on-risk` policy may
display an approval prompt. Each request carries a unique approval ID, execution ID,
action, redacted target, reason, and interaction mode; the outcome is emitted as a
structured tracing event.

Unattended execution never waits for terminal input. By default,
`unattended_approval = "deny"`: a tool requiring approval immediately returns an
unavailable/denied result and the agent can report the blocked action. Operators may
set it to `allow` only when prompts, workspace, and OS identity form a trusted
automation boundary. `approval = "never"` bypasses questions for both modes but does
not bypass roots or the deny list.

## Logs, health, and diagnostics

Use `--log-format json` with either binary for collector-friendly structured logs.
Approval and tool events carry correlation identifiers. Vessel accepts a safe
`X-Request-ID` or creates one, includes it in response headers, and records it in the
request span.

- `GET /health` is process liveness and does not inspect dependencies.
- `GET /ready` verifies database access, not attachment availability.
- `GET /metrics` exposes `voyage_connectivity_enabled 0`; no fleet/task gauges remain.
- `GET /v1/diagnostics` requires operator authentication and reports version,
  unavailable connectivity, unloaded legacy state and unimplemented attachment.
- `helm doctor` prints a secret-free JSON check of configuration, workspace,
  credential presence, session location, environment allowlist, and MCP names.

Do not expose diagnostics or metrics publicly without network-layer authentication.
They contain no credentials or prompts but reveal operational topology. Retain JSON
logs according to local policy and restrict them as potentially sensitive metadata.

The Vessel status page at `/ui` and diagnostics are disabled unless
`VESSEL_OPERATOR_TOKEN` (or `--operator-token`) is set. It accepts that secret as an
HTTP Bearer token or as the password in HTTP Basic authentication. Use a randomly
generated secret, keep it out of command history by preferring the environment
variable, and terminate TLS before Vessel outside loopback. The console never emits
the configured token into HTML, logs, diagnostics, or persisted control-plane state.

## Incident checklist

1. Capture UTC time, release version, execution/request/approval IDs, and redacted
   logs.
2. Stop new dispatch while preserving database and session files.
3. Isolate old workers/servers and their credentials for suspected compromise. This
   interim build has no enrollment or revocation API; preserve private evidence.
4. Confirm no managed or shell child process remains.
5. Run `helm doctor`, Vessel `/ready`, `/metrics`, and `/v1/diagnostics` locally.
6. Preserve evidence and follow the cutover rollback runbook for material failures.
