# Vessel

Vessel is under development as part of Voyage's unreleased first version. The target
is a release-quality management plane with complete, tested user workflows. The
interim state below does not meet that target yet.

Vessel is Voyage's management plane. Its legacy pairing and HTTP task-worker
implementation has been removed ahead of the replacement attachment flow.
**Helm connectivity, fleet/task operations and session management are currently
unavailable.** Local Helm functionality remains usable independently.

## Run the interim management plane

```sh
cargo run -p vessel -- --bind 127.0.0.1:9480 --database vessel.db
```

Set `VESSEL_OPERATOR_TOKEN` securely in the environment to enable the status page
and diagnostics. They accept a Bearer token or the token as the password in HTTP
Basic authentication. Prefer the environment over `--operator-token` to avoid
command-history/process-list disclosure. Terminate TLS before non-loopback access.

Retained endpoints:

| Endpoint | Behavior |
| --- | --- |
| `/health` | Process liveness |
| `/ready` | Database connection check; does not claim attachment is ready |
| `/metrics` | `voyage_connectivity_enabled 0`; no legacy fleet/task gauges |
| `/v1/diagnostics` | Authenticated status: connectivity unavailable, legacy state not loaded |
| `/ui` | Authenticated explanation of the transition; no task forms |

UI/diagnostics return 503 without a configured operator token and 401 for missing or
invalid authentication when enabled. Request correlation and `--log-format json`
remain supported, as do `completions` and `manpage`.

`pair`, `fleet`, lease/pairing/staleness timing flags, and all old pairing, worker,
fleet and task API/UI routes have been removed. Retired routes return 404 even for
authenticated clients. There is no new invitation or attachment endpoint yet.

## Stored development data and build updates

The server opens SQLite for readiness but does not load, rewrite, migrate or delete
legacy `control_plane` snapshots or `schema_migrations`. Stored running tasks do not
represent live execution and are not retried. No credentials or history are imported
into the future connection model. See [retirement and data preservation](../docs/vessel-connectivity-retirement.md)
before updating or reverting development builds; stop old processes separately and
protect backups.

The agreed [Vessel-managed session design](../docs/vessel-session-management.md)
remains the target: `helm attach VESSEL_URL JOIN_KEY`, outbound interactive control,
explicit sharing and phased service/approval support. Delivery is tracked in
[#77](https://github.com/o-psi/voyage/issues/77). These are not implemented commands.

See [security and operations](../docs/security-operations.md) for retained access and
logging boundaries. The system regression fixture `tests/system/vessel_lifecycle.py`
now verifies the clean break and data preservation rather than the removed scheduler.
