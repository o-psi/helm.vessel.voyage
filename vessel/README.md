# Vessel

Vessel supervises and exposes local Voyage runtime processes to Helm
interfaces. Helm connects only to local or remote Vessels; each Voyage runtime owns
**one session per independent process**. See the
[architecture](../docs/architecture.md).

Vessel now has a Linux process supervisor: `local-serve` launches independent voyage
executables and exposes its process protocol through an authenticated ephemeral
loopback HTTP endpoint. Helm discovers the endpoint and bearer credential from the
owned private `process-http.json` record. Commands use bounded POST requests and
incoming durable invalidations use SSE. Vessel verifies runtime identity, serializes
competing starts and supports explicit stop/restart.
The separate scoped HTTP process gateway supports human workspace pairing and
session grants with current revision, expiration and revocation checks. Participant
bindings, signed owner transfer and explicit recovery use the same supervisor. See
[current implementation](../docs/current-state.md) and
[process access setup](../docs/process-access.md).

From the repository root:

```sh
cargo build --workspace --locked
./target/debug/vessel local-serve --directory /absolute/private-vessel \
  --voyage-binary "$PWD/target/debug/voyage"
# Separate HTTP management/gateway service:
./target/debug/vessel --bind 127.0.0.1:9480 --database vessel.db \
  --process-directory /absolute/private-vessel \
  --public-origin https://vessel.example.com
```

Set `VESSEL_OPERATOR_TOKEN` securely in the environment to enable authenticated
status and diagnostics. Prefer it over a token in command arguments. Use the
[process access guide](../docs/process-access.md) for scoped grants and TLS proxy
setup. Outbound worker mode, enrollment and its relay/control API are retired.

The `/health` and `/ready` endpoints report process/database state; `/metrics`
reports limited service metadata. `/ui` and `/v1/diagnostics` require operator
authentication when configured. The web page is a status view, not an execution
console. Providers and credentials stay on the executing host. For the process
supervisor, Helm detaches without cancelling voyages. Supervisor stop does not intentionally
kill independent runtimes; machine reboot still ends processes. Linux service
installation is documented in the [installer guide](../installer/README.md).

See [configuration](../docs/configuration.md), [security](../docs/security.md) and
[development](../docs/development.md). Automated tests and evaluations were removed
in #140; endpoint availability does not establish deployment or release readiness.
