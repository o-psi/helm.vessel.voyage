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

The runtime library supplies shared configuration and data types to Helm; Helm no
longer constructs a session executor. Additional process operations cover lifecycle,
configuration, workflow/operator tools, private terminal attachment, scoped grants,
participant assignments and positively fenced owner transfer. Administrative
`recover`, `legacy-recover`, `upgrade-journal`, `remote-consent` and `host-resources`
commands preserve explicit recovery and attestation boundaries without model calls.
See [current state](../docs/current-state.md), [operations](../docs/operations.md),
[configuration](../docs/configuration.md), [process access](../docs/process-access.md)
and the [implementation ledger](../docs/implementation.md) for evidence and limits.
