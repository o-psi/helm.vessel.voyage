# Voyage

Voyage is a system for general-purpose LLM work across local and remote machines.

- **Helm** is the Rust TUI and agent runtime. `helm serve` makes a Helm remotely
  reachable while preserving its local tool policy and workspace boundary.
- **Vessel** is the management plane. It registers Helms, tracks liveness, exposes
  fleet discovery, and dispatches work to a chosen online Helm.
- **voyage-protocol** owns the versioned wire types shared by both binaries.

```text
operator ── Helm TUI ── local tools
    │
    └── Vessel API ── registered Helm(s) ── scoped tools/workspaces
```

## Run locally

```sh
cargo build --workspace

# management plane (token must match protected Helms)
VESSEL_HELM_TOKEN=... cargo run -p vessel -- --bind 127.0.0.1:9480

# agent node, registered with Vessel
OPENAI_API_KEY=... cargo run -p helm -- serve \
  --bind 127.0.0.1:9470 \
  --public-url http://127.0.0.1:9470 \
  --vessel http://127.0.0.1:9480 \
  --name workstation
```

The initial network API is `/v1`: Vessel offers Helm registration, heartbeat,
inventory, removal, and task dispatch; Helm offers health, metadata, and task
execution. Set `HELM_SERVER_TOKEN` when exposing a Helm beyond a trusted network.

See [Helm's README](helm/README.md) for provider, policy, session, and CLI details.

For self-hosted source control, see [the local Git server guide](docs/local-git.md).
