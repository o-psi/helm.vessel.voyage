# Helm

The target Helm is a TUI that connects only to local or remote Vessels. Each Vessel
supervises and exposes Voyage runtime processes, with **one session per independent
Voyage process**. See the [architecture](../docs/architecture.md).

`helm connect` is the connected multiplexer for independent voyage processes through
local or SSH-accessed Vessels. The legacy `helm chat`, plain chat and one-shot
entrypoints still execute locally through the shared voyage library. Connected
mode does not yet replace all legacy workflow, task, terminal and lifecycle controls.
See the [current implementation](../docs/current-state.md).

From the repository root:

```sh
cargo build --workspace --release --locked
export PATH="$PWD/target/release:$PATH"
helm --help
helm connect
helm config
helm doctor
helm chat
helm chat --plain
helm run "summarize the files in this workspace"
helm sessions
```

Configure a provider before sending a prompt. Native API providers do not require
Codex; API billing and subscription access are separate. Provider selection, local
endpoints, credentials, policy and remembered chat preferences are covered in
[configuration](../docs/configuration.md) and the
[example configuration](config.example.toml).

[Operations](../docs/operations.md) covers session controls, managed sessions,
connected local/SSH voyages, foreground legacy remote workers, diagnostics and
recovery. `helm connect` needs the companion `vessel` and `voyage` executables. See
[security](../docs/security.md) for execution and privacy boundaries and
[development](../docs/development.md) for repository work. Automated tests and
evaluations were removed in #140; no current coverage is implied by this inventory.
