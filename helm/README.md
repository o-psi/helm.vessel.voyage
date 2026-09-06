# Helm

Helm is a TUI that connects only to local or remote Vessels. Each Vessel
supervises and exposes Voyage runtime processes, with **one session per independent
Voyage process**. See the [architecture](../docs/architecture.md).

`helm connect` combines independent voyages through local, SSH and scoped-grant
Vessel routes. Ordinary chat, plain chat, one-shot, workflow and managed commands
also use supervised voyage owners. Tools, policy, tasks, subagents, terminals,
workflows and lifecycle actions are authorized runtime requests. See the
[current implementation](../docs/current-state.md).

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
connected local/SSH voyages, supervised outbound workers, diagnostics and
recovery. `helm connect` needs the companion `vessel` and `voyage` executables. See
[security](../docs/security.md) for execution and privacy boundaries and
[development](../docs/development.md) for repository work. Automated tests and
evaluations were removed in #140; no current coverage is implied by this inventory.
