# Vessel

Vessel is Voyage's management plane. It currently provides health checks and an
authenticated status UI. Remote session management is planned.

## Run

```sh
cargo run -p vessel -- --bind 127.0.0.1:9480 --database vessel.db
```

Set `VESSEL_OPERATOR_TOKEN` securely in the environment to enable the status page
and diagnostics. They accept a Bearer token or the token as the password in HTTP
Basic authentication. Prefer the environment over `--operator-token` to avoid
command-history/process-list disclosure. Terminate TLS before non-loopback access.

Status endpoints:

| Endpoint | Behavior |
| --- | --- |
| `/health` | Process liveness |
| `/ready` | Database connection check |
| `/metrics` | `voyage_connectivity_enabled 0` |
| `/v1/diagnostics` | Authenticated service status |
| `/ui` | Authenticated status page |

UI/diagnostics return 503 without a configured operator token and 401 for missing or
invalid authentication when enabled. Request correlation and `--log-format json`
are supported, as are `completions` and `manpage`.

## Planned session management

The agreed [Vessel-managed session design](../docs/vessel-session-management.md)
specifies: `helm attach VESSEL_URL JOIN_KEY`, outbound interactive control,
explicit sharing and phased service/approval support. Delivery is tracked in
[#77](https://github.com/o-psi/voyage/issues/77). These are not implemented commands.

See [security and operations](../docs/security-operations.md) for access and
logging boundaries.
