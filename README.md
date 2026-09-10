# Voyage

Voyage is a system for ongoing AI-assisted work, operated through a terminal
interface across local and remote machines.

The architecture has three programs:

| Program | Responsibility |
| --- | --- |
| **Helm** (`helm`) | The TUI. Connects to local and remote Vessels to view and manage voyages. |
| **Vessel** (`vessel`) | Supervises and exposes voyage processes on its machine. |
| **Voyage** (`voyage`) | The execution runtime. Each session has its own independent process, conversation, agent and tools. |

```text
Helm TUI
  ├── local Vessel
  │     ├── voyage process A — session A
  │     └── voyage process B — session B
  └── remote Vessel
        └── voyage process C — session C
```

Switching views changes the input target, not which voyages execute. Closing Helm
must leave voyage processes running. A Vessel supervises those processes; it does
not run their agent loops inside its own service process. A voyage can admit
participating Vessels without creating another canonical owner of the session.
The [architecture](docs/architecture.md) defines these boundaries.

## Current implementation

This is a first-release development project. Ordinary Helm chat and run commands,
managed sessions and connected clients reach independent `voyage` processes through
Vessel. Local WS and scoped WSS routes share one authenticated full-duplex socket per
active Helm connection for commands, replies and durable invalidations. See the
[duplex transport](docs/duplex-transport.md). Session lifecycle, durable decisions,
private terminal attachment, participant execution
and positively fenced owner moves are implemented. See the
[current-state guide](docs/current-state.md) for supported paths and validation limits.

From a source checkout:

```sh
cargo build --workspace --release --locked
./target/release/helm --help
./target/release/helm connect
```

Configure a provider before starting chat; see [configuration](docs/configuration.md).
From an extracted full archive, use `./bin/helm` or `.\bin\helm.exe`. Keep its guide
and configuration directories together. Linux connected mode needs `vessel` and
`voyage` beside `helm`; it can start an absent local Vessel. The
[installer](installer/README.md) provides a real Linux installation wizard,
versioned upgrades and rollback, and user-service setup.

To review and install a built release on Linux, run
`./target/release/voyage-installer`. For later releases, use the new release's
installer with `upgrade --start`; `voyage-installer status` shows the installed
and rollback versions.

## Documentation

Start with the [documentation index](docs/README.md). The main paths are:

- [Architecture](docs/architecture.md) and [runtime contract](docs/runtime-contract.md).
- [Current behavior](docs/current-state.md), [configuration](docs/configuration.md)
  and [operations](docs/operations.md).
- [Security boundaries](docs/security.md) and [implementation sequence](docs/implementation.md).
- [Development](docs/development.md), [validation](docs/quality.md)
  and [release procedure](docs/releasing.md).

Automated tests and evaluation scenarios are currently absent. Build, static
analysis and packaging checks do not establish regression coverage. Documentation
of a capability is not a claim that it has passed behavioral or platform validation.
