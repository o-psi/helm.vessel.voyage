# Security, approvals, and operations

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

Vessel work is unattended and never waits for terminal input. By default,
`unattended_approval = "deny"`: a tool requiring approval immediately returns an
unavailable/denied result and the agent can report the blocked action. Operators may
set it to `allow` only when Vessel, prompts, workspace, and OS identity form a trusted
automation boundary. `approval = "never"` bypasses questions for both modes but does
not bypass roots or the deny list.

## Logs, health, and diagnostics

Use `--log-format json` with either binary for collector-friendly structured logs.
Approval and tool events carry correlation identifiers. Vessel accepts a safe
`X-Request-ID` or creates one, includes it in response headers, and records it in the
request span.

- `GET /health` is process liveness and does not inspect dependencies.
- `GET /ready` verifies database access and gates traffic readiness.
- `GET /metrics` exposes Prometheus text gauges for Helm and task states.
- `GET /v1/diagnostics` returns a secret-free version, protocol, fleet, pairing, and
  queue snapshot.
- `helm doctor` prints a secret-free JSON check of configuration, workspace,
  credential presence, session location, environment allowlist, and MCP names.

Do not expose diagnostics or metrics publicly without network-layer authentication.
They contain no credentials or prompts but reveal operational topology. Retain JSON
logs according to local policy and restrict them as potentially sensitive metadata.

The embedded Vessel operator console at `/ui` is disabled unless
`VESSEL_OPERATOR_TOKEN` (or `--operator-token`) is set. It accepts that secret as an
HTTP Bearer token or as the password in HTTP Basic authentication. Use a randomly
generated secret, keep it out of command history by preferring the environment
variable, and terminate TLS before Vessel outside loopback. The console never emits
the configured token into HTML, logs, diagnostics, or persisted control-plane state.

## Incident checklist

1. Capture UTC time, release version, execution/request/approval IDs, and redacted
   logs.
2. Stop new dispatch while preserving database and session files.
3. Revoke enrollment credentials for suspected compromise.
4. Confirm no managed or shell child process remains.
5. Run `helm doctor`, Vessel `/ready`, `/metrics`, and `/v1/diagnostics` locally.
6. Preserve evidence and follow the cutover rollback runbook for material failures.
