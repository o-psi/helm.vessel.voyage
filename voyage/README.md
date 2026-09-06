# Voyage runtime

The `voyage` executable owns one session in an independent process. Linux Vessel
creates its private registration and launches it; use `helm connect` through Vessel
for normal operations. Starting an arbitrary runtime socket directly from Helm is
not the supported process route.

```sh
cargo build --workspace --locked
./target/debug/voyage --help
./target/debug/helm connect
```

The runtime reuses managed journal admission, canonical checkpoints, execution
fences, steering, durable decisions and cleanup obligations. Its provider, tools
and execution policy are loaded on the executing host. Runtime registration binds
session and incarnation with a private token; the Unix peer must be the same OS
user. Provider credentials are never supplied by the connected interface.

The process API supports snapshots, exact command receipts, submission, steering,
run cancellation, rename, model selection, decisions and stop. Stop requests
cancellation and retains uncertain cleanup obligations. A clean-stop marker is
written only after observed cleanup. Unavailability does not authorize another
owner or automatic replay of uncertain effects.

Runtime code also remains a library used by legacy Helm chat/run/managed/worker
entrypoints. That compatibility dependency means the final architecture cutover
is incomplete. Connected workflow parity, branch/archive/delete, private terminal
attachment, enrolled session grants, participant execution and owner migration
remain unfinished. See [current state](../docs/current-state.md),
[operations](../docs/operations.md), [configuration](../docs/configuration.md) and
the [implementation ledger](../docs/implementation.md).
