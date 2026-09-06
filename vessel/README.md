# Vessel

The target Vessel supervises and exposes local Voyage runtime processes to Helm
interfaces. Helm connects only to local or remote Vessels; each Voyage runtime owns
**one session per independent process**. See the
[architecture](../docs/architecture.md).

**Current Vessel is a management plane and optional relay.** It serves health and
status, enrollment, authenticated outbound Helm presence and opt-in dedicated
remote-session HTTP operations. It does not yet launch or supervise Voyage
processes. Execution currently remains in Helm, and no `voyage` executable exists.
See the [current implementation](../docs/current-state.md).

From the repository root:

```sh
cargo run -p vessel -- --help
cargo run -p vessel -- --bind 127.0.0.1:9480 --database vessel.db
```

Set `VESSEL_OPERATOR_TOKEN` securely in the environment to enable authenticated
status and diagnostics. Prefer it over a token in command arguments. Use the
[operations guide](../docs/operations.md) for enrollment, TLS, presence and the
explicit `--remote-execution` relay. `--coordination-control` enables control
metadata and nomination leases, not task execution.

The `/health` and `/ready` endpoints report process/database state; `/metrics`
reports limited service metadata. `/ui` and `/v1/diagnostics` require operator
authentication when configured. The web page is a status view, not an execution
console. Enrollment and presence do not share private sessions or grant tool
execution, and providers and credentials stay on the current executing Helm.

See [configuration](../docs/configuration.md), [security](../docs/security.md) and
[development](../docs/development.md). Automated tests and evaluations were removed
in #140; endpoint availability does not establish deployment or release readiness.
