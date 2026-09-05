# Vessel

Vessel is Voyage's management plane. It currently provides health checks and an
authenticated status page, authenticated outbound Helm presence, and opt-in
[dedicated remote-session HTTP operations](../docs/remote-sessions.md). Helm is the
planned operator interface for both local and remote work; Vessel routes control
and observations without executing model tools.

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
| `/metrics` | Whether attachment presence is configured |
| `/v1/diagnostics` | Authenticated service status |
| `/ui` | Authenticated status page |

UI/diagnostics return 503 without a configured operator token and 401 for missing or
invalid authentication when enabled. Request correlation and `--log-format json`
are supported, as are `completions` and `manpage`.

## Attachment presence

Configure enrollment and run an explicit foreground Helm connection using the
[attachment presence guide](../docs/attachment-presence.md). Operator diagnostics
show bounded current connection metadata. Presence does not grant execution or
share session content.

## Voyages across Helms

The agreed [voyage product model](../docs/voyages.md) separates the Helm an operator
opens from the Helm coordinating a voyage and the Helms executing delegated work.
A voyage is an open-ended session scoped to user-selected machines; it needs no
project, repository or component map. The coordinator may run remotely while an
authorized Helm interface disconnects or reconnects. Each executing Helm retains
its local policy, credentials and execution authority.

That orchestration and Helm operator interface remain planned under
[#77](https://github.com/o-psi/voyage/issues/77). Today's `helm remote-worker`
exposes one dedicated session through authenticated HTTP; it does not coordinate
multiple Helms. The `/ui` endpoint is a status page, and browser console work is
deferred. See the [attachment design](../docs/vessel-session-management.md) for
control-plane requirements and [remote sessions](../docs/remote-sessions.md) for
implemented setup and recovery.

See [security and operations](../docs/security-operations.md) for access and
logging boundaries.
