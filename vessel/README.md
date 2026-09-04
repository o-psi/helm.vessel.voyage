# Vessel

Vessel is the durable Voyage control plane. Its SQLite database is migrated at
startup and written with WAL and full synchronous durability. Worker credentials
are stored as SHA-256 verifier hashes, never as bearer-token plaintext.

```sh
vessel --database /var/lib/voyage/vessel.db --bind 127.0.0.1:9480
vessel pair voyage:v1:PAIRINGCODE
vessel fleet
```

## Operator console

Set `VESSEL_OPERATOR_TOKEN` to enable the embedded web console, then open `/ui`.
The browser uses HTTP Basic authentication: any username is accepted and the token
is the password. Bearer authentication is also supported for scripted UI checks.
Serve Vessel behind TLS whenever it is not bound exclusively to loopback; Basic and
Bearer credentials must not traverse plaintext networks.

The console refreshes every five seconds and provides fleet totals, worker status,
version, model, capabilities and last-seen time; recent task inventory; full task
state, attempt, lease, error, result, token usage and session details; task creation;
cancellation; and explicit retry of failed or cancelled tasks. Task UUIDs are the
durable correlation identifiers shown in logs and the console. Empty, missing,
disabled-auth and validation states produce explicit operator messages.

Task delivery uses ownership leases: every pull creates a unique lease, stale
completions are rejected, expired work is requeued up to three attempts, and
explicit failures can be retryable or terminal. `POST /v1/tasks/{id}/cancel`
cancels queued or running work. Operational APIs include `GET /v1/helms`, `GET
/v1/tasks`, and `GET /v1/fleet/summary`.
Operational endpoints are `/health` for liveness, `/ready` for database readiness,
`/metrics` for Prometheus-compatible fleet/task gauges, and `/v1/diagnostics` for a
secret-free support snapshot. Run with `--log-format json` for structured request
spans and correlation IDs. See [security and operations](../docs/security-operations.md).

## Planned Vessel session management

The agreed [Vessel-managed session design](../docs/vessel-session-management.md) specifies
`helm attach VESSEL_URL JOIN_KEY`, outbound interactive control, explicit sharing,
and phased service/approval support. These are planned capabilities, not current
CLI commands; existing pairing and task interfaces remain unchanged. Delivery is
tracked in [#77](https://github.com/o-psi/voyage/issues/77).
